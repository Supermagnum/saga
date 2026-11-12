//! Simulated loss / jitter / reorder — lower correlation thresholds.

mod common;

use common::{linked_pipelines, tone_pcm};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use saga_media::{signal_correlation, FRAME_SAMPLES};

#[tokio::test]
async fn loss_5_percent() {
    run_loss(0.05, 0.50).await;
}

#[tokio::test]
async fn loss_15_percent() {
    run_loss(0.15, 0.30).await;
}

async fn run_loss(loss_rate: f64, corr_min: f64) {
    let mut pair = linked_pipelines().await;
    let pcm = tone_pcm(80, 1000.0);
    let frames = pcm.len() / FRAME_SAMPLES;
    let mut rng = StdRng::seed_from_u64(0x5A6A_1055);
    let mut played = Vec::new();

    pair.caller
        .set_packet_loss_hint((loss_rate * 100.0) as i32)
        .unwrap();

    for f in 0..frames {
        let mut frame = pcm[f * FRAME_SAMPLES..(f + 1) * FRAME_SAMPLES].to_vec();
        let protected = pair.caller.protect_pcm_for_test(&mut frame).unwrap();
        // Random loss + occasional reorder via delayed inject.
        if rng.gen::<f64>() < loss_rate {
            continue;
        }
        if rng.gen::<f64>() < 0.1 {
            // hold one, inject next first (reorder)
            let mut frame2 = if f + 1 < frames {
                pcm[(f + 1) * FRAME_SAMPLES..(f + 2) * FRAME_SAMPLES].to_vec()
            } else {
                frame.clone()
            };
            let p2 = pair.caller.protect_pcm_for_test(&mut frame2).unwrap();
            pair.callee.inject_srtp_for_test(&p2);
            pair.callee.inject_srtp_for_test(&protected);
        } else {
            pair.callee.inject_srtp_for_test(&protected);
        }
        // Simulate jitter delay without sleeping much.
        let mut out = vec![0i16; FRAME_SAMPLES];
        let _ = pair.callee.playout_frame(&mut out);
        played.extend_from_slice(&out);
        assert!(
            pair.callee.jitter_len() <= pair.callee.jitter_cap(),
            "jitter buffer unbounded"
        );
    }
    for _ in 0..32 {
        let mut out = vec![0i16; FRAME_SAMPLES];
        let _ = pair.callee.playout_frame(&mut out);
        played.extend_from_slice(&out);
    }
    let n = pcm.len().min(played.len());
    let corr = signal_correlation(&pcm[..n], &played[..n], FRAME_SAMPLES * 8);
    eprintln!("loss={loss_rate} corr={corr:.3}");
    assert!(
        corr >= corr_min,
        "corr {corr} < {corr_min} at loss {loss_rate}"
    );
}
