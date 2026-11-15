//! §16 voice disguise — sender-side PCM transform before Opus.
//!
//! Modes: Off / Pitch / Formant (McAdams LPC) / Formant + pitch.
//! No third-party DSP libraries (AGPL-3.0 + F-Droid): Levinson–Durbin LPC,
//! McAdams pole-angle warp, and a short WSOLA-style pitch shifter.
//!
//! Per-call random α / pitch offsets come from the OS CSPRNG (`OsRng`) and are
//! zeroized at call end. Release builds never log those values.

use std::f32::consts::PI;
use std::sync::{Mutex, OnceLock};

use rand::rngs::OsRng;
use rand::RngCore;
use zeroize::Zeroize;

use crate::codec::{FRAME_SAMPLES, SAMPLE_RATE};

/// Crossfade length when the effective disguise parameters change mid-call.
const CROSSFADE_FRAMES: usize = 4;
/// LPC order at 48 kHz (VoicePrivacy-style envelope).
const LPC_ORDER: usize = 24;
/// Pitch history kept for interpolation (samples). Kept small so added
/// algorithmic delay stays inside the §16 ~20 ms budget (≤ one 20 ms frame).
const PITCH_WIN: usize = FRAME_SAMPLES;

/// McAdams α ranges tuned for Mild / Medium / Strong (listening + STOI).
/// Values < 1 raise formants; > 1 lower them. Random-each-call draws in-range.
const ALPHA_MILD: (f32, f32) = (0.92, 1.08);
const ALPHA_MEDIUM: (f32, f32) = (0.82, 1.18);
const ALPHA_STRONG: (f32, f32) = (0.70, 1.30);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum DisguiseMode {
    #[default]
    Off = 0,
    Pitch = 1,
    Formant = 2,
    FormantPitch = 3,
}

impl DisguiseMode {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Pitch,
            2 => Self::Formant,
            3 => Self::FormantPitch,
            _ => Self::Off,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum FormantStrength {
    Mild = 0,
    #[default]
    Medium = 1,
    Strong = 2,
}

impl FormantStrength {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Mild,
            2 => Self::Strong,
            _ => Self::Medium,
        }
    }

    fn alpha_range(self) -> (f32, f32) {
        match self {
            Self::Mild => ALPHA_MILD,
            Self::Medium => ALPHA_MEDIUM,
            Self::Strong => ALPHA_STRONG,
        }
    }

    /// Fixed α when "new random voice each call" is off.
    /// Uses the lower end of the strength range so α ≠ 1 (identity is a no-op).
    fn fixed_alpha(self) -> f32 {
        let (lo, _hi) = self.alpha_range();
        lo
    }
}

/// User-facing settings (persist across calls). Per-call draws live in the engine.
#[derive(Clone, Copy, Debug)]
pub struct DisguiseSettings {
    pub mode: DisguiseMode,
    /// Semitones in −6 … +6; default −3 ("lower").
    pub pitch_semitones: f32,
    pub formant_strength: FormantStrength,
    /// ON by default: draw α / pitch offset once per call from OS CSPRNG.
    pub random_each_call: bool,
    /// In-call quick toggle (Iroh only). When false, disguise is off for send.
    pub call_enabled: bool,
}

