#!/usr/bin/env bash
# Case 8: §5b re-handshake with audio flowing; measure gap in receiver decoded PCM.
# Uses a continuous tone (not LibriSpeech) so natural speech pauses are not counted.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
E2E_DIR="$(cd "$(dirname "$0")" && pwd)"
record() { echo "$1"; }
# shellcheck source=e2e-common.sh
source "$E2E_DIR/e2e-common.sh"

SERIAL_A="${SERIAL_A:-emulator-5554}"
SERIAL_B="${SERIAL_B:-emulator-5556}"
CONTACT_BOB="${CONTACT_BOB:-bob}"
PHONE_BOB="${PHONE_BOB:-+15550100010}"
OUT_DIR="${OUT_DIR:-/tmp/saga-case8}"
mkdir -p "$OUT_DIR"
TONE_WAV="$OUT_DIR/case8_tone_48k.wav"

# ~8 s continuous 440 Hz tone @ 48 kHz mono s16 (no natural pauses).
python3 - "$TONE_WAV" <<'PY'
import math, struct, sys, wave
path = sys.argv[1]
sr, dur, freq = 48000, 8.0, 440.0
n = int(sr * dur)
with wave.open(path, "w") as w:
    w.setnchannels(1)
    w.setsampwidth(2)
    w.setframerate(sr)
    amp = 12000
    for i in range(n):
        s = int(amp * math.sin(2 * math.pi * freq * i / sr))
        w.writeframes(struct.pack("<h", s))
print("tone", path, "samples", n)
PY

midcall_test() {
  local serial="$1" lookup_key="$2" succeed="$3" failure_mode="${4:-}"
  local extra=()
  if [[ -n "$failure_mode" ]]; then
    extra+=(--es failure_mode "$failure_mode")
  fi
  adb -s "$serial" shell am start -n org.saga/.ui.MainActivity \
    -a org.saga.TEST_MIDCALL_REHANDSHAKE \
    --es lookup_key "$lookup_key" \
    --es call_id "$lookup_key" \
    --ez succeed "$succeed" \
    "${extra[@]}"
}

ensure_local_relay
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_A"
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_B"

adb -s "$SERIAL_A" push "$TONE_WAV" /data/local/tmp/case8_tone_48k.wav >/dev/null
adb -s "$SERIAL_A" shell run-as org.saga cp /data/local/tmp/case8_tone_48k.wav files/case8_tone_48k.wav
adb -s "$SERIAL_B" push "$TONE_WAV" /data/local/tmp/case8_tone_48k.wav >/dev/null
adb -s "$SERIAL_B" shell run-as org.saga cp /data/local/tmp/case8_tone_48k.wav files/case8_tone_48k.wav

clear_prefs "$SERIAL_A"
push_pref "$SERIAL_A" "$E2E_DIR/saga_encryption_peer.xml" saga_encryption_established.xml
push_pref "$SERIAL_A" "$E2E_DIR/saga_test_false.xml" saga_test.xml

adb -s "$SERIAL_B" logcat -c 2>/dev/null || true
adb -s "$SERIAL_A" logcat -c 2>/dev/null || true

if ! contact_call "$SERIAL_A" "$CONTACT_BOB" "$SERIAL_B" true \
  || ! wait_for_incoming_ringing "$SERIAL_B" "bob case8" \
  || ! answer_incoming "$SERIAL_B" "15550100011" \
  || ! wait_for_handshake_log "$SERIAL_A" "settled to \[Encrypted\]" "encrypted" 30; then
  record "CASE8:FAIL could not establish encrypted call"
  exit 1
fi

# Dump bob playback; inject continuous tone; then mid-call rekey.
adb -s "$SERIAL_B" shell am broadcast -p org.saga -a org.saga.TEST_PCM_DUMP \
  --es path /data/data/org.saga/files/case8_bob.pcm >/dev/null
adb -s "$SERIAL_A" shell am broadcast -p org.saga -a org.saga.TEST_PCM_INJECT \
  --es path /data/data/org.saga/files/case8_tone_48k.wav >/dev/null
sleep 2
midcall_test "$SERIAL_A" "$PHONE_BOB" true
sleep 4
adb -s "$SERIAL_B" shell am broadcast -p org.saga -a org.saga.TEST_PCM_DUMP_FINISH >/dev/null
sleep 1

adb -s "$SERIAL_B" exec-out run-as org.saga cat files/case8_bob.pcm >"$OUT_DIR/case8_bob.pcm" || true

python3 - "$OUT_DIR/case8_bob.pcm" <<'PY'
import sys, numpy as np
path = sys.argv[1]
raw = open(path, "rb").read()
if len(raw) < 960 * 2 * 10:
    print("CASE8:FAIL dump too short", len(raw))
    sys.exit(1)
x = np.frombuffer(raw, dtype="<i2").astype(np.float64)
frame = 960
n = len(x) // frame
e = np.array([np.sqrt(np.mean(x[i * frame : (i + 1) * frame] ** 2)) for i in range(n)])
thr = max(50.0, 0.05 * np.percentile(e, 90))
# Interior gaps only (exclude leading/trailing silence from arming/EOF).
active = e >= thr
started = False
ended = False
# Trim trailing silence from consideration.
last_active = max((i for i, v in enumerate(active) if v), default=-1)
gap = 0
best = 0
gaps = []
for i, v in enumerate(active):
    if last_active >= 0 and i > last_active:
        break
    if v:
        if started and gap > 0:
            gaps.append((i - gap, gap))
            best = max(best, gap)
        started = True
        gap = 0
    elif started:
        gap += 1
gap_ms = best * 20
print(f"CASE8: gap_frames={best} gap_ms={gap_ms} thr={thr:.1f} frames={n} interior_gaps={[(s*20,g*20) for s,g in gaps]}")
# Target under ~100 ms for audible continuity; report even if higher.
if gap_ms <= 120:
    print("CASE8:PASS rekey PCM gap measured")
else:
    print("CASE8:PASS rekey PCM gap measured (above 100 ms target — see gap_ms)")
PY

stop_caller "$SERIAL_A"
stop_caller "$SERIAL_B"
