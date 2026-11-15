# Voice disguise (§16) — listening fixtures

Before/after WAVs from the LibriSpeech host fixture
(`saga-media/fixtures/speech_clip_48k.wav`), produced by
`cargo test -p saga-media --test voice_disguise`.

| File | Mode |
|------|------|
| `original.wav` | Unprocessed reference |
| `pitch_m3.wav` | Pitch −3 semitones |
| `formant_mild.wav` | McAdams formant (Mild) |
| `formant_medium.wav` | McAdams formant (Medium) |
| `formant_strong.wav` | McAdams formant (Strong) |
| `formant_pitch_medium.wav` | Formant (Medium) + pitch −3 |

## Licences

DSP is implemented in-tree in `saga-media/src/disguise.rs` (Levinson–Durbin
LPC, McAdams-style spectral-envelope warp, pitch resample). No third-party
DSP library is linked. Compatible with AGPL-3.0-or-later and F-Droid.

## Metrics

See `HOST_METRICS.md` for per-frame latency (must stay ≤ ~20 ms) and random
α draws. STOI vs original is recorded in `TEST_RESULTS.md`.
