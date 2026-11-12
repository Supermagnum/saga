#!/usr/bin/env python3
"""Score raw s16le mono 48 kHz dump vs reference WAV: corr, SI-SNR, STOI.

STOI: both signals are lag-aligned (same lag as corr), trimmed to equal
length, then resampled from the source rate (48 kHz for fixture + dumps)
to 16 kHz before pystoi.

Usage:
  python3 testing/e2e/pcm_score.py \\
    --ref saga-media/fixtures/speech_clip_48k.wav \\
    --deg-raw /tmp/bob-play.pcm \\
    --label alice_to_bob

  # Sanity: STOI(ref, ref) and optional host pair:
  python3 testing/e2e/pcm_score.py --ref ... --self-stoi
  python3 testing/e2e/pcm_score.py --ref ... --deg-wav saga-media/target/stoi/deg-clean.wav \\
    --label host_clean
"""

from __future__ import annotations

import argparse
import sys
import wave
from pathlib import Path

import numpy as np


def load_wav_mono_i16(path: str) -> tuple[np.ndarray, int]:
    with wave.open(path, "rb") as w:
        assert w.getnchannels() == 1, path
        assert w.getsampwidth() == 2, path
        sr = w.getframerate()
        raw = w.readframes(w.getnframes())
    samples = np.frombuffer(raw, dtype="<i2").astype(np.float64)
    return samples, sr


def load_raw_s16le(path: str) -> np.ndarray:
    data = Path(path).read_bytes()
    n = len(data) // 2
    return np.frombuffer(data[: n * 2], dtype="<i2").astype(np.float64)


def best_correlation(
    ref: np.ndarray, deg: np.ndarray, max_lag: int, step: int = 160
) -> tuple[float, int]:
    n = min(len(ref), len(deg))
    ref = ref[:n]
    deg = deg[:n]
    best = -1.0
    best_lag = 0
    for lag in range(-max_lag, max_lag + 1, step):
        if lag >= 0:
            a = ref[lag:]
            b = deg[: len(a)]
        else:
            b = deg[-lag:]
            a = ref[: len(b)]
        if len(a) < 1000:
            continue
        a = a - a.mean()
        b = b - b.mean()
        denom = np.sqrt((a * a).sum() * (b * b).sum())
        if denom <= 0:
            continue
        c = float((a * b).sum() / denom)
        if c > best:
            best = c
            best_lag = lag
    # Fine search ±2 steps around best.
    fine_lo = best_lag - step
    fine_hi = best_lag + step
    for lag in range(fine_lo, fine_hi + 1):
        if lag < -max_lag or lag > max_lag:
            continue
        if lag >= 0:
            a = ref[lag:]
            b = deg[: len(a)]
        else:
            b = deg[-lag:]
            a = ref[: len(b)]
        if len(a) < 1000:
            continue
        a = a - a.mean()
        b = b - b.mean()
        denom = np.sqrt((a * a).sum() * (b * b).sum())
        if denom <= 0:
            continue
        c = float((a * b).sum() / denom)
        if c > best:
            best = c
            best_lag = lag
    return best, best_lag


def align_pair(ref: np.ndarray, deg: np.ndarray, lag: int) -> tuple[np.ndarray, np.ndarray]:
    """Apply the same delay compensation as corr / SI-SNR; trim equal length."""
    if lag >= 0:
        a = ref[lag:]
        b = deg[: len(a)]
    else:
        b = deg[-lag:]
        a = ref[: len(b)]
    n = min(len(a), len(b))
    return a[:n], b[:n]


def si_snr_db(ref: np.ndarray, deg: np.ndarray, lag: int) -> float:
    a, b = align_pair(ref, deg, lag)
    a = a - a.mean()
    b = b - b.mean()
    proj = (np.dot(b, a) / (np.dot(a, a) + 1e-12)) * a
    noise = b - proj
    num = np.dot(proj, proj) + 1e-12
    den = np.dot(noise, noise) + 1e-12
    return float(10.0 * np.log10(num / den))


def resample_linear(x: np.ndarray, sr_in: int, sr_out: int) -> np.ndarray:
    """Linear resample float PCM in [-1, 1] (or raw float scale) to sr_out."""
    if sr_in == sr_out:
        return x.copy()
    n_out = int(round(len(x) * sr_out / sr_in))
    if n_out < 1:
        return np.zeros(0, dtype=np.float64)
    t = np.linspace(0.0, 1.0, len(x), endpoint=False)
    ti = np.linspace(0.0, 1.0, n_out, endpoint=False)
    return np.interp(ti, t, x)


def compute_stoi(
    ref: np.ndarray, deg: np.ndarray, sr_in: int, lag: int
) -> tuple[float | None, int, int]:
    """Lag-align at sr_in, trim, resample BOTH to 16 kHz, run pystoi.

    Returns (score, sr_in, sr_stoi). Score is None if pystoi missing/fails.
    """
    a, b = align_pair(ref, deg, lag)
    # Float in [-1, 1] for STOI.
    a_f = a / 32768.0
    b_f = b / 32768.0
    sr_stoi = 16000
    a16 = resample_linear(a_f, sr_in, sr_stoi)
    b16 = resample_linear(b_f, sr_in, sr_stoi)
    m = min(len(a16), len(b16))
    if m < sr_stoi:  # < 1 s
        return None, sr_in, sr_stoi
    try:
        from pystoi import stoi

        return float(stoi(a16[:m], b16[:m], sr_stoi, extended=False)), sr_in, sr_stoi
    except Exception as e:
        print(f"STOI unavailable ({e})", file=sys.stderr)
        return None, sr_in, sr_stoi


