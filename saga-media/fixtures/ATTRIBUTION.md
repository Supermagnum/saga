# Media fixtures attribution

## `speech_clip_48k.wav`

Real recorded utterance from **LibriSpeech** (OpenSLR resource 31 /
`dev-clean-2`), upsampled to mono 48 kHz PCM for Saga host tests.

| Field | Value |
|-------|--------|
| Utterance id | `5694-64038-0005` |
| Source path | `LibriSpeech/dev-clean-2/5694/64038/5694-64038-0005.flac` |
| Transcript | `HE WALKED UP AND SAYS HELLO BOYS WHAT IS IT BOSS` |
| Duration | ~4.7 s (16 kHz original; CI uses 48 kHz resample) |
| Dataset | https://www.openslr.org/12/ (parent); subset https://www.openslr.org/31/ |
| Licence | Creative Commons Attribution 4.0 International (CC BY 4.0) |
| Cite | V. Panayotov, G. Chen, D. Povey, S. Khudanpur, "Librispeech: an ASR corpus based on public domain audio books", ICASSP 2015 |

Do not replace this fixture with copyrighted material.

## espeak fixture (not shipped)

An earlier espeak-ng TTS clip was used only during development. It is **not**
kept in-tree: LibriSpeech is the CI speech gate. espeak remains useful only as
a zero-download smoke generator when OpenSLR is unreachable — regenerate
locally if needed; do not commit it.
