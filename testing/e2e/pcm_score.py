#!/usr/bin/env python3
"""Score raw s16le mono 48 kHz dump vs reference WAV: corr, SI-SNR, STOI.

Usage:
  python3 testing/e2e/pcm_score.py \\
    --ref saga-media/fixtures/speech_clip_48k.wav \\
    --deg-raw /tmp/bob-play.pcm \\
    --label alice_to_bob
"""

from __future__ import annotations

import argparse
import struct
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


def si_snr_db(ref: np.ndarray, deg: np.ndarray, lag: int) -> float:
    if lag >= 0:
        a = ref[lag:]
        b = deg[: len(a)]
    else:
        b = deg[-lag:]
        a = ref[: len(b)]
    n = min(len(a), len(b))
    a = a[:n]
    b = b[:n]
    a = a - a.mean()
    b = b - b.mean()
    proj = (np.dot(b, a) / (np.dot(a, a) + 1e-12)) * a
    noise = b - proj
    num = np.dot(proj, proj) + 1e-12
    den = np.dot(noise, noise) + 1e-12
    return float(10.0 * np.log10(num / den))


def to_wav(path: Path, samples: np.ndarray, sr: int = 48000) -> None:
    clipped = np.clip(samples, -32768, 32767).astype("<i2")
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(sr)
        w.writeframes(clipped.tobytes())


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--ref", required=True)
    ap.add_argument("--deg-raw", required=True, help="raw s16le mono 48 kHz")
    ap.add_argument("--label", default="deg")
    ap.add_argument("--out-wav", default="", help="optional WAV write of degraded")
    ap.add_argument("--max-lag-frames", type=int, default=8, help="lag search in 20 ms frames")
    ap.add_argument("--edge-frames", type=int, default=5)
    args = ap.parse_args()

    ref, sr = load_wav_mono_i16(args.ref)
    if sr != 48000:
        print(f"ERROR: ref sr={sr} need 48000", file=sys.stderr)
        return 1
    deg = load_raw_s16le(args.deg_raw)
    if len(deg) < 48000:
        print(f"ERROR: deg too short ({len(deg)} samples)", file=sys.stderr)
        return 1

    frame = 960
    # Skip leading near-silence on dump (JB underrun before first packet).
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

    # Align window: up to ~3 s of speech after onset.
    max_n = frame * 150
    n = min(len(ref), len(deg), max_n)
    n -= n % frame
    ref = ref[:n]
    deg = deg[:n]

    edge = args.edge_frames * frame
    max_lag = max(args.max_lag_frames * frame, 50 * frame)  # ±1 s lag search
    if n > edge * 2:
        ref_s = ref[edge : n - edge]
        deg_s = deg[edge : n - edge]
    else:
        ref_s, deg_s = ref, deg

    corr, lag = best_correlation(ref_s, deg_s, max_lag)
    snr = si_snr_db(ref_s, deg_s, lag)

    out_wav = args.out_wav or str(Path(args.deg_raw).with_suffix(".wav"))
    to_wav(Path(out_wav), deg, 48000)

    stoi_score = None
    try:
        from pystoi import stoi

        # Resample to 16 kHz linearly for STOI.
        def rs16(x: np.ndarray) -> np.ndarray:
            n_out = int(round(len(x) * 16000 / 48000))
            t = np.linspace(0.0, 1.0, len(x), endpoint=False)
            ti = np.linspace(0.0, 1.0, n_out, endpoint=False)
            return np.interp(ti, t, x / 32768.0)

        r16 = rs16(ref_s)
        d16 = rs16(deg_s)
        m = min(len(r16), len(d16))
        stoi_score = float(stoi(r16[:m], d16[:m], 16000, extended=False))
    except Exception as e:
        print(f"STOI unavailable ({e})", file=sys.stderr)

    stoi_s = f"{stoi_score:.4f}" if stoi_score is not None else "n/a"
    print(
        f"{args.label}: corr={corr:.3f} si_snr_db={snr:.1f} stoi={stoi_s} "
        f"lag={lag} onset={onset} samples={n} out={out_wav}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
