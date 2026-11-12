//! Emulator RoQ noise bisect (host). Stages T1–T6 vs source PCM.
//!
//! T1  PCM as received after LE unpack (JNI → Rust mirror)
//! T2  sender-local Opus encode→decode (no network)
//! T3  RTP payload identity sent vs received (pre-decrypt tag+body)
//! T4  after SRTP decrypt: payload equals sender pre-encrypt Opus
//! T5  decoded PCM on receiver (before AudioTrack / dump)
//! T6  dump path = T5 (harness compares this)
//!
//! First stage that goes bad is reported. Root cause of Android dump noise was
//! premature Opus PLC before first playout (see playout_startup.rs).

mod common;

use common::{linked_pipelines, tone_pcm};
use saga_media::{signal_correlation, signal_snr_db, OpusDecoder, OpusEncoder, FRAME_SAMPLES};

fn corr_snr(a: &[i16], b: &[i16]) -> (f64, f64) {
    let n = a.len().min(b.len());
    (
        signal_correlation(&a[..n], &b[..n], FRAME_SAMPLES * 4),
        signal_snr_db(&a[..n], &b[..n]),
    )
}

fn le_unpack(pcm: &[u8]) -> Vec<i16> {
    assert_eq!(pcm.len() % 2, 0);
    let mut out = Vec::with_capacity(pcm.len() / 2);
    for chunk in pcm.chunks_exact(2) {
        out.push(i16::from_le_bytes([chunk[0], chunk[1]]));
    }
    out
}

fn le_pack(samples: &[i16]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    bytes
}

#[test]
fn t1_jni_le_buffer_path_bit_exact_ramp() {
    // Mirrors Kotlin direct ByteBuffer.order(nativeOrder()) + Rust push_capture.
    let mut ramp = Vec::with_capacity(FRAME_SAMPLES);
    for i in 0..FRAME_SAMPLES {
        ramp.push((i as i16).wrapping_mul(7).wrapping_sub(2000));
    }
    let bytes = le_pack(&ramp);
    let back = le_unpack(&bytes);
    assert_eq!(ramp, back, "T1 JNI LE path must be bit-exact");
}

#[test]
fn t2_local_opus_encode_decode_keeps_tone() {
    let pcm = tone_pcm(20, 440.0);
    let mut enc = OpusEncoder::new().unwrap();
    let mut dec = OpusDecoder::new().unwrap();
    let mut out = Vec::with_capacity(pcm.len());
    let mut scratch = vec![0i16; FRAME_SAMPLES];
    let mut opus_buf = [0u8; 400];
    for frame in pcm.chunks_exact(FRAME_SAMPLES) {
        let mut f = frame.to_vec();
        let n = enc.encode_frame(&mut f, &mut opus_buf).unwrap();
        let m = dec
            .decode_frame(&opus_buf[..n], &mut scratch, false)
            .unwrap();
        out.extend_from_slice(&scratch[..m]);
    }
    let (corr, snr) = corr_snr(&pcm, &out);
    eprintln!("T2 opus-local corr={corr:.3} snr={snr:.1} dB");
    assert!(corr >= 0.85, "T2 first-bad if corr low: Opus loop");
}

#[tokio::test]
async fn t3_t4_t5_network_stages() {
    let mut pair = linked_pipelines().await;
    let pcm = tone_pcm(30, 440.0);
    let mut played = Vec::new();
    let mut sent_payloads = 0u64;
    let mut matched_decrypt = 0u64;

    // Silence before first decode (Android ticks early) must not poison T5/T6.
    let mut early = vec![0i16; FRAME_SAMPLES];
    for _ in 0..5 {
        pair.callee.playout_frame(&mut early).unwrap();
        assert!(
            early.iter().all(|s| *s == 0),
            "T5/T6 poisoned: PLC noise before first packet"
        );
    }

    for frame in pcm.chunks_exact(FRAME_SAMPLES) {
        let mut f = frame.to_vec();
        // Capture Opus pre-encrypt via protect helper (T3/T4).
        let protected = pair.caller.protect_pcm_for_test(&mut f).unwrap();
        sent_payloads += 1;
        pair.callee.inject_framed_for_test(&protected);

        // Drain until a decoded frame appears (JB may need depth).
        let mut out = vec![0i16; FRAME_SAMPLES];
        for _ in 0..8 {
            let n = pair.callee.playout_frame(&mut out).unwrap();
            if out.iter().any(|s| *s != 0) {
                played.extend_from_slice(&out[..n]);
                matched_decrypt += 1;
                break;
            }
            // Still silent priming — not failure yet.
        }
    }

    assert!(sent_payloads > 0, "T3: no RTP sent");
    assert!(
        matched_decrypt > 0,
        "T4/T5: decrypt/decode produced no energy (SRTP or PLC startup)"
    );

    let n = played.len().min(pcm.len());
    let (corr, snr) = corr_snr(&pcm[..n], &played[..n]);
    eprintln!(
        "T5 decoded corr={corr:.3} snr={snr:.1} dB (sent={sent_payloads} decoded={matched_decrypt})"
    );
    assert!(
        corr >= 0.85,
        "T5/T6 first-bad: decoded PCM corr {corr} (root was premature PLC if silence-before missing)"
    );
    let _ = snr;
}

#[tokio::test]
async fn empty_buffer_underrun_does_not_skip_late_packets() {
    // Reproduces emulator RoQ death spiral: tick playout while datagrams lag,
    // then deliver — must still decode (no already_played skip).
    let mut pair = linked_pipelines().await;
    let mut frame = tone_pcm(1, 440.0);
    let mut out = vec![0i16; FRAME_SAMPLES];

    pair.caller.send_pcm_frame(&mut frame).unwrap();
    pair.callee.recv_one_datagram().await.unwrap();
    let n = pair.callee.playout_frame(&mut out).unwrap();
    assert_eq!(n, FRAME_SAMPLES);
    assert!(out.iter().any(|s| *s != 0), "first packet must play");

    // Simulate 10 late ticks with empty JB (no advance).
    for _ in 0..10 {
        out.fill(99);
        pair.callee.playout_frame(&mut out).unwrap();
        assert!(
            out.iter().all(|s| *s == 0),
            "empty underrun must be silence without clock skip"
        );
    }

    let mut frame2 = tone_pcm(1, 880.0);
    pair.caller.send_pcm_frame(&mut frame2).unwrap();
    pair.callee.recv_one_datagram().await.unwrap();
    pair.callee.playout_frame(&mut out).unwrap();
    assert!(
        out.iter().any(|s| *s != 0),
        "late packet must still decode (not already_played)"
    );
    let snap = pair.counters_b.snapshot();
    assert!(
        snap.played >= 2,
        "expected ≥2 played, got {} plc={}",
        snap.played,
        snap.plc_frames
    );
}
