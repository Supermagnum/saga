# Media fixtures attribution

## `speech_clip_48k.wav`

Short mono 48 kHz English utterance synthesized with **espeak-ng** for offline
CI round-trip tests (not a human recording). Regenerate with:

```text
espeak-ng -v en-us -s 140 -w /tmp/speech_raw.wav \
  "The birch canoe slid on the smooth planks. Glue the sheet to the dark blue background."
ffmpeg -y -i /tmp/speech_raw.wav -ac 1 -ar 48000 -sample_fmt s16 speech_clip_48k.wav
```

For formal listening / ViSQOL against real speech, substitute a short utterance
from **LibriSpeech** (Panayotov et al., CC BY 4.0):

- Dataset: https://www.openslr.org/12/
- Licence: Creative Commons Attribution 4.0 International
- Cite: V. Panayotov, G. Chen, D. Povey, S. Khudanpur, "Librispeech: an ASR
  corpus based on public domain audio books", ICASSP 2015.

Do not replace this fixture with copyrighted material.