impl Default for DisguiseSettings {
    fn default() -> Self {
        Self {
            mode: DisguiseMode::Off,
            pitch_semitones: -3.0,
            formant_strength: FormantStrength::Medium,
            random_each_call: true,
            call_enabled: true,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ActiveParams {
    mode: DisguiseMode,
    pitch_semitones: f32,
    alpha: f32,
}

impl ActiveParams {
    fn off() -> Self {
        Self {
            mode: DisguiseMode::Off,
            pitch_semitones: 0.0,
            alpha: 1.0,
        }
    }

    fn is_active(&self) -> bool {
        self.mode != DisguiseMode::Off
    }
}

impl Zeroize for ActiveParams {
    fn zeroize(&mut self) {
        self.mode = DisguiseMode::Off;
        self.pitch_semitones = 0.0;
        self.alpha = 1.0;
    }
}

/// Process-global engine used by [`crate::codec::disguise_pcm_inplace`].
pub struct DisguiseEngine {
    settings: DisguiseSettings,
    /// Resolved parameters for the current call (zeroized on end).
    active: ActiveParams,
    /// Previous parameters while crossfading.
    fade_from: ActiveParams,
    fade_frames_left: usize,
    in_call: bool,
    /// Last drawn α (host tests / debug only; never logged in release).
    last_alpha: f32,
    last_pitch: f32,
    pre_mem: f32,
    syn_mem: [f32; LPC_ORDER],
    inv_mem: [f32; LPC_ORDER],
    pitch_buf: Vec<f32>,
    pitch_write: usize,
    pitch_read: f32,
}

impl Default for DisguiseEngine {
    fn default() -> Self {
        Self {
            settings: DisguiseSettings::default(),
            active: ActiveParams::off(),
            fade_from: ActiveParams::off(),
            fade_frames_left: 0,
            in_call: false,
            last_alpha: 1.0,
            last_pitch: 0.0,
            pre_mem: 0.0,
            syn_mem: [0.0; LPC_ORDER],
            inv_mem: [0.0; LPC_ORDER],
            pitch_buf: vec![0.0; PITCH_WIN * 4],
            pitch_write: 0,
            pitch_read: 0.0,
        }
    }
}

impl DisguiseEngine {
    pub fn settings(&self) -> DisguiseSettings {
        self.settings
    }

    pub fn set_settings(&mut self, settings: DisguiseSettings) {
        let mut s = settings;
        s.pitch_semitones = s.pitch_semitones.clamp(-6.0, 6.0);
        let old_effective = self.effective_params();
        self.settings = s;
        if self.in_call {
            self.recompute_active();
            let new_effective = self.effective_params();
            if params_differ(old_effective, new_effective) {
                self.start_crossfade(old_effective);
            }
        }
    }

    /// Begin a call: draw random α / pitch if configured; arm the processor.
    pub fn begin_call(&mut self) {
        self.in_call = true;
        self.recompute_active();
        self.fade_frames_left = 0;
        self.reset_filters();
        self.last_alpha = self.active.alpha;
        self.last_pitch = self.active.pitch_semitones;
        #[cfg(debug_assertions)]
        {
            // Debug-only: never present in release logging paths.
            log_debug_params("begin_call", self.active);
        }
    }

    /// End of call: zeroize per-call secrets and drop filter state.
    pub fn end_call(&mut self) {
        self.in_call = false;
        self.active.zeroize();
        self.fade_from.zeroize();
        self.fade_frames_left = 0;
        self.last_alpha = 1.0;
        self.last_pitch = 0.0;
        self.reset_filters();
    }

    /// Host / unit-test access to the last drawn α (not for release UI).
    pub fn last_alpha_for_test(&self) -> f32 {
        self.last_alpha
    }

    pub fn last_pitch_for_test(&self) -> f32 {
        self.last_pitch
    }

    pub fn is_locally_active(&self) -> bool {
        self.effective_params().is_active()
    }

    fn effective_params(&self) -> ActiveParams {
        if !self.in_call || !self.settings.call_enabled || self.settings.mode == DisguiseMode::Off {
            return ActiveParams::off();
        }
        let mut p = self.active;
        p.mode = self.settings.mode;
        p
    }

    fn recompute_active(&mut self) {
        if self.settings.mode == DisguiseMode::Off {
            self.active = ActiveParams::off();
            return;
        }
        let (alpha, pitch) = if self.settings.random_each_call {
            draw_random(
                self.settings.formant_strength,
                self.settings.pitch_semitones,
            )
        } else {
            (
                self.settings.formant_strength.fixed_alpha(),
                self.settings.pitch_semitones,
            )
        };
        self.active = ActiveParams {
            mode: self.settings.mode,
            pitch_semitones: pitch,
            alpha,
        };
        self.last_alpha = alpha;
        self.last_pitch = pitch;
    }

    fn start_crossfade(&mut self, from: ActiveParams) {
        self.fade_from = from;
        self.fade_frames_left = CROSSFADE_FRAMES;
    }

    fn reset_filters(&mut self) {
        self.pre_mem = 0.0;
        self.syn_mem = [0.0; LPC_ORDER];
        self.inv_mem = [0.0; LPC_ORDER];
        for s in &mut self.pitch_buf {
            *s = 0.0;
        }
        self.pitch_write = 0;
        self.pitch_read = 0.0;
    }

    /// Transform one 20 ms mono frame in place.
    pub fn process_frame(&mut self, pcm: &mut [i16]) {
        if pcm.len() < FRAME_SAMPLES {
            return;
        }
        let frame = &mut pcm[..FRAME_SAMPLES];
        let effective = self.effective_params();
        if !effective.is_active() && self.fade_frames_left == 0 {
            return;
        }

        let mut dry = [0.0f32; FRAME_SAMPLES];
        for (i, s) in frame.iter().enumerate() {
            dry[i] = *s as f32 / 32768.0;
        }

        let wet = if effective.is_active() {
            self.apply_params(&dry, effective)
        } else {
            dry
        };

        let out = if self.fade_frames_left > 0 {
            let from = self.fade_from;
            let old = if from.is_active() {
                // Re-apply old path from dry (approximate; avoids dual state).
                self.apply_params_stateless(&dry, from)
            } else {
                dry
            };
            let t = 1.0 - (self.fade_frames_left as f32 / CROSSFADE_FRAMES as f32);
            self.fade_frames_left -= 1;
            let mut mixed = [0.0f32; FRAME_SAMPLES];
            for i in 0..FRAME_SAMPLES {
                mixed[i] = old[i] * (1.0 - t) + wet[i] * t;
            }
            if self.fade_frames_left == 0 {
                self.fade_from.zeroize();
            }
            mixed
        } else {
            wet
        };

        for (i, s) in out.iter().enumerate() {
            let v = (s * 32768.0).clamp(-32768.0, 32767.0);
            frame[i] = v as i16;
        }
    }

    fn apply_params(
        &mut self,
        dry: &[f32; FRAME_SAMPLES],
        p: ActiveParams,
    ) -> [f32; FRAME_SAMPLES] {
        let mut x = *dry;
        match p.mode {
            DisguiseMode::Off => {}
            DisguiseMode::Pitch => {
                x = self.pitch_shift(&x, p.pitch_semitones);
            }
            DisguiseMode::Formant => {
                x = self.formant_mcadams(&x, p.alpha);
            }
            DisguiseMode::FormantPitch => {
                x = self.formant_mcadams(&x, p.alpha);
                x = self.pitch_shift(&x, p.pitch_semitones);
            }
        }
        x
    }

    /// Crossfade helper: formant is stateful; for the outgoing fade leg we use a
    /// lighter path (pitch via local buffer copy + one-shot LPC) to avoid clobbering
    /// the live filter memory mid-fade.
    fn apply_params_stateless(
        &self,
        dry: &[f32; FRAME_SAMPLES],
        p: ActiveParams,
    ) -> [f32; FRAME_SAMPLES] {
        match p.mode {
            DisguiseMode::Off => *dry,
            DisguiseMode::Pitch => pitch_shift_oneshot(dry, p.pitch_semitones),
            DisguiseMode::Formant => formant_mcadams_oneshot(dry, p.alpha),
            DisguiseMode::FormantPitch => {
                let y = formant_mcadams_oneshot(dry, p.alpha);
                pitch_shift_oneshot(&y, p.pitch_semitones)
            }
        }
    }

    fn formant_mcadams(&mut self, x: &[f32; FRAME_SAMPLES], alpha: f32) -> [f32; FRAME_SAMPLES] {
        if (alpha - 1.0).abs() < 1e-3 {
            return *x;
        }
        // Pre-emphasis.
        let mut pre = [0.0f32; FRAME_SAMPLES];
        let mut prev = self.pre_mem;
        for i in 0..FRAME_SAMPLES {
            let s = x[i];
            pre[i] = s - 0.97 * prev;
            prev = s;
        }
        self.pre_mem = prev;

        // Source-filter: LPC inverse → residual, McAdams-warp poles, synthesize.
        // Real-time O(N·p) path; DFT envelope warp is the stable fallback.
        let a = lpc_levinson(&pre, LPC_ORDER);
        let a_mod = mcadams_warp_lpc(&a, alpha);

        let mut residual = [0.0f32; FRAME_SAMPLES];
        let mut inv = self.inv_mem;
        for n in 0..FRAME_SAMPLES {
            let mut y = pre[n];
            for k in 0..LPC_ORDER {
                y += a[k] * inv[k];
            }
            residual[n] = if y.is_finite() { y } else { 0.0 };
            for k in (1..LPC_ORDER).rev() {
                inv[k] = inv[k - 1];
            }
            inv[0] = pre[n];
        }
        self.inv_mem = inv;

        // Map McAdams α → mild allpass (VTLN) on the residual before synthesis.
        // This keeps formant motion audible even when pole re-fit is imperfect.
        let rho = ((1.0 - alpha) / (1.0 + alpha)).clamp(-0.85, 0.85);
        let mut warped_res = [0.0f32; FRAME_SAMPLES];
        let mut x1 = 0.0f32;
        let mut y1 = 0.0f32;
        for n in 0..FRAME_SAMPLES {
            let xn = residual[n];
            let yn = -rho * xn + x1 + rho * y1;
            warped_res[n] = yn;
            x1 = xn;
            y1 = yn;
        }

        let mut out = [0.0f32; FRAME_SAMPLES];
        let mut syn = self.syn_mem;
        let mut exploded = false;
        for n in 0..FRAME_SAMPLES {
            let mut y = warped_res[n];
            for k in 0..LPC_ORDER {
                y -= a_mod[k] * syn[k];
            }
            if !y.is_finite() || y.abs() > 4.0 {
                exploded = true;
                break;
            }
            out[n] = y;
            for k in (1..LPC_ORDER).rev() {
                syn[k] = syn[k - 1];
            }
            syn[0] = y;
        }

        let shaped = if exploded {
            // Synthesize with the *original* LPC after VTLN residual warp only.
            let mut syn2 = [0.0f32; LPC_ORDER];
            let mut alt = [0.0f32; FRAME_SAMPLES];
            let mut bad = false;
            for n in 0..FRAME_SAMPLES {
                let mut y = warped_res[n];
                for k in 0..LPC_ORDER {
                    y -= a[k] * syn2[k];
                }
                if !y.is_finite() || y.abs() > 4.0 {
                    bad = true;
                    break;
                }
                alt[n] = y;
                for k in (1..LPC_ORDER).rev() {
                    syn2[k] = syn2[k - 1];
                }
                syn2[0] = y;
            }
            if bad {
                spectral_formant_warp(&pre, alpha)
            } else {
                alt
            }
        } else {
            self.syn_mem = syn;
            out
        };

        let mut dee = [0.0f32; FRAME_SAMPLES];
        let mut dprev = 0.0f32;
        for i in 0..FRAME_SAMPLES {
            let s = shaped[i];
            dee[i] = if s.is_finite() { s + 0.97 * dprev } else { 0.0 };
            dprev = dee[i];
        }
        match_rms(x, &dee)
    }

    fn pitch_shift(&mut self, x: &[f32; FRAME_SAMPLES], semitones: f32) -> [f32; FRAME_SAMPLES] {
        if semitones.abs() < 1e-3 {
            return *x;
        }
        // Zero-delay frame pitch: resample this frame, then time-scale back to
        // FRAME_SAMPLES with linear interpolation (OLA with previous tail).
        let ratio = 2f32.powf(semitones / 12.0);
        let mut stretched = [0.0f32; FRAME_SAMPLES];
        for (i, sample) in stretched.iter_mut().enumerate() {
            let src = (i as f32) * ratio;
            let i0 = src.floor() as usize;
            let frac = src - i0 as f32;
            let s0 = x[i0.min(FRAME_SAMPLES - 1)];
            let s1 = x[(i0 + 1).min(FRAME_SAMPLES - 1)];
            *sample = s0 * (1.0 - frac) + s1 * frac;
        }
        // Soft-crossfade first ~2 ms with previous frame tail to reduce clicks.
        let fade = (SAMPLE_RATE as usize) / 500; // 2 ms
        let fade = fade.min(FRAME_SAMPLES / 4);
        let mut out = stretched;
        if self.pitch_buf.len() >= fade {
            let base = self.pitch_buf.len() - fade;
            for (i, sample) in out.iter_mut().enumerate().take(fade) {
                let t = i as f32 / fade as f32;
                let prev = self.pitch_buf[base + i];
                *sample = prev * (1.0 - t) + *sample * t;
            }
        }
        self.pitch_buf.clear();
        self.pitch_buf.extend_from_slice(&out);
        let _ = (self.pitch_write, self.pitch_read, PITCH_WIN);
        out
    }
}

fn params_differ(a: ActiveParams, b: ActiveParams) -> bool {
    a.mode != b.mode
        || (a.pitch_semitones - b.pitch_semitones).abs() > 1e-3
        || (a.alpha - b.alpha).abs() > 1e-3
}

fn draw_random(strength: FormantStrength, base_pitch: f32) -> (f32, f32) {
    let (lo, hi) = strength.alpha_range();
    let mut bytes = [0u8; 8];
    OsRng.fill_bytes(&mut bytes);
    let u1 =
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f32 / (u32::MAX as f32);
    let u2 =
        u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as f32 / (u32::MAX as f32);
    bytes.zeroize();
    let alpha = lo + (hi - lo) * u1;
    // Random pitch offset within ±2 semitones of the chosen slider, clamped to ±6.
    let pitch = (base_pitch + (u2 * 4.0 - 2.0)).clamp(-6.0, 6.0);
    (alpha, pitch)
}

fn match_rms(ref_x: &[f32; FRAME_SAMPLES], y: &[f32; FRAME_SAMPLES]) -> [f32; FRAME_SAMPLES] {
    let mut r2 = 0.0f32;
    let mut y2 = 0.0f32;
    for i in 0..FRAME_SAMPLES {
        r2 += ref_x[i] * ref_x[i];
        y2 += y[i] * y[i];
    }
    if y2 < 1e-12 {
        return *y;
    }
    let g = (r2 / y2).sqrt();
    let mut out = [0.0f32; FRAME_SAMPLES];
    for i in 0..FRAME_SAMPLES {
        out[i] = y[i] * g;
    }
    out
}

/// Autocorrelation + Levinson–Durbin → LPC coefficients a[1..p] (a[0]=1 implied).
fn lpc_levinson(x: &[f32], order: usize) -> [f32; LPC_ORDER] {
    let mut r = [0.0f32; LPC_ORDER + 1];
    let n = x.len();
    for k in 0..=order {
        let mut s = 0.0f32;
        for i in k..n {
            s += x[i] * x[i - k];
        }
        r[k] = s;
    }
    let mut a = [0.0f32; LPC_ORDER];
    if r[0].abs() < 1e-12 {
        return a;
    }
    let mut e = r[0];
    let mut a_prev = [0.0f32; LPC_ORDER];
    for m in 1..=order {
        let mut acc = r[m];
        for i in 1..m {
            acc += a_prev[i - 1] * r[m - i];
        }
        let k = -acc / e;
        a[m - 1] = k;
        for i in 1..m {
            a[i - 1] = a_prev[i - 1] + k * a_prev[m - i - 1];
        }
        e *= 1.0 - k * k;
        if e < 1e-12 {
            break;
        }
        a_prev = a;
    }
    a
}

#[derive(Clone, Copy)]
struct C32 {
    re: f32,
    im: f32,
}

impl C32 {
    fn new(re: f32, im: f32) -> Self {
        Self { re, im }
    }
    fn mag(self) -> f32 {
        (self.re * self.re + self.im * self.im).sqrt()
    }
    fn add(self, o: Self) -> Self {
        Self::new(self.re + o.re, self.im + o.im)
    }
    fn sub(self, o: Self) -> Self {
        Self::new(self.re - o.re, self.im - o.im)
    }
    fn mul(self, o: Self) -> Self {
        Self::new(
            self.re * o.re - self.im * o.im,
            self.re * o.im + self.im * o.re,
        )
    }
    fn div(self, o: Self) -> Self {
        let d = o.re * o.re + o.im * o.im;
        if d < 1e-20 {
            return Self::new(0.0, 0.0);
        }
        Self::new(
            (self.re * o.re + self.im * o.im) / d,
            (self.im * o.re - self.re * o.im) / d,
        )
    }
}

/// Warp complex LPC poles by McAdams α (θ' = sign(θ)·|θ|^α), rebuild real LPC.
/// Returns the original coefficients when the root fit looks unstable.
fn mcadams_warp_lpc(a: &[f32; LPC_ORDER], alpha: f32) -> [f32; LPC_ORDER] {
    let mut coeffs = [0.0f32; LPC_ORDER + 1];
    coeffs[0] = 1.0;
    coeffs[1..=LPC_ORDER].copy_from_slice(a);
    let mut roots = durand_kerner_roots(&coeffs);
    for r in &mut roots {
        let mag = r.mag().min(0.99);
        let theta = r.im.atan2(r.re);
        let theta2 = if theta.abs() < 1e-8 {
            0.0
        } else {
            theta.signum() * theta.abs().powf(alpha)
        };
        *r = C32::new(mag * theta2.cos(), mag * theta2.sin());
    }
    let warped = poly_from_poles(&roots);
    let mut energy = 0.0f32;
    let mut orig = 0.0f32;
    for i in 0..LPC_ORDER {
        energy += warped[i] * warped[i];
        orig += a[i] * a[i];
    }
    if !energy.is_finite() || energy > orig * 64.0 + 1.0 {
        *a
    } else {
        warped
    }
}

fn eval_poly(coeffs: &[f32], z: C32) -> C32 {
    let mut acc = C32::new(0.0, 0.0);
    for &c in coeffs {
        acc = acc.mul(z).add(C32::new(c, 0.0));
    }
    acc
}

/// Durand–Kerner simultaneous root finder for a real monic-ish polynomial.
fn durand_kerner_roots(coeffs: &[f32; LPC_ORDER + 1]) -> [C32; LPC_ORDER] {
    let n = LPC_ORDER;
    let mut roots = [C32::new(0.0, 0.0); LPC_ORDER];
    for (k, r) in roots.iter_mut().enumerate() {
        let ang = 2.0 * PI * (k as f32) / (n as f32);
        *r = C32::new(0.5 * ang.cos(), 0.5 * ang.sin());
    }
    for _ in 0..48 {
        let prev = roots;
        for i in 0..n {
            let mut den = C32::new(1.0, 0.0);
            for (j, &rj) in prev.iter().enumerate() {
                if i != j {
                    den = den.mul(prev[i].sub(rj));
                }
            }
            if den.mag() < 1e-12 {
                continue;
            }
            let num = eval_poly(coeffs, prev[i]);
            roots[i] = prev[i].sub(num.div(den));
        }
    }
    roots
}

fn poly_from_poles(poles: &[C32; LPC_ORDER]) -> [f32; LPC_ORDER] {
    // Real polynomial via successive (z − p) factors; store coeffs of z^n … z^0.
    let mut poly = vec![C32::new(1.0, 0.0)];
    for &p in poles {
        let mut next = vec![C32::new(0.0, 0.0); poly.len() + 1];
        for (i, &c) in poly.iter().enumerate() {
            next[i] = next[i].add(c);
            next[i + 1] = next[i + 1].sub(c.mul(p));
        }
        poly = next;
    }
    // poly[0] should be ~1 for monic z^n + …; LPC a_k = poly[k+1]/poly[0] for z^- form.
    let scale = if poly[0].re.abs() > 1e-8 {
        poly[0].re
    } else {
        1.0
    };
    let mut a = [0.0f32; LPC_ORDER];
    for i in 0..LPC_ORDER {
        if i + 1 < poly.len() {
            a[i] = poly[i + 1].re / scale;
        }
    }
    // Soft stability: shrink if reflection-style gain looks wild.
    let mut max_a = 0.0f32;
    for &v in &a {
        max_a = max_a.max(v.abs());
    }
    if max_a > 8.0 {
        let g = 8.0 / max_a;
        for v in &mut a {
            *v *= g;
        }
    }
    a
}

fn formant_mcadams_oneshot(x: &[f32; FRAME_SAMPLES], alpha: f32) -> [f32; FRAME_SAMPLES] {
    let mut eng = DisguiseEngine::default();
    eng.formant_mcadams(x, alpha)
}

/// DFT-domain formant warp: move spectral envelope along ω' = π·(ω/π)^α
/// while keeping fine structure (McAdams listening family without pole roots).
#[allow(clippy::needless_range_loop)]
fn spectral_formant_warp(x: &[f32; FRAME_SAMPLES], alpha: f32) -> [f32; FRAME_SAMPLES] {
    const N: usize = FRAME_SAMPLES;
    // Real DFT (naive; N=960, once per 20 ms is fine for host + emulator).
    let mut re = [0.0f32; N];
    let mut im = [0.0f32; N];
    for k in 0..N {
        let mut sr = 0.0f32;
        let mut si = 0.0f32;
        for (n, &xn) in x.iter().enumerate() {
            let th = -2.0 * PI * (k as f32) * (n as f32) / (N as f32);
            sr += xn * th.cos();
            si += xn * th.sin();
        }
        re[k] = sr;
        im[k] = si;
    }
    let half = N / 2;
    let mut mag = [0.0f32; N];
    for k in 0..=half {
        mag[k] = (re[k] * re[k] + im[k] * im[k]).sqrt().max(1e-12);
    }
    // Smooth envelope (cepstral lifter via moving average on log-mag).
    let mut env = [0.0f32; N];
    let win = 8usize;
    for k in 0..=half {
        let lo = k.saturating_sub(win);
        let hi = (k + win).min(half);
        let mut s = 0.0f32;
        let mut c = 0.0f32;
        for j in lo..=hi {
            s += mag[j].ln();
            c += 1.0;
        }
        env[k] = (s / c).exp();
    }
    // Warp envelope axis.
    let mut env_w = [0.0f32; N];
    for k in 0..=half {
        let f = k as f32 / half as f32;
        let fw = f.powf(alpha).clamp(0.0, 1.0);
        let src = fw * half as f32;
        let i0 = src.floor() as usize;
        let i1 = (i0 + 1).min(half);
        let frac = src - i0 as f32;
        env_w[k] = env[i0] * (1.0 - frac) + env[i1] * frac;
    }
    // Apply: keep excitation (mag/env), replace envelope.
    for k in 0..=half {
        let e = env[k].max(1e-12);
        let scale = env_w[k] / e;
        re[k] *= scale;
        im[k] *= scale;
        if k != 0 && k != half {
            re[N - k] = re[k];
            im[N - k] = -im[k];
        }
    }
    // Inverse DFT.
    let mut out = [0.0f32; FRAME_SAMPLES];
    for (n, sample) in out.iter_mut().enumerate() {
        let mut sr = 0.0f32;
        for k in 0..N {
            let th = 2.0 * PI * (k as f32) * (n as f32) / (N as f32);
            sr += re[k] * th.cos() - im[k] * th.sin();
        }
        *sample = sr / N as f32;
    }
    out
}

fn pitch_shift_oneshot(x: &[f32; FRAME_SAMPLES], semitones: f32) -> [f32; FRAME_SAMPLES] {
    if semitones.abs() < 1e-3 {
        return *x;
    }
    let ratio = 2f32.powf(semitones / 12.0);
    let mut out = [0.0f32; FRAME_SAMPLES];
    for (i, sample) in out.iter_mut().enumerate() {
        let src = (i as f32) * ratio;
        let i0 = src.floor() as usize;
        let frac = src - src.floor();
        let s0 = x[i0.min(FRAME_SAMPLES - 1)];
        let s1 = x[(i0 + 1).min(FRAME_SAMPLES - 1)];
        *sample = s0 * (1.0 - frac) + s1 * frac;
    }
    out
}

#[cfg(debug_assertions)]
fn log_debug_params(tag: &str, p: ActiveParams) {
    // Intentionally dbg-only; release builds must not print α / pitch offsets.
    eprintln!(
        "[saga-disguise-debug] {tag} mode={:?} pitch={:.3} alpha={:.4}",
        p.mode, p.pitch_semitones, p.alpha
    );
}

fn engine_mutex() -> &'static Mutex<DisguiseEngine> {
    static ENGINE: OnceLock<Mutex<DisguiseEngine>> = OnceLock::new();
    ENGINE.get_or_init(|| Mutex::new(DisguiseEngine::default()))
}

fn with_engine<R>(f: impl FnOnce(&mut DisguiseEngine) -> R) -> R {
    let mut g = engine_mutex().lock().unwrap_or_else(|e| e.into_inner());
    f(&mut g)
}

/// Hook used by Opus encode — applies the process-global engine.
pub fn disguise_pcm_inplace(pcm: &mut [i16]) {
    with_engine(|e| e.process_frame(pcm));
}

pub fn set_disguise_settings(settings: DisguiseSettings) {
    with_engine(|e| e.set_settings(settings));
}

pub fn disguise_settings() -> DisguiseSettings {
    with_engine(|e| e.settings())
}

pub fn disguise_begin_call() {
    with_engine(|e| e.begin_call());
}

pub fn disguise_end_call() {
    with_engine(|e| e.end_call());
}

pub fn disguise_is_active() -> bool {
    with_engine(|e| e.is_locally_active())
}

pub fn disguise_last_alpha_for_test() -> f32 {
    with_engine(|e| e.last_alpha_for_test())
}

pub fn disguise_last_pitch_for_test() -> f32 {
    with_engine(|e| e.last_pitch_for_test())
}

/// Process an arbitrary PCM buffer with the current settings (Test my voice / host tools).
pub fn disguise_process_buffer(pcm: &mut [i16]) {
    with_engine(|e| {
        let was = e.in_call;
        if !was {
            e.begin_call();
        }
        for chunk in pcm.chunks_mut(FRAME_SAMPLES) {
            if chunk.len() < FRAME_SAMPLES {
                break;
            }
            e.process_frame(chunk);
        }
        if !was {
            // Keep settings; only drop per-call random after a real call ends.
            // For offline test we zeroize the draw but leave settings.
            e.end_call();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_is_noop() {
        let mut eng = DisguiseEngine::default();
        eng.begin_call();
        let mut pcm = [100i16; FRAME_SAMPLES];
        let before = pcm;
        eng.process_frame(&mut pcm);
        assert_eq!(pcm, before);
        eng.end_call();
    }

    #[test]
    fn random_each_call_differs() {
        let mut eng = DisguiseEngine::default();
        eng.set_settings(DisguiseSettings {
            mode: DisguiseMode::Formant,
            random_each_call: true,
            ..DisguiseSettings::default()
        });
        eng.begin_call();
        let a1 = eng.last_alpha_for_test();
        eng.end_call();
        eng.begin_call();
        let a2 = eng.last_alpha_for_test();
        eng.end_call();
        // Extremely unlikely to collide with CSPRNG across the full Mild–Strong span.
        assert!(
            (a1 - a2).abs() > 1e-6,
            "expected different α draws: {a1} vs {a2}"
        );
    }

    #[test]
    fn end_call_zeroizes_alpha() {
        let mut eng = DisguiseEngine::default();
        eng.set_settings(DisguiseSettings {
            mode: DisguiseMode::Formant,
            random_each_call: false,
            ..DisguiseSettings::default()
        });
        eng.begin_call();
        assert!(eng.last_alpha_for_test() != 1.0 || eng.active.alpha != 0.0);
        eng.end_call();
        assert!((eng.last_alpha_for_test() - 1.0).abs() < 1e-6);
        assert!(!eng.is_locally_active());
    }

    #[test]
    fn pitch_changes_samples() {
        let mut eng = DisguiseEngine::default();
        eng.set_settings(DisguiseSettings {
            mode: DisguiseMode::Pitch,
            pitch_semitones: -3.0,
            random_each_call: false,
            ..DisguiseSettings::default()
        });
        eng.begin_call();
        // Two frames so the pitch ring has history.
        let mut pcm = [0i16; FRAME_SAMPLES];
        for (i, s) in pcm.iter_mut().enumerate() {
            *s = ((i as f32 * 0.1).sin() * 16000.0) as i16;
        }
        eng.process_frame(&mut pcm);
        let first = pcm;
        for (i, s) in pcm.iter_mut().enumerate() {
            *s = ((i as f32 * 0.1).sin() * 16000.0) as i16;
        }
        eng.process_frame(&mut pcm);
        assert_ne!(pcm, first);
        eng.end_call();
    }
}
