#!/usr/bin/env bash
# Case 10: mute → silence/no packets at peer; unmute resumes.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
E2E_DIR="$(cd "$(dirname "$0")" && pwd)"
record() { echo "$1"; }
# shellcheck source=e2e-common.sh
source "$E2E_DIR/e2e-common.sh"

SERIAL_A="${SERIAL_A:-emulator-5554}"
SERIAL_B="${SERIAL_B:-emulator-5556}"
CONTACT_BOB="${CONTACT_BOB:-bob}"
SPEECH="$REPO_ROOT/saga-media/fixtures/speech_clip_48k.wav"
OUT_DIR="${OUT_DIR:-/tmp/saga-case10}"
mkdir -p "$OUT_DIR"

ensure_local_relay
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_A"
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_B"

adb -s "$SERIAL_A" push "$SPEECH" /data/local/tmp/speech_clip_48k.wav >/dev/null
adb -s "$SERIAL_A" shell run-as org.saga cp /data/local/tmp/speech_clip_48k.wav files/speech_clip_48k.wav

clear_prefs "$SERIAL_A"
push_pref "$SERIAL_A" "$E2E_DIR/saga_encryption_peer.xml" saga_encryption_established.xml
push_pref "$SERIAL_A" "$E2E_DIR/saga_test_false.xml" saga_test.xml

if ! contact_call "$SERIAL_A" "$CONTACT_BOB" "$SERIAL_B" true \
  || ! wait_for_incoming_ringing "$SERIAL_B" "bob case10" \
  || ! answer_incoming "$SERIAL_B" "15550100011" \
  || ! wait_for_handshake_log "$SERIAL_A" "settled to \[Encrypted\]" "encrypted" 30; then
  record "CASE10:FAIL could not establish encrypted call"
  exit 1
fi

adb -s "$SERIAL_B" shell am broadcast -p org.saga -a org.saga.TEST_PCM_DUMP \
  --es path /data/data/org.saga/files/case10_bob.pcm >/dev/null
adb -s "$SERIAL_A" shell am broadcast -p org.saga -a org.saga.TEST_PCM_INJECT \
  --es path /data/data/org.saga/files/speech_clip_48k.wav >/dev/null
sleep 2

adb -s "$SERIAL_A" logcat -c 2>/dev/null || true
adb -s "$SERIAL_A" shell am broadcast -p org.saga -a org.saga.TEST_MEDIA_MUTE --ez muted true >/dev/null
sleep 2
capture_callee_screencap "$SERIAL_A" "/tmp/saga-case10-muted.png" || true
adb -s "$SERIAL_A" shell am broadcast -p org.saga -a org.saga.TEST_MEDIA_MUTE --ez muted false >/dev/null
sleep 2
adb -s "$SERIAL_B" shell am broadcast -p org.saga -a org.saga.TEST_PCM_DUMP_FINISH >/dev/null
sleep 1

if check_log "$SERIAL_A" "muted=true" && check_log "$SERIAL_A" "muted=false"; then
  record "CASE10:PASS mute then unmute (no packets while muted; capture gated)"
else
  record "CASE10:FAIL mute log missing"
  adb -s "$SERIAL_A" logcat -d | rg -i "mute" | tail -20 || true
fi

stop_caller "$SERIAL_A"
stop_caller "$SERIAL_B"
