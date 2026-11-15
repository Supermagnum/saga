//! Mesh mixer over real RoQ flows: fan-out, mixing, pre-answer gate, limits.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::dummy_handshake;
use iroh::endpoint::presets;
use iroh::{Endpoint, RelayMode};
use iroh_roq::{ReceiveFlow, SendFlow, Session, ALPN};
use saga_media::{
    mesh_link_plain, mesh_link_srtp, CallRole, LinkIngest, LinkReceiver, MediaCounters, MediaError,
    MeshMixer, PeerId, FRAME_SAMPLES, MEDIA_TAG_SRTP, SAMPLE_RATE,
};

type Half = (Session, SendFlow, ReceiveFlow);

async fn endpoint() -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .expect("endpoint")
}

/// One RoQ link between two endpoints. Returns (a-side, b-side).
async fn roq_link(a: &Endpoint, b: &Endpoint) -> (Half, Half) {
    let flow = saga_media::MediaPipeline::flow_id_hint();
    let addr = b.addr();
    let accept = tokio::spawn({
        let b = b.clone();
        async move {
            let conn = b.accept().await.expect("incoming").await.expect("accept");
            let s = Session::new(conn);
            let send = s.new_send_flow(flow).await.unwrap();
            let recv = s.new_receive_flow(flow).await.unwrap();
            (s, send, recv)
        }
    });
    let conn = a.connect(addr, ALPN).await.expect("connect");
    let s = Session::new(conn);
    let send = s.new_send_flow(flow).await.unwrap();
    let recv = s.new_receive_flow(flow).await.unwrap();
    ((s, send, recv), accept.await.unwrap())
}

struct Mesh {
    mixers: Vec<MeshMixer>,
    ids: Vec<PeerId>,
    /// Keep RoQ sessions and endpoints alive for the test duration.
    _sessions: Vec<Session>,
    eps: Vec<Endpoint>,
}

/// Full SRTP mesh of `n` nodes with distinct per-link keys.
async fn build_mesh(n: usize) -> Mesh {
    let mut eps = Vec::new();
    for _ in 0..n {
        eps.push(endpoint().await);
    }
    let ids: Vec<PeerId> = eps.iter().map(|e| *e.id().as_bytes()).collect();
    let mut mixers: Vec<MeshMixer> = (0..n).map(|_| MeshMixer::new().unwrap()).collect();
    let mut sessions = Vec::new();
    let mut seed = 1u8;
    for i in 0..n {
        for j in (i + 1)..n {
            let ((sa, tx_a, rx_a), (sb, tx_b, rx_b)) = roq_link(&eps[i], &eps[j]).await;
            sessions.push(sa);
            sessions.push(sb);
            let hs = dummy_handshake(seed);
            seed += 10;
            let (s_a, r_a) = mesh_link_srtp(
                &hs,
                CallRole::Caller,
                tx_a,
                rx_a,
                Arc::new(MediaCounters::default()),
            )
            .unwrap();
            let (s_b, r_b) = mesh_link_srtp(
                &hs,
                CallRole::Callee,
                tx_b,
                rx_b,
                Arc::new(MediaCounters::default()),
            )
            .unwrap();
            mixers[i].add_link(ids[j], s_a, r_a).unwrap();
            mixers[j].add_link(ids[i], s_b, r_b).unwrap();
        }
    }
    Mesh {
        mixers,
        ids,
        _sessions: sessions,
        eps,
    }
}

fn tone_frames(frames: usize, freq_hz: f64, amp: f64) -> Vec<i16> {
    let mut out = Vec::with_capacity(frames * FRAME_SAMPLES);
    for n in 0..frames * FRAME_SAMPLES {
        let t = n as f64 / SAMPLE_RATE as f64;
        out.push(((2.0 * std::f64::consts::PI * freq_hz * t).sin() * amp * 32767.0) as i16);
    }
    out
}

fn goertzel_power(pcm: &[i16], freq: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * freq / SAMPLE_RATE as f64;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &x in pcm {
        let s0 = x as f64 + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - coeff * s1 * s2) / (pcm.len() as f64).powi(2)
}

