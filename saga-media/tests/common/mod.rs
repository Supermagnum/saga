#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;

use iroh::endpoint::presets;
use iroh::{Endpoint, RelayMode};
use iroh_roq::{Session, ALPN};
use saga_media::{CallRole, MediaCounters, MediaPipeline, FRAME_SAMPLES, SAMPLE_RATE};
use saga_session::HandshakeOutput;
use zeroize::Zeroizing;

pub fn dummy_handshake(seed: u8) -> HandshakeOutput {
    HandshakeOutput {
        shared_secret: Zeroizing::new(vec![seed; 48]),
        transcript_hash: [seed.wrapping_add(1); 32],
        profile_prk: Zeroizing::new([seed.wrapping_add(2); 32]),
        saga_session_key: Zeroizing::new([seed.wrapping_add(3); 32]),
        confirmation_key: Zeroizing::new([seed.wrapping_add(4); 32]),
    }
}

pub fn tone_pcm(frames: usize, freq_hz: f64) -> Vec<i16> {
    let mut out = Vec::with_capacity(frames * FRAME_SAMPLES);
    for f in 0..frames {
        for n in 0..FRAME_SAMPLES {
            let t = ((f * FRAME_SAMPLES + n) as f64) / SAMPLE_RATE as f64;
            let s = (2.0 * std::f64::consts::PI * freq_hz * t).sin();
            out.push((s * 0.4 * 32767.0) as i16);
        }
    }
    out
}

pub fn load_wav_mono_48k(path: &str) -> Vec<i16> {
    let mut reader = hound::WavReader::open(path).expect("open wav");
    assert_eq!(reader.spec().sample_rate, SAMPLE_RATE);
    assert_eq!(reader.spec().channels, 1);
    reader
        .samples::<i16>()
        .map(|s| s.expect("sample"))
        .collect()
}

pub fn fixture_speech() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/speech_clip_48k.wav")
}

pub struct LinkedPair {
    pub caller: MediaPipeline,
    pub callee: MediaPipeline,
    pub counters_a: Arc<MediaCounters>,
    pub counters_b: Arc<MediaCounters>,
    /// Keep RoQ sessions alive: dropping them aborts the datagram demuxer.
    _session_a: Session,
    _session_b: Session,
    _ep1: Endpoint,
    _ep2: Endpoint,
}

pub async fn linked_pipelines() -> LinkedPair {
    let ep1 = Endpoint::builder(presets::Minimal)
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .expect("ep1");
    let ep2 = Endpoint::builder(presets::Minimal)
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .expect("ep2");
    let addr = ep2.addr();
    let flow = MediaPipeline::flow_id_hint();
    let hs = dummy_handshake(7);

    let accept = tokio::spawn({
        let ep2 = ep2.clone();
        async move {
            let incoming = ep2.accept().await.expect("incoming").await.expect("accept");
            assert_eq!(incoming.alpn(), ALPN);
            let session = Session::new(incoming);
            let send = session.new_send_flow(flow).await.unwrap();
            let recv = session.new_receive_flow(flow).await.unwrap();
            (session, send, recv)
        }
    });

    let conn = ep1.connect(addr, ALPN).await.expect("connect");
    let session_a = Session::new(conn);
    let send_a = session_a.new_send_flow(flow).await.unwrap();
    let recv_a = session_a.new_receive_flow(flow).await.unwrap();
    let (session_b, send_b, recv_b) = accept.await.unwrap();

    let counters_a = Arc::new(MediaCounters::default());
    let counters_b = Arc::new(MediaCounters::default());
    // Caller session owns (send_a, recv_a); callee owns (send_b, recv_b).
    let caller =
        MediaPipeline::new(&hs, CallRole::Caller, send_a, recv_a, counters_a.clone()).unwrap();
    let callee =
        MediaPipeline::new(&hs, CallRole::Callee, send_b, recv_b, counters_b.clone()).unwrap();

    LinkedPair {
        caller,
        callee,
        counters_a,
        counters_b,
        _session_a: session_a,
        _session_b: session_b,
        _ep1: ep1,
        _ep2: ep2,
    }
}

/// Send PCM frames A→B and collect playout (interleaved so JB cap is not hit).
pub async fn roundtrip_a_to_b(pair: &mut LinkedPair, pcm: &[i16]) -> Vec<i16> {
    let frames = pcm.len() / FRAME_SAMPLES;
    let mut played = Vec::with_capacity(pcm.len());
    for f in 0..frames {
        let mut frame = pcm[f * FRAME_SAMPLES..(f + 1) * FRAME_SAMPLES].to_vec();
        pair.caller.send_pcm_frame(&mut frame).unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            pair.callee.recv_one_datagram(),
        )
        .await
        .expect("recv timeout")
        .unwrap();
        let mut out = vec![0i16; FRAME_SAMPLES];
        let _ = pair.callee.playout_frame(&mut out);
        played.extend_from_slice(&out);
    }
    // Do not append PLC drain into the compared signal — it tanks SI-SNR.
    played
}
