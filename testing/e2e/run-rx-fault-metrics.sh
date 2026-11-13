#!/usr/bin/env bash
# Debug Rx fault injector: 5% drop + ~30 ms jitter on alice↔bob (both directions).
# No sudo / tc needed — receive-side only (emulator).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
E2E_DIR="$(cd "$(dirname "$0")" && pwd)"
record() { echo "$1"; }
# shellcheck source=e2e-common.sh
source "$E2E_DIR/e2e-common.sh"

SERIAL_A="${SERIAL_A:-emulator-5554}"
SERIAL_B="${SERIAL_B:-emulator-5556}"
SPEECH="$REPO_ROOT/saga-media/fixtures/speech_clip_48k.wav"
OUT_DIR="${OUT_DIR:-/tmp/saga-rx-fault}"
DROP_PCT="${DROP_PCT:-5}"
JITTER_MS="${JITTER_MS:-30}"
mkdir -p "$OUT_DIR"

DEVICE_SPEECH_REL="files/speech_clip_48k.wav"
DEVICE_DUMP_A_REL="files/saga_play_a.pcm"
DEVICE_DUMP_B_REL="files/saga_play_b.pcm"
APP_SPEECH="/data/data/org.saga/files/speech_clip_48k.wav"
APP_DUMP_A="/data/data/org.saga/files/saga_play_a.pcm"
APP_DUMP_B="/data/data/org.saga/files/saga_play_b.pcm"

pcm_broadcast() {
  local serial="$1" action="$2"
  shift 2
  adb -s "$serial" shell am broadcast -p org.saga -a "$action" "$@" >/dev/null
}

stage_speech() {
  local serial="$1"
  adb -s "$serial" push "$SPEECH" /data/local/tmp/speech_clip_48k.wav >/dev/null
  adb -s "$serial" shell run-as org.saga cp /data/local/tmp/speech_clip_48k.wav "$DEVICE_SPEECH_REL"
}

pull_app_file() {
  local serial="$1" rel="$2" host="$3"
  adb -s "$serial" exec-out run-as org.saga cat "$rel" >"$host"
}

arm_dump() { pcm_broadcast "$1" org.saga.TEST_PCM_DUMP --es path "$2"; }
arm_inject() { pcm_broadcast "$1" org.saga.TEST_PCM_INJECT --es path "$2"; }
clear_inject() { pcm_broadcast "$1" org.saga.TEST_PCM_INJECT; }
finish_dump() { pcm_broadcast "$1" org.saga.TEST_PCM_DUMP_FINISH; }
arm_rx_fault() { pcm_broadcast "$1" org.saga.TEST_RX_FAULT --ei drop_pct "$DROP_PCT" --ei jitter_ms "$JITTER_MS"; }
clear_rx_fault() { pcm_broadcast "$1" org.saga.TEST_RX_FAULT --ei drop_pct 0 --ei jitter_ms 0; }

ensure_local_relay
stage_speech "$SERIAL_A"
stage_speech "$SERIAL_B"
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_A"
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_B"

encrypted_call_armed() {
  local caller="$1" contact="$2" callee="$3" inject_serial="$4" dump_serial="$5" dump_path="$6"
  ensure_callee_listening "$callee" || return 1
  wait_for_relay_online "$callee" "callee" || return 1
  warm_encrypted_caller "$caller" || return 1
  stage_speech "$caller"
  stage_speech "$callee"
  sleep 1
  arm_rx_fault "$dump_serial"
  clear_rx_fault "$inject_serial"
  arm_dump "$dump_serial" "$dump_path"
  arm_inject "$inject_serial" "$APP_SPEECH"
  if [[ "$inject_serial" == "$caller" ]]; then
    clear_inject "$callee"
  else
    clear_inject "$caller"
  fi
  sleep 1
  adb -s "$caller" logcat -c 2>/dev/null || true
  adb -s "$callee" logcat -d >/dev/null 2>&1 || true
  adb -s "$callee" logcat -c 2>/dev/null || true
  arm_rx_fault "$dump_serial"
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
  python3 "$E2E_DIR/pcm_score.py" \
    --ref "$SPEECH" \
    --deg-raw "$host_raw" \
    --label "$label" \
    --out-wav "${host_raw%.pcm}.wav"
}

run_dir() {
  local caller="$1" contact="$2" callee="$3" inject="$4" dump="$5" dump_app="$6" dump_rel="$7" label="$8" answer_num="$9"
  echo ""
  echo "=== Rx-fault $label (${DROP_PCT}% drop + ${JITTER_MS}ms jitter) ==="
  encrypted_call_armed "$caller" "$contact" "$callee" "$inject" "$dump" "$dump_app"
  wait_for_incoming_ringing "$callee" "rxf-$label" \
    && answer_incoming "$callee" "$answer_num" \
    && wait_for_handshake_log "$caller" "settled to \[Encrypted\]" "enc-$label" 45 \
    && wait_for_handshake_log "$callee" "\\[Saga Media\\] start session=" "media-c-$label" 30 \
    && wait_for_handshake_log "$caller" "\\[Saga Media\\] start session=" "media-a-$label" 30
  adb -s "$callee" shell am start -a org.saga.TEST_IN_CALL_UI -n org.saga/.ui.SagaInCallActivity >/dev/null 2>&1 || true
  adb -s "$caller" shell am start -a org.saga.TEST_IN_CALL_UI -n org.saga/.ui.SagaInCallActivity >/dev/null 2>&1 || true
  sleep 5
  finish_dump "$dump"
  adb -s "$dump" logcat -d | rg 'saga-rx-fault|counters session|PcmDebug' \
    | tee "$OUT_DIR/${label}_logcat.txt" | tail -40 || true
  rg 'counters session=' "$OUT_DIR/${label}_logcat.txt" | tail -3 || true
  clear_rx_fault "$dump"
  stop_caller "$caller" || true
  stop_caller "$callee" || true
  sleep 1
  pull_score "$dump" "$dump_rel" "$label" "$OUT_DIR/${label}.pcm"
}

run_dir "$SERIAL_A" bob "$SERIAL_B" "$SERIAL_A" "$SERIAL_B" "$APP_DUMP_B" "$DEVICE_DUMP_B_REL" \
  "alice_to_bob_fault" "15550100011"
adb -s "$SERIAL_A" shell am force-stop org.saga
adb -s "$SERIAL_B" shell am force-stop org.saga
sleep 3
run_dir "$SERIAL_B" alice "$SERIAL_A" "$SERIAL_B" "$SERIAL_A" "$APP_DUMP_A" "$DEVICE_DUMP_A_REL" \
  "bob_to_alice_fault" "15550100010"

echo ""
echo "=== Rx-fault artifacts in $OUT_DIR ==="
ls -la "$OUT_DIR"
