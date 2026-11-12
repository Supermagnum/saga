//! Startup playout must emit silence until the first RTP arrives.
//! Premature Opus PLC (empty packet) synthesizes high-energy noise that
//! poisoned Android dump scoring (emulator RoQ "noise" bisect).

mod common;

use common::linked_pipelines;
use saga_media::FRAME_SAMPLES;

#[tokio::test]
async fn playout_before_first_packet_is_silence() {
    let mut pair = linked_pipelines().await;
    let mut out = vec![0i16; FRAME_SAMPLES];
    for _ in 0..10 {
        out.fill(123);
        let n = pair.callee.playout_frame(&mut out).unwrap();
        assert_eq!(n, FRAME_SAMPLES);
        assert!(
            out.iter().all(|s| *s == 0),
            "expected silence before first packet, got peak {:?}",
            out.iter().map(|s| s.wrapping_abs()).max()
        );
    }
    let mut frame = vec![1000i16; FRAME_SAMPLES];
    pair.caller.send_pcm_frame(&mut frame).unwrap();
    pair.callee.recv_one_datagram().await.unwrap();
    let n = pair.callee.playout_frame(&mut out).unwrap();
    assert_eq!(n, FRAME_SAMPLES);
    assert!(
        out.iter().any(|s| *s != 0),
        "expected decoded energy after first packet"
    );
}

#[tokio::test]
async fn silence_while_buffering_before_first_decode() {
    // Even after RTP is inserted, do not PLC/advance until a real frame plays.
    let mut pair = linked_pipelines().await;
    let mut out = vec![0i16; FRAME_SAMPLES];
    assert!(!pair.callee.jb_playout_started_for_test());
    for _ in 0..5 {
        let n = pair.callee.playout_frame(&mut out).unwrap();
        assert_eq!(n, FRAME_SAMPLES);
        assert!(out.iter().all(|s| *s == 0));
        assert!(!pair.callee.jb_playout_started_for_test());
    }
}

#[test]
fn le_pcm_byte_path_roundtrips_ramp() {
    // Mirrors Android JNI → push_capture LE unpack (i16::from_le_bytes).
    let mut ramp = Vec::with_capacity(FRAME_SAMPLES);
    for i in 0..FRAME_SAMPLES {
        ramp.push((i as i16).wrapping_mul(3).wrapping_sub(1000));
    }
    let mut bytes = vec![0u8; FRAME_SAMPLES * 2];
    for (i, s) in ramp.iter().enumerate() {
        let b = s.to_le_bytes();
        bytes[i * 2] = b[0];
        bytes[i * 2 + 1] = b[1];
    }
    let mut back = vec![0i16; FRAME_SAMPLES];
    for i in 0..FRAME_SAMPLES {
        back[i] = i16::from_le_bytes([bytes[i * 2], bytes[i * 2 + 1]]);
    }
    assert_eq!(ramp, back);
}