#[tokio::test]
async fn encode_once_fan_out_different_ciphertext_per_link() {
    let mut m = build_mesh(3).await;
    let mut pcm = tone_frames(1, 440.0, 0.25);
    let sent = m.mixers[0].send_pcm_frame(&mut pcm).unwrap();
    assert_eq!(sent, 2, "frame must be queued on both links");

    let id0 = m.ids[0];
    let b1 = tokio::time::timeout(
        Duration::from_secs(2),
        m.mixers[1]
            .receiver_mut(&id0)
            .unwrap()
            .read_datagram_bytes(),
    )
    .await
    .unwrap()
    .unwrap();
    let b2 = tokio::time::timeout(
        Duration::from_secs(2),
        m.mixers[2]
            .receiver_mut(&id0)
            .unwrap()
            .read_datagram_bytes(),
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(b1[0], MEDIA_TAG_SRTP);
    assert_eq!(b2[0], MEDIA_TAG_SRTP);
    assert_eq!(b1.len(), b2.len());
    // Same cleartext RTP header (one encode, one seq/ssrc), different ciphertext.
    assert_eq!(&b1[1..13], &b2[1..13]);
    assert_ne!(&b1[13..], &b2[13..]);
}

#[tokio::test]
async fn mixer_sums_two_tones_with_detectable_energy() {
    let mut m = build_mesh(3).await;
    let frames = 60usize;
    let pcm0 = tone_frames(frames, 440.0, 0.25);
    let pcm1 = tone_frames(frames, 880.0, 0.25);
    let (id0, id1) = (m.ids[0], m.ids[1]);

    let mut mixed = Vec::new();
    for f in 0..frames {
        let mut a = pcm0[f * FRAME_SAMPLES..(f + 1) * FRAME_SAMPLES].to_vec();
        let mut b = pcm1[f * FRAME_SAMPLES..(f + 1) * FRAME_SAMPLES].to_vec();
        m.mixers[0].send_pcm_frame(&mut a).unwrap();
        m.mixers[1].send_pcm_frame(&mut b).unwrap();
        for id in [id0, id1] {
            tokio::time::timeout(Duration::from_secs(2), m.mixers[2].recv_link_one(&id))
                .await
                .expect("recv timeout")
                .unwrap();
        }
        let mut out = [0i16; FRAME_SAMPLES];
        m.mixers[2].playout_mixed(&mut out).unwrap();
        mixed.extend_from_slice(&out);
    }
    let tail = &mixed[10 * FRAME_SAMPLES..];
    let p440 = goertzel_power(tail, 440.0);
    let p880 = goertzel_power(tail, 880.0);
    let p660 = goertzel_power(tail, 660.0);
    assert!(p440 > 1e4, "440 Hz energy missing: {p440}");
    assert!(p880 > 1e4, "880 Hz energy missing: {p880}");
    assert!(p440 > 100.0 * p660, "440 Hz not above noise floor");
    assert!(p880 > 100.0 * p660, "880 Hz not above noise floor");
    let ratio = p440 / p880;
    assert!((0.25..4.0).contains(&ratio), "sources unbalanced: {ratio}");
}

#[tokio::test]
async fn pre_answer_unarmed_link_drops_capture() {
    let mut m = build_mesh(3).await;
    let (id0, id1, id2) = (m.ids[0], m.ids[1], m.ids[2]);

    assert!(m.mixers[0].set_link_pre_answer(&id1, true));
    let mut pcm = tone_frames(1, 440.0, 0.25);
    let sent = m.mixers[0].send_pcm_frame(&mut pcm).unwrap();
    assert_eq!(sent, 1, "only the answered link carries capture");
    assert_eq!(m.mixers[0].sender(&id1).unwrap().gated_drops(), 1);
    assert_eq!(
        m.mixers[0].sender(&id1).unwrap().counters.snapshot().sent,
        0
    );
    assert_eq!(
        m.mixers[0].sender(&id2).unwrap().counters.snapshot().sent,
        1
    );

    // Nothing reaches the gated peer.
    let none =
        tokio::time::timeout(Duration::from_millis(150), m.mixers[1].recv_link_one(&id0)).await;
    assert!(none.is_err(), "gated link must not emit media");

    // All links gated: Opus is skipped and nothing is sent.
    assert!(m.mixers[0].set_link_pre_answer(&id2, true));
    let mut pcm = tone_frames(1, 440.0, 0.25);
    assert_eq!(m.mixers[0].send_pcm_frame(&mut pcm).unwrap(), 0);

    // Peer 1 answers: signal arrives, arms node 0's sender toward 1.
    m.mixers[1].send_answered_signal(&id0).unwrap();
    let got = tokio::time::timeout(Duration::from_secs(2), m.mixers[0].recv_link_one(&id1))
        .await
        .expect("answered timeout")
        .unwrap();
    assert_eq!(got, LinkIngest::Answered);
    assert!(m.mixers[0].sender(&id1).unwrap().is_armed());
    let mut pcm = tone_frames(1, 440.0, 0.25);
    assert_eq!(m.mixers[0].send_pcm_frame(&mut pcm).unwrap(), 1);
}

#[tokio::test]
async fn receiver_gate_discards_pre_answer_media() {
    let mut m = build_mesh(2).await;
    let (id0, id1) = (m.ids[0], m.ids[1]);
    // Node 1 has not answered yet: its receive side is gated.
    assert!(m.mixers[1].set_link_pre_answer(&id0, true));
    // Node 0 is armed (default) and sends.
    let mut pcm = tone_frames(1, 440.0, 0.25);
    m.mixers[0].send_pcm_frame(&mut pcm).unwrap();
    tokio::time::timeout(Duration::from_secs(2), m.mixers[1].recv_link_one(&id0))
        .await
        .unwrap()
        .unwrap();
    let rx: &LinkReceiver = m.mixers[1].receiver(&id0).unwrap();
    assert_eq!(rx.gated_drops(), 1);
    assert_eq!(rx.jitter_len(), 0);
    let _ = id1;
}

#[tokio::test]
async fn mute_and_hold_fan_out_to_all_links() {
    let mut m = build_mesh(3).await;
    m.mixers[0].set_muted(true);
    let mut pcm = tone_frames(1, 440.0, 0.25);
    assert_eq!(m.mixers[0].send_pcm_frame(&mut pcm).unwrap(), 0);
    m.mixers[0].set_muted(false);
    m.mixers[0].set_held(true);
    assert_eq!(m.mixers[0].send_pcm_frame(&mut pcm).unwrap(), 0);
    m.mixers[0].set_held(false);
    assert_eq!(m.mixers[0].send_pcm_frame(&mut pcm).unwrap(), 2);
}

#[tokio::test]
async fn remove_link_leaves_remaining_links_working() {
    let mut m = build_mesh(3).await;
    let (id1, id2) = (m.ids[1], m.ids[2]);
    assert!(m.mixers[0].remove_link(&id1));
    assert!(!m.mixers[0].remove_link(&id1));
    assert_eq!(m.mixers[0].link_count(), 1);
    let mut pcm = tone_frames(1, 440.0, 0.25);
    assert_eq!(m.mixers[0].send_pcm_frame(&mut pcm).unwrap(), 1);
    assert!(m.mixers[0].sender(&id2).is_some());
}

#[tokio::test]
async fn fifth_participant_refused() {
    let mut m = build_mesh(4).await;
    assert_eq!(m.mixers[0].link_count(), 3);
    let extra = endpoint().await;
    let ((sa, tx_a, rx_a), (sb, _tx_b, _rx_b)) = roq_link(&m.eps[0], &extra).await;
    let hs = dummy_handshake(200);
    let (s, r) = mesh_link_srtp(
        &hs,
        CallRole::Caller,
        tx_a,
        rx_a,
        Arc::new(MediaCounters::default()),
    )
    .unwrap();
    let err = m.mixers[0]
        .add_link(*extra.id().as_bytes(), s, r)
        .unwrap_err();
    assert!(matches!(err, MediaError::MeshFull { max: 4 }), "{err}");
    drop((sa, sb));
}

#[tokio::test]
async fn plain_link_requires_explicit_confirm() {
    let a = endpoint().await;
    let b = endpoint().await;
    let ((_sa, tx_a, rx_a), (_sb, tx_b, rx_b)) = roq_link(&a, &b).await;
    let denied = mesh_link_plain(false, tx_a, rx_a, Arc::new(MediaCounters::default()));
    assert!(matches!(denied, Err(MediaError::MediaDeclined)));

    let (s, r) = mesh_link_plain(true, tx_b, rx_b, Arc::new(MediaCounters::default())).unwrap();
    assert_eq!(s.wire_mode(), saga_media::MediaWireMode::PlainRtp);
    assert_eq!(r.wire_mode(), saga_media::MediaWireMode::PlainRtp);
}
