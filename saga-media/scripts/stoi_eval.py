#!/usr/bin/env python3
"""Local STOI eval for saga-media (not CI). Speech mode; resample to 16 kHz.

Usage (from saga-media/):

  python3 scripts/stoi_eval.py \\
    --ref fixtures/speech_clip_48k.wav \\
    --deg /tmp/saga-deg-clean.wav

  # Loss conditions: write degraded WAVs from the Rust harness, then:
  python3 scripts/stoi_eval.py --ref fixtures/speech_clip_48k.wav \\
    --deg /tmp/saga-deg-clean.wav --deg /tmp/saga-deg-5pct.wav \\
    --deg /tmp/saga-deg-15pct.wav --labels clean,5pct,15pct

Requires: pip install pystoi soundfile numpy
"""

from __future__ import annotations

import argparse
import sys

import numpy as np
import soundfile as sf
from pystoi import stoi


def load_mono_16k(path: str) -> tuple[np.ndarray, int]:
    audio, sr = sf.read(path, always_2d=False)
    if audio.ndim > 1:
        audio = audio.mean(axis=1)
    audio = audio.astype(np.float64)
    if sr != 16000:
        # Linear resample (adequate for STOI; avoid extra deps).
        n_out = int(round(len(audio) * 16000 / sr))
        x = np.linspace(0.0, 1.0, len(audio), endpoint=False)
        xi = np.linspace(0.0, 1.0, n_out, endpoint=False)
        audio = np.interp(xi, x, audio)
        sr = 16000
    return audio, sr


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--ref", required=True, help="reference WAV (48 kHz OK)")
    ap.add_argument(
        "--deg",
        action="append",
        required=True,
        help="degraded WAV (repeatable)",
    )
    ap.add_argument(
        "--labels",
        default="",
        help="comma-separated labels matching --deg order",
    )
    args = ap.parse_args()
    ref, sr = load_mono_16k(args.ref)
    labels = [s.strip() for s in args.labels.split(",") if s.strip()]
    while len(labels) < len(args.deg):
        labels.append(f"deg{len(labels)}")
    print(f"ref={args.ref} sr={sr} samples={len(ref)}")
    for path, label in zip(args.deg, labels):
        deg, _ = load_mono_16k(path)
        n = min(len(ref), len(deg))
        if n < sr:  # < 1 s
            print(f"{label}: ERROR too short ({n} samples)", file=sys.stderr)
            return 1
        score = stoi(ref[:n], deg[:n], sr, extended=False)
        print(f"{label}: STOI={score:.4f}  (n={n})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
