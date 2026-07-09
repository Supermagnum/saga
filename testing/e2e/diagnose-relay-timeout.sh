#!/usr/bin/env bash
# Forensic relay investigation with native JNI poll (not logcat-as-IPC).
# Phase A: Case-2-position suite-start bob wait.
# Phase B: 8-cycle restart sweep (restart-count correlation).
# Default: local relay (SAGA_IROH_RELAY_MODE=public for N0 comparison).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
E2E_DIR="$REPO_ROOT/testing/e2e"
SERIAL_A="${SERIAL_A:-emulator-5554}"
SERIAL_B="${SERIAL_B:-emulator-5556}"
OUT_DIR="${OUT_DIR:-/tmp/saga-relay-forensics-$(date +%Y%m%d-%H%M%S)}"
SAGA_IROH_RELAY_MODE="${SAGA_IROH_RELAY_MODE:-local}"

mkdir -p "$OUT_DIR"
record() { echo "$1" | tee -a "$OUT_DIR/summary.log"; }

# shellcheck source=e2e-common.sh
source "$E2E_DIR/e2e-common.sh"

capture_relay_forensics() {
  local serial="$1" phase="$2"
  local outfile="$OUT_DIR/${serial//-/_}_${phase}.log"
  local native_status logcat_relay listen_count pid
  native_status="$(query_relay_ready_native "$serial")"
  logcat_relay="$(adb -s "$serial" logcat -d 2>/dev/null | rg -c '\[Saga Iroh Listen\] relay online' || true)"
  listen_count="$(adb -s "$serial" logcat -d 2>/dev/null | rg -c '\[Saga Iroh Listen\] endpoint bound' || true)"
  pid="$(adb -s "$serial" shell pidof org.saga 2>/dev/null | tr -d '\r' || true)"
  {
    echo "timestamp=$(date -Is)"
    echo "phase=$phase"
    echo "relay_mode=$SAGA_IROH_RELAY_MODE"
    echo "native_poll=$native_status"
    echo "logcat_relay_lines=$logcat_relay"
    echo "logcat_listen_lines=$listen_count"
    echo "pidof=$pid"
    echo "--- mismatch diagnostic ---"
    if [[ "$native_status" == "1" && "${logcat_relay:-0}" == "0" ]]; then
      echo "DETECTION_GAP: native ready but logcat marker absent"
    elif [[ "$native_status" != "1" && "${logcat_relay:-0}" != "0" ]]; then
      echo "DETECTION_GAP: logcat marker present but native not ready"
    elif [[ "$native_status" == "1" ]]; then
      echo "AGREE: native and logcat both indicate relay online"
    else
      echo "AGREE: neither native nor logcat shows relay online"
    fi
    echo "--- relay infra (last 30s) ---"
    adb -s "$serial" logcat -d -t '30s' 2>/dev/null \
      | rg 'relay|pkarr|probe timed|portmapper|net_report' | tail -20 || true
  } >"$outfile"
  echo "  forensics native=$native_status logcat_relay=$logcat_relay -> $outfile"
}

instrumented_native_relay_wait() {
  local serial="$1" tag="$2"
  local elapsed=0 interval="$RELAY_POLL_INTERVAL_SECS" max="$RELAY_POLL_MAX_SECS"
  local status
  record "WAIT_START $tag [$serial] mode=$SAGA_IROH_RELAY_MODE max=${max}s (native poll)"
  while (( elapsed < max )); do
    status="$(query_relay_ready_native "$serial")"
    if [[ "$status" == "$RELAY_READY_NATIVE" ]]; then
      record "NATIVE_PASS $tag [$serial] at ${elapsed}s"
      capture_relay_forensics "$serial" "${tag}_pass_${elapsed}s"
      return 0
    fi
    if (( elapsed > 0 && elapsed % 30 == 0 )); then
      capture_relay_forensics "$serial" "${tag}_t${elapsed}s"
    fi
    sleep "$interval"
    elapsed=$((elapsed + interval))
  done
  capture_relay_forensics "$serial" "${tag}_TIMEOUT"
  sleep 10
  capture_relay_forensics "$serial" "${tag}_post_timeout_+10s"
  local post_native
  post_native="$(query_relay_ready_native "$serial")"
  if [[ "$post_native" == "1" ]]; then
    record "POST_TIMEOUT_NATIVE_READY $tag [$serial] (genuine slow registration)"
    return 2
  fi
  record "RELAY_ABSENT $tag [$serial] native still pending after ${max}s + 10s"
  return 1
}

replay_suite_start_bob_path() {
  record ""
  record "=== PHASE A: Case-2-position bob wait ==="
  ensure_local_relay
  push_identity "$SERIAL_B" "bob"
  push_iroh_relay_pref "$SERIAL_B"
  bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_B" >/dev/null
  adb -s "$SERIAL_B" shell am force-stop org.saga
  sleep 1
  adb -s "$SERIAL_B" logcat -c 2>/dev/null || true
  bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_B" >/dev/null
  ensure_callee_listening "$SERIAL_B" || true
  instrumented_native_relay_wait "$SERIAL_B" "suite_start_case2" || true
}

restart_correlation_sweep() {
  local i elapsed status
  record ""
  record "=== PHASE B: 8-cycle restart sweep (native poll) ==="
  record "restart,seconds,result,native_at_end,logcat_relay_lines"
  push_iroh_relay_pref "$SERIAL_B"
  for i in $(seq 1 8); do
    adb -s "$SERIAL_B" shell am force-stop org.saga
    sleep 1
    adb -s "$SERIAL_B" logcat -c 2>/dev/null || true
    adb -s "$SERIAL_B" shell monkey -p org.saga -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || true
    sleep 3
    elapsed=0
    status="FAIL"
    while (( elapsed < RELAY_POLL_MAX_SECS )); do
      if [[ "$(query_relay_ready_native "$SERIAL_B")" == "$RELAY_READY_NATIVE" ]]; then
        status="PASS"
        break
      fi
      sleep "$RELAY_POLL_INTERVAL_SECS"
      elapsed=$((elapsed + RELAY_POLL_INTERVAL_SECS))
    done
    local native_end logcat_n
    native_end="$(query_relay_ready_native "$SERIAL_B")"
    logcat_n="$(adb -s "$SERIAL_B" logcat -d 2>/dev/null | rg -c '\[Saga Iroh Listen\] relay online' || true)"
    if [[ "$status" == "PASS" ]]; then
      record "RESTART $i: PASS at ${elapsed}s native=$native_end logcat=$logcat_n"
    else
      capture_relay_forensics "$SERIAL_B" "restart${i}_timeout"
      record "RESTART $i: FAIL at ${RELAY_POLL_MAX_SECS}s native=$native_end logcat=$logcat_n"
    fi
    record "$i,${elapsed:-$RELAY_POLL_MAX_SECS},$status,$native_end,${logcat_n:-0}"
  done
}

record "Relay forensics: $OUT_DIR mode=$SAGA_IROH_RELAY_MODE"
replay_suite_start_bob_path
restart_correlation_sweep
record "Done."
