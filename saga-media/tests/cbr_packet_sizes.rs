//! Packet-size / bitrate helper for Opus CBR switch.
//! Run: `cargo test -p saga-media --test cbr_packet_sizes -- --nocapture`

mod common;

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use common::{fixture_speech, load_wav_mono_48k};
use hound::{SampleFormat, WavSpec, WavWriter};
use saga_media::{
    signal_correlation, signal_snr_db, OpusDecoder, OpusEncoder, FRAME_SAMPLES, SAMPLE_RATE,
    TARGET_BITRATE,
};

fn encode_sizes(pcm: &[i16]) -> (Vec<usize>, Vec<i16>) {
    let mut enc = OpusEncoder::new().expect("enc");
    let mut dec = OpusDecoder::new().expect("dec");
    let mut out = vec![0u8; 4000];
    let mut sizes = Vec::new();
    let mut decoded = Vec::new();
    let mut frame = vec![0i16; FRAME_SAMPLES];
    let mut pcm_out = vec![0i16; FRAME_SAMPLES];
    let mut off = 0usize;
    while off + FRAME_SAMPLES <= pcm.len() {
        frame.copy_from_slice(&pcm[off..off + FRAME_SAMPLES]);
        let n = enc.encode_frame(&mut frame, &mut out).expect("encode");
        sizes.push(n);
        let dn = dec
            .decode_frame(&out[..n], &mut pcm_out, false)
            .expect("decode");
        decoded.extend_from_slice(&pcm_out[..dn]);
        off += FRAME_SAMPLES;
    }
    (sizes, decoded)
}

fn size_stats(sizes: &[usize]) -> (usize, usize, f64, f64, usize) {
    let min = *sizes.iter().min().unwrap();
    let max = *sizes.iter().max().unwrap();
    let mean = sizes.iter().sum::<usize>() as f64 / sizes.len() as f64;
    let var = sizes
        .iter()
        .map(|s| {
            let d = *s as f64 - mean;
            d * d
        })
        .sum::<f64>()
        / sizes.len() as f64;
    let std_dev = var.sqrt();
    let mut uniq = sizes.to_vec();
    uniq.sort_unstable();
    uniq.dedup();
    (min, max, mean, std_dev, uniq.len())
}

#[test]
fn report_packet_size_distribution() {
    let fixture = fixture_speech();
    let pcm = load_wav_mono_48k(fixture.to_str().unwrap());
    let pcm = &pcm[..pcm.len() - (pcm.len() % FRAME_SAMPLES)];
    let (sizes, decoded) = encode_sizes(pcm);
    let (min, max, mean, std_dev, unique) = size_stats(&sizes);
    let bytes_total: usize = sizes.iter().sum();
    let duration_s = (sizes.len() * FRAME_SAMPLES) as f64 / SAMPLE_RATE as f64;
    let bitrate_kbps = (bytes_total as f64 * 8.0) / duration_s / 1000.0;
    let edge = 5 * FRAME_SAMPLES;
    let (a, b) = if pcm.len().min(decoded.len()) > edge * 2 {
        (
            &pcm[edge..pcm.len().min(decoded.len()) - edge],
            &decoded[edge..pcm.len().min(decoded.len()) - edge],
        )
    } else {
        (&pcm[..pcm.len().min(decoded.len())], &decoded[..pcm.len().min(decoded.len())])
    };
    let corr = signal_correlation(a, b, FRAME_SAMPLES * 4);
    let si = signal_snr_db(a, b);

    let mut hist = BTreeMap::<usize, usize>::new();
    for &s in &sizes {
        *hist.entry(s).or_default() += 1;
    }

    println!("TARGET_BITRATE={TARGET_BITRATE}");
    println!(
        "frames={} duration_s={duration_s:.3} bitrate_kbps={bitrate_kbps:.2}",
        sizes.len()
    );
    println!(
        "size_min={min} size_max={max} size_mean={mean:.2} size_std={std_dev:.2} unique_sizes={unique}"
    );
    println!("corr={corr:.4} si_snr_db={si:.2}");
    println!("histogram (size->count):");
    for (k, v) in hist.iter().take(50) {
        println!("  {k}: {v}");
    }
    if hist.len() > 50 {
        println!("  ... {} more buckets", hist.len() - 50);
    }

    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/cbr_measure");
    std::fs::create_dir_all(&out_dir).ok();
    let out_wav = out_dir.join("decoded.wav");
    let spec = WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut w = WavWriter::create(&out_wav, spec).expect("wav");
    for s in &decoded {
        w.write_sample(*s).unwrap();
    }
    w.finalize().unwrap();
    println!("decoded_wav={}", out_wav.display());

    let sizes_path = out_dir.join("sizes.txt");
    let mut f = File::create(&sizes_path).unwrap();
    for s in &sizes {
        writeln!(f, "{s}").unwrap();
    }
    println!("sizes_path={}", sizes_path.display());

    // CBR gate: Opus payload size must be constant so ciphertext length
    // does not track speech (DTX remains off).
    assert_eq!(unique, 1, "CBR expected a single Opus payload size, got {unique}");
    assert_eq!(min, max, "CBR size range {min}..={max}");
    assert!(
        (bitrate_kbps - 32.0).abs() < 0.05,
        "CBR bitrate {bitrate_kbps} kbps not ~32"
    );
    assert!(corr >= 0.85, "corr {corr}");
    assert!(si >= 10.0, "si_snr {si}");
}
