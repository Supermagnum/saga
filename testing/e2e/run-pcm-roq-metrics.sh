#!/usr/bin/env bash
# Emulator↔emulator RoQ speech metrics via PCM inject/dump (real network media).
# Uses broadcast intents so MainActivity does not steal in-call focus mid-call.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
E2E_DIR="$(cd "$(dirname "$0")" && pwd)"
record() { echo "$1"; }
# shellcheck source=e2e-common.sh
source "$E2E_DIR/e2e-common.sh"

SERIAL_A="${SERIAL_A:-emulator-5554}"
SERIAL_B="${SERIAL_B:-emulator-5556}"
SPEECH="$REPO_ROOT/saga-media/fixtures/speech_clip_48k.wav"
OUT_DIR="${OUT_DIR:-/tmp/saga-roq-pcm}"
mkdir -p "$OUT_DIR"

DEVICE_SPEECH_REL="files/speech_clip_48k.wav"
DEVICE_DUMP_A_REL="files/saga_play_a.pcm"
DEVICE_DUMP_B_REL="files/saga_play_b.pcm"
APP_SPEECH="/data/data/org.saga/files/speech_clip_48k.wav"
APP_DUMP_A="/data/data/org.saga/files/saga_play_a.pcm"
APP_DUMP_B="/data/data/org.saga/files/saga_play_b.pcm"

stage_speech() {
  local serial="$1"
  adb -s "$serial" push "$SPEECH" /data/local/tmp/speech_clip_48k.wav >/dev/null
  adb -s "$serial" shell run-as org.saga cp /data/local/tmp/speech_clip_48k.wav "$DEVICE_SPEECH_REL"
}

pull_app_file() {
  local serial="$1" rel="$2" host="$3"
  adb -s "$serial" exec-out run-as org.saga cat "$rel" >"$host"
}

pcm_broadcast() {
  local serial="$1" action="$2"
  shift 2
  # RECEIVER_EXPORTED debug receiver — no activity focus change.
  adb -s "$serial" shell am broadcast -p org.saga -a "$action" "$@" >/dev/null
}

arm_dump() { pcm_broadcast "$1" org.saga.TEST_PCM_DUMP --es path "$2"; }
arm_inject() { pcm_broadcast "$1" org.saga.TEST_PCM_INJECT --es path "$2"; }
clear_inject() { pcm_broadcast "$1" org.saga.TEST_PCM_INJECT; }
finish_dump() { pcm_broadcast "$1" org.saga.TEST_PCM_DUMP_FINISH; }

ensure_local_relay

echo "=== Stage speech fixture into app files ==="
stage_speech "$SERIAL_A"
stage_speech "$SERIAL_B"

echo "=== Seed contacts ==="
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_A"
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_B"

encrypted_call_armed() {
  local caller="$1" contact="$2" callee="$3" inject_serial="$4" dump_serial="$5" dump_path="$6"
  ensure_callee_listening "$callee" || return 1
  wait_for_relay_online "$callee" "callee" || return 1
  warm_encrypted_caller "$caller" || return 1
  # Force-stop clears process; re-stage speech on caller after warm.
  stage_speech "$caller"
  stage_speech "$callee"
  sleep 1
  arm_dump "$dump_serial" "$dump_path"
  arm_inject "$inject_serial" "$APP_SPEECH"
  if [[ "$inject_serial" == "$caller" ]]; then
    clear_inject "$callee"
  else
    clear_inject "$caller"
  fi
  sleep 1
  adb -s "$caller" logcat -c 2>/dev/null || true
  adb -s "$callee" logcat -c 2>/dev/null || true
  # Re-arm after logcat clear (state is in-process, not log-dependent).
  arm_dump "$dump_serial" "$dump_path"
  arm_inject "$inject_serial" "$APP_SPEECH"
  adb -s "$caller" shell am start -n org.saga/.ui.MainActivity \
    -a org.saga.TEST_CONTACT_CALL --es contact_name "$contact"
}

pull_score() {
  local serial="$1" rel="$2" label="$3" host_raw="$4"
  pull_app_file "$serial" "$rel" "$host_raw"
  local sz
  sz=$(stat -c%s "$host_raw" 2>/dev/null || echo 0)
  echo "pulled $label bytes=$sz"
  if (( sz < 96000 )); then
    echo "WARN: $label dump short ($sz bytes) — may be silence/no media"
  fi
  python3 "$E2E_DIR/pcm_score.py" \
    --ref "$SPEECH" \
    --deg-raw "$host_raw" \
    --label "$label" \
    --out-wav "${host_raw%.pcm}.wav"
}

