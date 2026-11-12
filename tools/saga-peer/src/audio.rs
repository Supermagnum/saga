//! WAV I/O at 48 kHz mono and optional cpal live devices.

use std::path::Path;

use anyhow::{bail, Context, Result};
use saga_media::{FRAME_SAMPLES, SAMPLE_RATE};

pub fn load_wav_mono_48k(path: &Path) -> Result<Vec<i16>> {
    let mut reader = hound::WavReader::open(path).with_context(|| format!("open {path:?}"))?;
    let spec = reader.spec();
    if spec.sample_rate != SAMPLE_RATE {
        bail!(
            "wav sample rate {} != {} (resample offline or use live-audio)",
            spec.sample_rate,
            SAMPLE_RATE
        );
    }
    if spec.channels != 1 {
        bail!("wav must be mono (got {} channels)", spec.channels);
    }
    Ok(reader
        .samples::<i16>()
        .map(|s| s.expect("sample"))
        .collect())
}

pub fn write_wav_mono_48k(path: &Path, pcm: &[i16]) -> Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer =
        hound::WavWriter::create(path, spec).with_context(|| format!("create {path:?}"))?;
    for &s in pcm {
        writer.write_sample(s)?;
    }
    writer.finalize()?;
    Ok(())
}

/// Frame source for the send path: WAV buffer or (with `live-audio`) cpal mic.
pub enum CaptureSource {
    Wav {
        samples: Vec<i16>,
        cursor: usize,
        loop_clip: bool,
    },
    Silence,
    #[cfg(feature = "live-audio")]
    Live(LiveCapture),
}

impl CaptureSource {
    pub fn from_wav(path: &Path, loop_clip: bool) -> Result<Self> {
        Ok(Self::Wav {
            samples: load_wav_mono_48k(path)?,
            cursor: 0,
            loop_clip,
        })
    }

    /// Fill one 20 ms frame. Returns false when WAV is exhausted (non-loop).
    pub fn next_frame(&mut self, out: &mut [i16], muted: bool) -> bool {
        out.fill(0);
        if muted {
            return true;
        }
        match self {
            CaptureSource::Silence => true,
            CaptureSource::Wav {
                samples,
                cursor,
                loop_clip,
            } => {
                if *cursor + FRAME_SAMPLES > samples.len() {
                    if *loop_clip && !samples.is_empty() {
                        *cursor = 0;
                    } else {
                        return false;
                    }
                }
                let end = (*cursor + FRAME_SAMPLES).min(samples.len());
                let n = end - *cursor;
                out[..n].copy_from_slice(&samples[*cursor..end]);
                *cursor = end;
                true
            }
            #[cfg(feature = "live-audio")]
            CaptureSource::Live(live) => {
                live.pull_frame(out);
                true
            }
        }
    }
}

pub struct PlayoutSink {
    pub recorded: Vec<i16>,
    wav_out: Option<std::path::PathBuf>,
    #[cfg(feature = "live-audio")]
    live: Option<LivePlayback>,
}

impl PlayoutSink {
    pub fn new(wav_out: Option<&Path>) -> Result<Self> {
        Ok(Self {
            recorded: Vec::new(),
            wav_out: wav_out.map(|p| p.to_path_buf()),
            #[cfg(feature = "live-audio")]
            live: None,
        })
    }

    #[cfg(feature = "live-audio")]
    pub fn with_live(mut self, device_name: Option<&str>) -> Result<Self> {
        self.live = Some(LivePlayback::open(device_name)?);
        Ok(self)
    }

    pub fn push_frame(&mut self, frame: &[i16]) {
        self.recorded.extend_from_slice(frame);
        #[cfg(feature = "live-audio")]
        if let Some(live) = self.live.as_mut() {
            live.push_frame(frame);
        }
    }

    pub fn finalize(&mut self) -> Result<()> {
        if let Some(path) = self.wav_out.take() {
            write_wav_mono_48k(&path, &self.recorded)?;
        }
        Ok(())
    }
}

impl Drop for PlayoutSink {
    fn drop(&mut self) {
        // Harness may SIGTERM the peer; still flush --wav-out.
        let _ = self.finalize();
    }
}

#[cfg(feature = "live-audio")]
mod live {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use rubato::{
        Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
    };

    pub struct LiveCapture {
        _stream: cpal::Stream,
        ring: Arc<Mutex<Vec<i16>>>,
        read_pos: usize,
    }

    impl LiveCapture {
        pub fn open(device_name: Option<&str>) -> Result<Self> {
            let host = cpal::default_host();
            let device = pick_input(&host, device_name)?;
            let config = device.default_input_config()?.into();
            let ring = Arc::new(Mutex::new(Vec::new()));
            let ring_w = ring.clone();
            let sr = config.sample_rate.0;
            let channels = config.channels as usize;
            let stream = device.build_input_stream(
                &config,
                move |data: &[f32], _| {
                    let mono = downmix_f32(data, channels);
                    let pcm = if sr == SAMPLE_RATE {
                        f32_to_i16(&mono)
                    } else {
                        resample_to_48k(&mono, sr).unwrap_or_default()
                    };
                    if let Ok(mut g) = ring_w.lock() {
                        g.extend_from_slice(&pcm);
                    }
                },
                |e| eprintln!("cpal input error: {e}"),
                None,
            )?;
            stream.play()?;
            Ok(Self {
                _stream: stream,
                ring,
                read_pos: 0,
            })
        }

