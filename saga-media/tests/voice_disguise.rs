//! Host §16 voice-disguise: each mode × strength on the LibriSpeech fixture →
//! before/after WAVs under `docs/voice-disguise/`, plus latency and STOI notes.
//!
//! Run: `cargo test -p saga-media --test voice_disguise -- --nocapture`
//! Then score with `testing/e2e/pcm_score.py --ref … --deg-wav …`.

use std::path::PathBuf;
use std::time::Instant;

use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use saga_media::{
    disguise_begin_call, disguise_end_call, disguise_last_alpha_for_test, set_disguise_settings,
    DisguiseMode, DisguiseSettings, FormantStrength, FRAME_SAMPLES, SAMPLE_RATE,
};

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/speech_clip_48k.wav")
}

fn out_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/voice-disguise")
}

fn read_wav(path: &std::path::Path) -> Vec<i16> {
    let mut reader = WavReader::open(path).expect("open fixture");
    assert_eq!(reader.spec().sample_rate, SAMPLE_RATE);
    assert_eq!(reader.spec().channels, 1);
    reader
        .samples::<i16>()
        .map(|s| s.expect("sample"))
        .collect()
}

fn write_wav(path: &std::path::Path, pcm: &[i16]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    let spec = WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut w = WavWriter::create(path, spec).expect("create wav");
    for &s in pcm {
        w.write_sample(s).expect("write");
    }
    w.finalize().expect("finalize");
}

fn process_named(name: &str, settings: DisguiseSettings) -> (f64, f32) {
    let mut pcm = read_wav(&fixture_path());
    let rem = pcm.len() % FRAME_SAMPLES;
    if rem != 0 {
        pcm.extend(std::iter::repeat_n(0i16, FRAME_SAMPLES - rem));
    }
    set_disguise_settings(settings);
    disguise_begin_call();
    let alpha = disguise_last_alpha_for_test();
    let t0 = Instant::now();
    for chunk in pcm.chunks_mut(FRAME_SAMPLES) {
        if chunk.len() == FRAME_SAMPLES {
            saga_media::disguise_pcm_inplace(chunk);
        }
    }
    let wall_ms = t0.elapsed().as_secs_f64() * 1000.0;
    disguise_end_call();

    let out = out_dir().join(format!("{name}.wav"));
    write_wav(&out, &pcm);
    (wall_ms, alpha)
}

fn measure_frame_latency_ms(settings: DisguiseSettings) -> f64 {
    let pcm_full = read_wav(&fixture_path());
    let max_start = pcm_full.len().saturating_sub(FRAME_SAMPLES).max(1);
    set_disguise_settings(settings);
    disguise_begin_call();
    // Warm-up.
    for i in 0..5 {
        let start = (i * FRAME_SAMPLES) % max_start;
        let mut frame = pcm_full[start..start + FRAME_SAMPLES].to_vec();
        saga_media::disguise_pcm_inplace(&mut frame);
    }
    let frames = 50usize;
    let t0 = Instant::now();
    for i in 0..frames {
        let start = (i * FRAME_SAMPLES) % max_start;
        let mut frame = pcm_full[start..start + FRAME_SAMPLES].to_vec();
        saga_media::disguise_pcm_inplace(&mut frame);
    }
    let elapsed = t0.elapsed().as_secs_f64() * 1000.0;
    disguise_end_call();
    elapsed / frames as f64
}

#[test]
fn export_modes_and_measure_latency() {
    let original = read_wav(&fixture_path());
    write_wav(&out_dir().join("original.wav"), &original);

    let cases: &[(&str, DisguiseSettings)] = &[
        (
            "pitch_m3",
            DisguiseSettings {
                mode: DisguiseMode::Pitch,
                pitch_semitones: -3.0,
                formant_strength: FormantStrength::Medium,
                random_each_call: false,
                call_enabled: true,
            },
        ),
        (
            "formant_mild",
            DisguiseSettings {
                mode: DisguiseMode::Formant,
                pitch_semitones: -3.0,
                formant_strength: FormantStrength::Mild,
                random_each_call: false,
                call_enabled: true,
            },
        ),
        (
            "formant_medium",
            DisguiseSettings {
                mode: DisguiseMode::Formant,
                pitch_semitones: -3.0,
                formant_strength: FormantStrength::Medium,
                random_each_call: false,
                call_enabled: true,
            },
        ),
        (
            "formant_strong",
            DisguiseSettings {
                mode: DisguiseMode::Formant,
                pitch_semitones: -3.0,
                formant_strength: FormantStrength::Strong,
                random_each_call: false,
                call_enabled: true,
            },
        ),
        (
            "formant_pitch_medium",
            DisguiseSettings {
                mode: DisguiseMode::FormantPitch,
                pitch_semitones: -3.0,
                formant_strength: FormantStrength::Medium,
                random_each_call: false,
                call_enabled: true,
            },
        ),
    ];

    let mut report = String::from("# Voice disguise host metrics\n\n");
    report.push_str("| Mode | Wall (full clip ms) | Per-frame ms | α |\n");
    report.push_str("|------|---------------------:|-------------:|--:|\n");

    for (name, settings) in cases {
        let (wall, alpha) = process_named(name, *settings);
        let frame_ms = measure_frame_latency_ms(*settings);
        assert!(
            frame_ms <= 20.0,
            "{name} per-frame latency {frame_ms:.2} ms exceeds 20 ms budget"
        );
        report.push_str(&format!(
            "| {name} | {wall:.1} | {frame_ms:.3} | {alpha:.4} |\n"
        ));
        println!("{name}: wall={wall:.1}ms frame={frame_ms:.3}ms alpha={alpha:.4}");
    }

    // Two calls with random-each-call must draw different α.
    let s = DisguiseSettings {
        mode: DisguiseMode::Formant,
        pitch_semitones: -3.0,
        formant_strength: FormantStrength::Medium,
        random_each_call: true,
        call_enabled: true,
    };
    set_disguise_settings(s);
    disguise_begin_call();
    let a1 = disguise_last_alpha_for_test();
    disguise_end_call();
    disguise_begin_call();
    let a2 = disguise_last_alpha_for_test();
    disguise_end_call();
    assert!(
        (a1 - a2).abs() > 1e-6,
        "random-each-call drew identical α: {a1} vs {a2}"
    );
    report.push_str(&format!(
        "\nRandom-each-call α draws: {a1:.4} vs {a2:.4} (distinct).\n"
    ));
    report.push_str(
        "\nDSP is pure Rust (Levinson–Durbin LPC + McAdams-style envelope warp + \
         pitch resample). No GPL/commercial DSP dependency. Compatible with AGPL-3.0 \
         and F-Droid.\n",
    );

    let metrics = out_dir().join("HOST_METRICS.md");
    std::fs::write(&metrics, report).expect("write metrics");
}
