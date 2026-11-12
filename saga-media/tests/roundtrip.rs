//! Clean round-trip SNR / correlation thresholds.
//!
//! Thresholds (justify in TEST_RESULTS): Opus VoIP @ 32 kbps on a 1 kHz tone
//! tops out around SI-SNR ≈ 15–16 dB. Speech gate re-baselined on LibriSpeech
//! `5694-64038-0005` (CC BY 4.0).

mod common;

use common::{fixture_speech, linked_pipelines, load_wav_mono_48k, roundtrip_a_to_b, tone_pcm};
use saga_media::{signal_correlation, signal_snr_db, FRAME_SAMPLES};

const CORR_MIN: f64 = 0.85;
const SNR_MIN_TONE_DB: f64 = 15.0;
/// Re-baselined on LibriSpeech 5694-64038-0005: measured SI-SNR ≈ 11.1 dB.
const SNR_MIN_SPEECH_DB: f64 = 10.0;
/// Skip encoder/JB warmup and trailing edge when scoring.
const EDGE_FRAMES: usize = 5;

fn score(pcm: &[i16], out: &[i16]) -> (f64, f64) {
    let n = pcm.len().min(out.len());
    let edge = EDGE_FRAMES * FRAME_SAMPLES;
    let (a, b) = if n > edge * 2 {
        (&pcm[edge..n - edge], &out[edge..n - edge])
    } else {
        (&pcm[..n], &out[..n])
    };
    (
        signal_correlation(a, b, FRAME_SAMPLES * 4),
        signal_snr_db(a, b),
    )
}

#[tokio::test]
async fn tone_1khz_roundtrip() {
    let mut pair = linked_pipelines().await;
    let pcm = tone_pcm(50, 1000.0); // 1 s
    let out = roundtrip_a_to_b(&mut pair, &pcm).await;
    let (corr, snr) = score(&pcm, &out);
    eprintln!("tone corr={corr:.3} snr={snr:.1} dB");
    assert!(corr >= CORR_MIN, "corr {corr} < {CORR_MIN}");
    assert!(snr >= SNR_MIN_TONE_DB, "snr {snr} < {SNR_MIN_TONE_DB}");
}

#[tokio::test]
async fn speech_clip_roundtrip() {
    let mut pair = linked_pipelines().await;
    let pcm = load_wav_mono_48k(fixture_speech().to_str().unwrap());
    // Trim to whole frames, max ~2 s for CI speed.
    let max = FRAME_SAMPLES * 100;
    let pcm = &pcm[..pcm.len().min(max)];
    let pcm = &pcm[..pcm.len() - (pcm.len() % FRAME_SAMPLES)];
    let out = roundtrip_a_to_b(&mut pair, pcm).await;
    let (corr, snr) = score(pcm, &out);
    eprintln!("speech corr={corr:.3} snr={snr:.1} dB");
    assert!(corr >= CORR_MIN, "corr {corr} < {CORR_MIN}");
    assert!(snr >= SNR_MIN_SPEECH_DB, "snr {snr} < {SNR_MIN_SPEECH_DB}");
}