        pub fn pull_frame(&mut self, out: &mut [i16]) {
            out.fill(0);
            let Ok(buf) = self.ring.lock() else {
                return;
            };
            if self.read_pos + FRAME_SAMPLES <= buf.len() {
                out.copy_from_slice(&buf[self.read_pos..self.read_pos + FRAME_SAMPLES]);
                self.read_pos += FRAME_SAMPLES;
            }
        }
    }

    pub struct LivePlayback {
        _stream: cpal::Stream,
        ring: Arc<Mutex<Vec<i16>>>,
        running: Arc<AtomicBool>,
    }

    impl LivePlayback {
        pub fn open(device_name: Option<&str>) -> Result<Self> {
            let host = cpal::default_host();
            let device = pick_output(&host, device_name)?;
            let config = device.default_output_config()?.into();
            let ring = Arc::new(Mutex::new(Vec::new()));
            let ring_r = ring.clone();
            let running = Arc::new(AtomicBool::new(true));
            let run = running.clone();
            let sr = config.sample_rate.0;
            let channels = config.channels as usize;
            let stream = device.build_output_stream(
                &config,
                move |data: &mut [f32], _| {
                    if !run.load(Ordering::Relaxed) {
                        data.fill(0.0);
                        return;
                    }
                    let need = data.len() / channels;
                    let mut mono = vec![0i16; need];
                    if let Ok(mut g) = ring_r.lock() {
                        let n = need.min(g.len());
                        mono[..n].copy_from_slice(&g[..n]);
                        g.drain(..n);
                    }
                    let f = if sr == SAMPLE_RATE {
                        i16_to_f32(&mono)
                    } else {
                        resample_from_48k(&mono, sr).unwrap_or_else(|_| vec![0.0; need])
                    };
                    for (i, frame) in f.iter().enumerate() {
                        for c in 0..channels {
                            if let Some(s) = data.get_mut(i * channels + c) {
                                *s = *frame;
                            }
                        }
                    }
                },
                |e| eprintln!("cpal output error: {e}"),
                None,
            )?;
            stream.play()?;
            Ok(Self {
                _stream: stream,
                ring,
                running,
            })
        }

        pub fn push_frame(&self, frame: &[i16]) {
            if let Ok(mut g) = self.ring.lock() {
                g.extend_from_slice(frame);
            }
        }
    }

    pub fn list_devices() -> Result<()> {
        let host = cpal::default_host();
        println!("input devices:");
        for d in host.input_devices()? {
            println!("  {}", d.name().unwrap_or_else(|_| "?".into()));
        }
        println!("output devices:");
        for d in host.output_devices()? {
            println!("  {}", d.name().unwrap_or_else(|_| "?".into()));
        }
        Ok(())
    }

    fn pick_input(host: &cpal::Host, name: Option<&str>) -> Result<cpal::Device> {
        if let Some(n) = name {
            for d in host.input_devices()? {
                if d.name().ok().as_deref() == Some(n) {
                    return Ok(d);
                }
            }
            bail!("input device [{n}] not found");
        }
        host.default_input_device()
            .context("no default input device")
    }

    fn pick_output(host: &cpal::Host, name: Option<&str>) -> Result<cpal::Device> {
        if let Some(n) = name {
            for d in host.output_devices()? {
                if d.name().ok().as_deref() == Some(n) {
                    return Ok(d);
                }
            }
            bail!("output device [{n}] not found");
        }
        host.default_output_device()
            .context("no default output device")
    }

    fn downmix_f32(data: &[f32], channels: usize) -> Vec<f32> {
        if channels <= 1 {
            return data.to_vec();
        }
        data.chunks(channels)
            .map(|c| c.iter().sum::<f32>() / channels as f32)
            .collect()
    }

    fn f32_to_i16(v: &[f32]) -> Vec<i16> {
        v.iter()
            .map(|x| (x.clamp(-1.0, 1.0) * 32767.0) as i16)
            .collect()
    }

    fn i16_to_f32(v: &[i16]) -> Vec<f32> {
        v.iter().map(|x| *x as f32 / 32768.0).collect()
    }

    fn sinc_params() -> SincInterpolationParameters {
        SincInterpolationParameters {
            sinc_len: 64,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 128,
            window: WindowFunction::BlackmanHarris2,
        }
    }

    fn resample_to_48k(input: &[f32], from_hz: u32) -> Result<Vec<i16>> {
        let ratio = SAMPLE_RATE as f64 / from_hz as f64;
        let mut resampler =
            SincFixedIn::<f32>::new(ratio, 2.0, sinc_params(), input.len().max(64), 1)?;
        let waves = vec![input.to_vec()];
        let out = resampler.process(&waves, None)?;
        Ok(f32_to_i16(&out[0]))
    }

    fn resample_from_48k(input: &[i16], to_hz: u32) -> Result<Vec<f32>> {
        let ratio = to_hz as f64 / SAMPLE_RATE as f64;
        let f = i16_to_f32(input);
        let mut resampler = SincFixedIn::<f32>::new(ratio, 2.0, sinc_params(), f.len().max(64), 1)?;
        let waves = vec![f];
        let out = resampler.process(&waves, None)?;
        Ok(out[0].clone())
    }
}

#[cfg(feature = "live-audio")]
pub use live::{list_devices, LiveCapture, LivePlayback};
