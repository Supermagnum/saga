//! Host CI: saga-peer A ↔ B with LibriSpeech fixture (encrypted + plain).

use std::path::PathBuf;

use saga_media::{signal_correlation, signal_snr_db, FRAME_SAMPLES};
use saga_peer::call::host_pair_roundtrip;
use saga_peer::load_wav_mono_48k;

const CORR_MIN: f64 = 0.85;
const SNR_MIN_SPEECH_DB: f64 = 10.0;
const EDGE_FRAMES: usize = 5;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../saga-media/fixtures/speech_clip_48k.wav")
}

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

fn clip_pcm() -> Vec<i16> {
    let pcm = load_wav_mono_48k(&fixture()).expect("fixture");
    let max = FRAME_SAMPLES * 100; // ~2 s
    let pcm = &pcm[..pcm.len().min(max)];
    let pcm = &pcm[..pcm.len() - (pcm.len() % FRAME_SAMPLES)];
    pcm.to_vec()
}

#[tokio::test]
async fn encrypted_speech_roundtrip() {
    let pcm = clip_pcm();
    let (out, _ca, _cb) = host_pair_roundtrip(&pcm, true).await.expect("encrypted");
    let (corr, snr) = score(&pcm, &out);
    eprintln!("encrypted speech corr={corr:.3} snr={snr:.1} dB");
    assert!(corr >= CORR_MIN, "corr {corr} < {CORR_MIN}");
    assert!(snr >= SNR_MIN_SPEECH_DB, "snr {snr} < {SNR_MIN_SPEECH_DB}");
}

#[tokio::test]
async fn plain_speech_roundtrip() {
    let pcm = clip_pcm();
    let (out, _ca, _cb) = host_pair_roundtrip(&pcm, false).await.expect("plain");
    let (corr, snr) = score(&pcm, &out);
    eprintln!("plain speech corr={corr:.3} snr={snr:.1} dB");
    assert!(corr >= CORR_MIN, "corr {corr} < {CORR_MIN}");
    assert!(snr >= SNR_MIN_SPEECH_DB, "snr {snr} < {SNR_MIN_SPEECH_DB}");
}