def to_wav(path: Path, samples: np.ndarray, sr: int = 48000) -> None:
    clipped = np.clip(samples, -32768, 32767).astype("<i2")
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(sr)
        w.writeframes(clipped.tobytes())


def prepare_window(
    ref: np.ndarray, deg: np.ndarray, edge_frames: int, max_lag_frames: int
) -> tuple[np.ndarray, np.ndarray, int, int]:
    """Onset trim on deg, equal-length window, edge trim for lag search."""
    frame = 960
    env = [
        float(np.sqrt(np.mean(deg[i : i + frame] ** 2)))
        for i in range(0, len(deg) - frame, frame)
    ]
    peak = max(env) if env else 0.0
    onset_frame = 0
    if peak > 50.0:
        thr = 0.08 * peak
        for i, e in enumerate(env):
            if e >= thr:
                onset_frame = i
                break
    onset = onset_frame * frame
    deg = deg[onset:]

    max_n = frame * 150
    n = min(len(ref), len(deg), max_n)
    n -= n % frame
    ref = ref[:n]
    deg = deg[:n]

    edge = edge_frames * frame
    if n > edge * 2:
        ref_s = ref[edge : n - edge]
        deg_s = deg[edge : n - edge]
    else:
        ref_s, deg_s = ref, deg
    return ref_s, deg_s, onset, n


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--ref", required=True, help="LibriSpeech fixture WAV (48 kHz mono s16)")
    ap.add_argument("--deg-raw", default="", help="raw s16le mono 48 kHz dump")
    ap.add_argument("--deg-wav", default="", help="degraded WAV (same rate as --ref preferred)")
    ap.add_argument("--label", default="deg")
    ap.add_argument("--out-wav", default="", help="optional WAV write of degraded")
    ap.add_argument("--max-lag-frames", type=int, default=8, help="lag search in 20 ms frames")
    ap.add_argument("--edge-frames", type=int, default=5)
    ap.add_argument(
        "--self-stoi",
        action="store_true",
        help="print STOI(ref,ref) after 48→16 kHz resample (expect ≈1.0)",
    )
    args = ap.parse_args()

    ref, sr = load_wav_mono_i16(args.ref)
    if sr != 48000:
        print(f"ERROR: ref sr={sr} need 48000 (LibriSpeech fixture)", file=sys.stderr)
        return 1

    if args.self_stoi:
        score, sr_in, sr_stoi = compute_stoi(ref, ref, sr, lag=0)
        stoi_s = f"{score:.4f}" if score is not None else "n/a"
        print(
            f"self_stoi: stoi={stoi_s} sr_in={sr_in} sr_stoi={sr_stoi} "
            f"ref={args.ref} samples={len(ref)}"
        )
        if not args.deg_raw and not args.deg_wav:
            return 0

    if args.deg_wav:
        deg, deg_sr = load_wav_mono_i16(args.deg_wav)
        if deg_sr != sr:
            # Resample deg to ref rate before lag search.
            deg = resample_linear(deg, deg_sr, sr)
        deg_path = args.deg_wav
    elif args.deg_raw:
        deg = load_raw_s16le(args.deg_raw)
        deg_path = args.deg_raw
    else:
        print("ERROR: need --deg-raw or --deg-wav (unless only --self-stoi)", file=sys.stderr)
        return 1

    if len(deg) < 48000:
        print(f"ERROR: deg too short ({len(deg)} samples)", file=sys.stderr)
        return 1

    frame = 960
    ref_s, deg_s, onset, n = prepare_window(ref, deg, args.edge_frames, args.max_lag_frames)
    max_lag = max(args.max_lag_frames * frame, 50 * frame)  # ±1 s lag search
    corr, lag = best_correlation(ref_s, deg_s, max_lag)
    snr = si_snr_db(ref_s, deg_s, lag)
    stoi_score, sr_in, sr_stoi = compute_stoi(ref_s, deg_s, sr, lag)
    lag_ms = 1000.0 * lag / sr

    out_wav = args.out_wav
    if not out_wav:
        if args.deg_raw:
            out_wav = str(Path(args.deg_raw).with_suffix(".wav"))
        else:
            out_wav = str(Path(deg_path).with_name(Path(deg_path).stem + ".scored.wav"))
    to_wav(Path(out_wav), deg_s if args.deg_raw else deg[:n], 48000)

    stoi_s = f"{stoi_score:.4f}" if stoi_score is not None else "n/a"
    print(
        f"{args.label}: corr={corr:.3f} si_snr_db={snr:.1f} stoi={stoi_s} "
        f"lag={lag} lag_ms={lag_ms:.1f} onset={onset} samples={n} "
        f"sr_in={sr_in} sr_stoi={sr_stoi} ref={args.ref} out={out_wav}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
