//! Local-only helper: export clean / 5% / 15% degraded WAVs for STOI.
//! Run: `cargo test --test stoi_export -- --ignored --nocapture`

mod common;

use std::path::PathBuf;

use common::{fixture_speech, linked_pipelines, load_wav_mono_48k};
use hound::{WavSpec, WavWriter};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use saga_media::FRAME_SAMPLES;

fn write_wav(path: &PathBuf, pcm: &[i16]) {
    let spec = WavSpec {
        channels: 1,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = WavWriter::create(path, spec).unwrap();
    for &s in pcm {
        w.write_sample(s).unwrap();
    }
    w.finalize().unwrap();
}

async fn degrade(loss_rate: f64) -> Vec<i16> {
    let mut pair = linked_pipelines().await;
    let pcm = load_wav_mono_48k(fixture_speech().to_str().unwrap());
    let max = FRAME_SAMPLES * 200;
    let pcm = &pcm[..pcm.len().min(max)];
    let pcm = &pcm[..pcm.len() - (pcm.len() % FRAME_SAMPLES)];
    let frames = pcm.len() / FRAME_SAMPLES;
    let mut rng = StdRng::seed_from_u64(0x5101_u64);
    let mut played = Vec::with_capacity(pcm.len());
    if loss_rate > 0.0 {
        pair.caller
            .set_packet_loss_hint((loss_rate * 100.0) as i32)
            .unwrap();
    }
    for f in 0..frames {
        let mut frame = pcm[f * FRAME_SAMPLES..(f + 1) * FRAME_SAMPLES].to_vec();
        let protected = pair.caller.protect_pcm_for_test(&mut frame).unwrap();
        if rng.gen::<f64>() >= loss_rate {
            pair.callee.inject_framed_for_test(&protected);
        }
        let mut out = vec![0i16; FRAME_SAMPLES];
        let _ = pair.callee.playout_frame(&mut out);
        played.extend_from_slice(&out);
    }
    played
}

#[tokio::test]
#[ignore = "local STOI export — not CI"]
async fn export_degraded_wavs_for_stoi() {
    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/stoi");
    std::fs::create_dir_all(&out_dir).unwrap();
    let ref_pcm = load_wav_mono_48k(fixture_speech().to_str().unwrap());
    write_wav(&out_dir.join("ref.wav"), &ref_pcm);

    for (name, loss) in [("clean", 0.0), ("5pct", 0.05), ("15pct", 0.15)] {
        let deg = degrade(loss).await;
        let path = out_dir.join(format!("deg-{name}.wav"));
        write_wav(&path, &deg);
        eprintln!("wrote {}", path.display());
    }
}
