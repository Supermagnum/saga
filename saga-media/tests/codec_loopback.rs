//! Codec-only loopback: measures Opus SI-SNR ceiling without RoQ/SRTP.
mod common;
use common::tone_pcm;
use saga_media::{signal_correlation, signal_snr_db, OpusDecoder, OpusEncoder, FRAME_SAMPLES};

#[test]
fn opus_tone_loopback_ceiling() {
    let mut enc = OpusEncoder::new().unwrap();
    let mut dec = OpusDecoder::new().unwrap();
    let pcm = tone_pcm(50, 1000.0);
    let frames = pcm.len() / FRAME_SAMPLES;
    let mut out = Vec::new();
    let mut buf = [0u8; 400];
    for f in 0..frames {
        let mut frame = pcm[f * FRAME_SAMPLES..(f + 1) * FRAME_SAMPLES].to_vec();
        let n = enc.encode_frame(&mut frame, &mut buf).unwrap();
        let mut pcm_out = vec![0i16; FRAME_SAMPLES];
        let m = dec.decode_frame(&buf[..n], &mut pcm_out, false).unwrap();
        out.extend_from_slice(&pcm_out[..m]);
    }
    let edge = 5 * FRAME_SAMPLES;
    let a = &pcm[edge..pcm.len() - edge];
    let b = &out[edge..out.len().min(pcm.len()) - edge];
    let corr = signal_correlation(a, b, FRAME_SAMPLES * 4);
    let snr = signal_snr_db(a, b);
    eprintln!("opus-only tone corr={corr:.4} snr={snr:.1} dB");
    assert!(corr >= 0.85);
}
