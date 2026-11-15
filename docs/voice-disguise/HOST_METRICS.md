# Voice disguise host metrics

| Mode | Wall (full clip ms) | Per-frame ms | α |
|------|---------------------:|-------------:|--:|
| pitch_m3 | 1.1 | 0.005 | 0.8200 |
| formant_mild | 30.9 | 0.133 | 0.9200 |
| formant_medium | 35.0 | 0.136 | 0.8200 |
| formant_strong | 34.3 | 0.147 | 0.7000 |
| formant_pitch_medium | 34.0 | 0.143 | 0.8200 |

Random-each-call α draws: 1.1719 vs 0.9067 (distinct).

DSP is pure Rust (Levinson–Durbin LPC + McAdams-style envelope warp + pitch resample). No GPL/commercial DSP dependency. Compatible with AGPL-3.0 and F-Droid.