echo ""
echo "=== Direction A→B: alice inject, bob dump ==="
encrypted_call_armed "$SERIAL_A" bob "$SERIAL_B" "$SERIAL_A" "$SERIAL_B" "$APP_DUMP_B"
wait_for_incoming_ringing "$SERIAL_B" "bob-pcm" \
  && answer_incoming "$SERIAL_B" "15550100011" \
  && wait_for_handshake_log "$SERIAL_A" "settled to \[Encrypted\]" "encrypted-a" 45 \
  && wait_for_handshake_log "$SERIAL_B" "\\[Saga Media\\] start session=" "media-b" 30 \
  && wait_for_handshake_log "$SERIAL_A" "\\[Saga Media\\] start session=" "media-a" 30

adb -s "$SERIAL_B" shell am start -a org.saga.TEST_IN_CALL_UI -n org.saga/.ui.SagaInCallActivity >/dev/null 2>&1 || true
adb -s "$SERIAL_A" shell am start -a org.saga.TEST_IN_CALL_UI -n org.saga/.ui.SagaInCallActivity >/dev/null 2>&1 || true
sleep 7
capture_callee_screencap "$SERIAL_B" "$OUT_DIR/bob-active-call.png" || true
adb -s "$SERIAL_A" shell screencap -p /sdcard/saga_alice_active.png >/dev/null 2>&1 || true
adb -s "$SERIAL_A" pull /sdcard/saga_alice_active.png "$OUT_DIR/alice-active-call.png" >/dev/null 2>&1 || true

finish_dump "$SERIAL_B"
adb -s "$SERIAL_A" logcat -d | rg 'PcmDebug|inject|dump|counters session' | tee "$OUT_DIR/alice_to_bob_logcat_a.txt" | tail -20 || true
adb -s "$SERIAL_B" logcat -d | rg 'PcmDebug|inject|dump|counters session' | tee "$OUT_DIR/alice_to_bob_logcat_b.txt" | tail -20 || true
# Receiver JB stats (bob dumps alice→bob).
rg 'counters session=\[inbound' "$OUT_DIR/alice_to_bob_logcat_b.txt" | tail -3 || true
stop_caller "$SERIAL_A" || true
stop_caller "$SERIAL_B" || true
sleep 1
pull_score "$SERIAL_B" "$DEVICE_DUMP_B_REL" "alice_to_bob" "$OUT_DIR/alice_to_bob.pcm"

echo ""
echo "=== Direction B→A: bob inject, alice dump ==="
# Hard reset so stale dump/inject/Telecom state cannot poison the reverse path.
adb -s "$SERIAL_A" shell am force-stop org.saga
adb -s "$SERIAL_B" shell am force-stop org.saga
sleep 3
encrypted_call_armed "$SERIAL_B" alice "$SERIAL_A" "$SERIAL_B" "$SERIAL_A" "$APP_DUMP_A"
wait_for_incoming_ringing "$SERIAL_A" "alice-pcm" \
  && answer_incoming "$SERIAL_A" "15550100010" \
  && wait_for_handshake_log "$SERIAL_B" "settled to \[Encrypted\]" "encrypted-b" 45 \
  && wait_for_handshake_log "$SERIAL_A" "\\[Saga Media\\] start session=" "media-a2" 45 \
  && wait_for_handshake_log "$SERIAL_B" "\\[Saga Media\\] start session=" "media-b2" 30

adb -s "$SERIAL_A" shell am start -a org.saga.TEST_IN_CALL_UI -n org.saga/.ui.SagaInCallActivity >/dev/null 2>&1 || true
sleep 8
capture_callee_screencap "$SERIAL_A" "$OUT_DIR/alice-active-call-b2a.png" || true

finish_dump "$SERIAL_A"
adb -s "$SERIAL_B" logcat -d | rg 'PcmDebug|inject|dump|counters session' | tee "$OUT_DIR/bob_to_alice_logcat_b.txt" | tail -25 || true
adb -s "$SERIAL_A" logcat -d | rg 'PcmDebug|inject|dump|counters session' | tee "$OUT_DIR/bob_to_alice_logcat_a.txt" | tail -25 || true
rg 'counters session=\[inbound' "$OUT_DIR/bob_to_alice_logcat_a.txt" | tail -3 || true
stop_caller "$SERIAL_A" || true
stop_caller "$SERIAL_B" || true
sleep 1
pull_score "$SERIAL_A" "$DEVICE_DUMP_A_REL" "bob_to_alice" "$OUT_DIR/bob_to_alice.pcm"

echo ""
echo "=== Metrics artifacts in $OUT_DIR ==="
ls -la "$OUT_DIR"
