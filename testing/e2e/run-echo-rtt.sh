#!/usr/bin/env bash
# Debug echo-mode mouth-to-ear on one clock (emulator only).
# Callee re-injects decoded PCM; caller injects a click and measures RTT locally.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
E2E_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=e2e-common.sh
source "$E2E_DIR/e2e-common.sh"

SERIAL_A="${SERIAL_A:-emulator-5554}"
SERIAL_B="${SERIAL_B:-emulator-5556}"
OUT_DIR="${OUT_DIR:-/tmp/saga-echo-rtt}"
mkdir -p "$OUT_DIR"

pcm_broadcast() {
  local serial="$1" action="$2"
  shift 2
  adb -s "$serial" shell am broadcast -p org.saga -a "$action" "$@" >/dev/null
}

ensure_local_relay
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_A"
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_B"

# caller contact callee answer_lookup label
# answer_lookup = remote (caller) phone digits, matching run-pcm-roq-metrics.sh
measure_path() {
  local caller="$1" contact="$2" callee="$3" answer_lookup="$4" label="$5"
  echo ""
  echo "=== Echo RTT $label (caller=$caller → $contact) ==="
  adb -s "$SERIAL_A" shell am force-stop org.saga
  adb -s "$SERIAL_B" shell am force-stop org.saga
  sleep 2
  ensure_callee_listening "$callee" || return 1
  wait_for_relay_online "$callee" "callee" || return 1
  warm_encrypted_caller "$caller" || return 1
  sleep 1
  pcm_broadcast "$callee" org.saga.TEST_ECHO_MODE --ez enabled true
  pcm_broadcast "$caller" org.saga.TEST_ECHO_MODE --ez enabled false
  pcm_broadcast "$caller" org.saga.TEST_PCM_DUMP --es path /data/data/org.saga/files/echo_rtt.pcm
  adb -s "$caller" logcat -c 2>/dev/null || true
  adb -s "$callee" logcat -c 2>/dev/null || true
  pcm_broadcast "$callee" org.saga.TEST_ECHO_MODE --ez enabled true
  pcm_broadcast "$caller" org.saga.TEST_PCM_DUMP --es path /data/data/org.saga/files/echo_rtt.pcm
  adb -s "$caller" shell am start -n org.saga/.ui.MainActivity \
    -a org.saga.TEST_CONTACT_CALL --es contact_name "$contact"
  wait_for_incoming_ringing "$callee" "echo-$label" \
    && answer_incoming "$callee" "$answer_lookup" \
    && wait_for_handshake_log "$caller" "settled to \[Encrypted\]" "enc-$label" 45 \
    && wait_for_handshake_log "$callee" "\\[Saga Media\\] start session=" "media-c-$label" 30 \
    && wait_for_handshake_log "$caller" "\\[Saga Media\\] start session=" "media-a-$label" 30
  adb -s "$caller" shell am start -a org.saga.TEST_IN_CALL_UI -n org.saga/.ui.SagaInCallActivity >/dev/null 2>&1 || true
  adb -s "$callee" shell am start -a org.saga.TEST_IN_CALL_UI -n org.saga/.ui.SagaInCallActivity >/dev/null 2>&1 || true
  sleep 2
  # Same-device inject+dump on caller → true RTT on one clock via echo.
  pcm_broadcast "$caller" org.saga.TEST_PCM_MARKER
  sleep 3
  local jb_line
  jb_line="$(adb -s "$caller" logcat -d | rg 'counters session=' | tail -1 || true)"
  echo "JB_AT_RTT $label: $jb_line"
  adb -s "$caller" logcat -d | rg 'LATENCY_MARKER|saga-echo-mode|counters session' \
    | tee "$OUT_DIR/${label}_logcat.txt" || true
  adb -s "$callee" logcat -d | rg 'saga-echo-mode|counters session' \
    | tee -a "$OUT_DIR/${label}_logcat.txt" || true
  local rtt
  rtt="$(rg -o 'm2e_ms=[0-9.]+' "$OUT_DIR/${label}_logcat.txt" | head -1 | cut -d= -f2 || true)"
  if [[ -n "$rtt" ]]; then
    python3 -c "rtt=float('$rtt'); print(f'ECHO_RTT $label: rtt_ms={rtt:.1f} one_way_ms={rtt/2:.1f} (emulator only)')"
  else
    echo "ECHO_RTT $label: FAIL no m2e_ms (see $OUT_DIR/${label}_logcat.txt)"
  fi
  pcm_broadcast "$callee" org.saga.TEST_ECHO_MODE --ez enabled false
  pcm_broadcast "$caller" org.saga.TEST_PCM_DUMP_FINISH
  stop_caller "$caller" || true
  stop_caller "$callee" || true
}

# alice→bob (bob echoes); bob answers with alice's lookup key
measure_path "$SERIAL_A" bob "$SERIAL_B" "15550100011" "alice_bob"
# bob→alice (alice echoes)
measure_path "$SERIAL_B" alice "$SERIAL_A" "15550100010" "bob_alice"

echo ""
echo "=== Echo RTT artifacts in $OUT_DIR ==="
ls -la "$OUT_DIR"
echo "NOTE: Emulator timings are not representative of phones."
