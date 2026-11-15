#!/usr/bin/env bash
# Verify caller sends no media while callee is RINGING; JB flush on answer.
# Expect: sent=0 on caller before answer; answered signal; post-answer metrics healthy.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
E2E_DIR="$(cd "$(dirname "$0")" && pwd)"
record() { echo "$1"; }
# shellcheck source=e2e-common.sh
source "$E2E_DIR/e2e-common.sh"

SERIAL_A="${SERIAL_A:-emulator-5554}"
SERIAL_B="${SERIAL_B:-emulator-5556}"
OUT_DIR="${OUT_DIR:-/tmp/saga-preanswer}"
mkdir -p "$OUT_DIR"

ensure_local_relay

run_direction() {
  local caller="$1" contact="$2" callee="$3" label="$4" answer_lookup="$5"
  echo ""
  echo "=== Pre-answer gate: $label ($caller → $callee) ==="
  adb -s "$caller" shell am force-stop org.saga
  adb -s "$callee" shell am force-stop org.saga
  sleep 2
  ensure_callee_listening "$callee" || return 1
  wait_for_relay_online "$callee" "callee" || return 1
  warm_encrypted_caller "$caller" || return 1
  sleep 1
  adb -s "$caller" logcat -c 2>/dev/null || true
  adb -s "$callee" logcat -c 2>/dev/null || true

  adb -s "$caller" shell am start -n org.saga/.ui.MainActivity \
    -a org.saga.TEST_CONTACT_CALL --es contact_name "$contact"

  if ! wait_for_incoming_ringing "$callee" "$label-ring" 45; then
    record "PREANSWER:FAIL $label no ring"
    return 1
  fi

  # Hold in RINGING so caller would have been sending under the old path.
  sleep 3
  adb -s "$caller" logcat -d >"$OUT_DIR/${label}_pre_answer_caller.txt" || true
  adb -s "$callee" logcat -d >"$OUT_DIR/${label}_pre_answer_callee.txt" || true

  local pre_sent
  pre_sent="$(rg -o 'sent=[0-9]+' "$OUT_DIR/${label}_pre_answer_caller.txt" | tail -1 | rg -o '[0-9]+' || echo 0)"
  if [[ -z "$pre_sent" ]]; then
    pre_sent=0
  fi
  if (( pre_sent > 0 )); then
    record "PREANSWER:FAIL $label caller sent=$pre_sent before answer (want 0)"
    return 1
  fi
  record "PREANSWER:PASS $label caller sent=0 while RINGING"

  if ! rg -q 'send/playout gated until answered|gated until answered' \
    "$OUT_DIR/${label}_pre_answer_caller.txt" \
    "$OUT_DIR/${label}_pre_answer_callee.txt"; then
    record "PREANSWER:WARN $label gated log marker not found (check APK)"
  fi

  # Callee must not have Audio I/O before answer.
  if rg -q 'audio I/O started' "$OUT_DIR/${label}_pre_answer_callee.txt"; then
    record "PREANSWER:FAIL $label callee audio I/O before answer"
    return 1
  fi
  record "PREANSWER:PASS $label callee capture/audio off while RINGING"

  answer_incoming "$callee" "$answer_lookup"
  sleep 2

  adb -s "$caller" logcat -d >"$OUT_DIR/${label}_post_answer_caller.txt" || true
  adb -s "$callee" logcat -d >"$OUT_DIR/${label}_post_answer_callee.txt" || true

  if ! rg -q 'local answered|JB flushed|answered signal' \
    "$OUT_DIR/${label}_post_answer_callee.txt"; then
    record "PREANSWER:FAIL $label missing local answered / flush log"
    return 1
  fi
  record "PREANSWER:PASS $label callee flush + answered signal"

  if ! rg -q 'answered signal received|peer answered signal observed|audio I/O started' \
    "$OUT_DIR/${label}_post_answer_caller.txt"; then
    record "PREANSWER:FAIL $label caller did not arm after answered signal"
    return 1
  fi
  record "PREANSWER:PASS $label caller armed after answered signal"

  # After answer, media should flow (sent > 0).
  sleep 3
  adb -s "$caller" logcat -d | rg 'counters session=' | tee "$OUT_DIR/${label}_counters.txt" | tail -5 || true
  local post_sent
  post_sent="$(rg -o 'sent=[0-9]+' "$OUT_DIR/${label}_counters.txt" | tail -1 | rg -o '[0-9]+' || echo 0)"
  if (( post_sent < 10 )); then
    record "PREANSWER:FAIL $label post-answer sent=$post_sent (want >=10)"
    return 1
  fi
  record "PREANSWER:PASS $label post-answer sent=$post_sent"

  stop_caller "$caller" || true
  stop_caller "$callee" || true
  return 0
}

run_direction "$SERIAL_A" bob "$SERIAL_B" "alice_to_bob" "15550100011"
run_direction "$SERIAL_B" alice "$SERIAL_A" "bob_to_alice" "15550100010"

echo ""
echo "=== Pre-answer gate artifacts in $OUT_DIR ==="
ls -la "$OUT_DIR"
