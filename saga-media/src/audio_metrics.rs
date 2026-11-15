//! SNR / correlation helpers for host tests.

/// Pearson correlation between two equal-length signals (best lag search).
pub fn best_correlation(ref_pcm: &[i16], deg_pcm: &[i16], max_lag: usize) -> f64 {
    if ref_pcm.is_empty() || deg_pcm.is_empty() {
        return 0.0;
    }
    let n = ref_pcm.len().min(deg_pcm.len());
    let mut best = f64::NEG_INFINITY;
    let max_lag = max_lag.min(n / 4);
    for lag in 0..=max_lag {
        let a = &ref_pcm[..n - lag];
        let b = &deg_pcm[lag..n];
        best = best.max(pearson(a, b));
        if lag > 0 {
            let a2 = &ref_pcm[lag..n];
            let b2 = &deg_pcm[..n - lag];
            best = best.max(pearson(a2, b2));
        }
    }
    best
}

fn pearson(a: &[i16], b: &[i16]) -> f64 {
    let n = a.len().min(b.len()) as f64;
    if n < 2.0 {
        return 0.0;
    }
    let mut sum_a = 0.0;
    let mut sum_b = 0.0;
    for i in 0..a.len().min(b.len()) {
        sum_a += a[i] as f64;
        sum_b += b[i] as f64;
    }
    let mean_a = sum_a / n;
    let mean_b = sum_b / n;
    let mut num = 0.0;
    let mut da = 0.0;
    let mut db = 0.0;
    for i in 0..a.len().min(b.len()) {
        let x = a[i] as f64 - mean_a;
        let y = b[i] as f64 - mean_b;
        num += x * y;
        da += x * x;
        db += y * y;
    }
    let den = (da * db).sqrt();
    if den < 1e-12 {
        return 0.0;
    }
    num / den
}

/// Scale-invariant SNR (dB) after aligning with best correlation lag.
///
/// Opus may change absolute gain; SI-SNR matches the degraded signal's
/// amplitude to the reference before measuring error (standard for codecs).
pub fn snr_db(ref_pcm: &[i16], deg_pcm: &[i16]) -> f64 {
    let n = ref_pcm.len().min(deg_pcm.len());
    if n == 0 {
        return 0.0;
    }
    let corr_lag = {
        let mut best_lag = 0usize;
        let mut best = f64::NEG_INFINITY;
        let max_lag = (n / 10).min(4800);
        for lag in 0..=max_lag {
            let c = pearson(&ref_pcm[..n - lag], &deg_pcm[lag..n]);
            if c > best {
                best = c;
                best_lag = lag;
            }
        }
        best_lag
    };
    let a = &ref_pcm[..n - corr_lag];
    let b = &deg_pcm[corr_lag..n];
    let mut dot = 0.0;
    let mut bb = 0.0;
    let mut aa = 0.0;
    for i in 0..a.len() {
        let s = a[i] as f64;
        let d = b[i] as f64;
        dot += s * d;
        bb += d * d;
        aa += s * s;
    }
    if bb < 1e-9 || aa < 1e-9 {
        return 0.0;
    }
    let scale = dot / bb;
    let mut noise = 0.0;
    for i in 0..a.len() {
        let s = a[i] as f64;
        let d = scale * (b[i] as f64);
        let e = s - d;
        noise += e * e;
    }
    if noise < 1e-9 {
        return 80.0;
    }
    10.0 * (aa / noise).log10()
}
