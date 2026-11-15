#!/usr/bin/env bash
# RING flake regression (issue #20): Cases 6, 4b, 10 — cold app start each run.
# Target: 20/20 RING:PASS on each case. Captures dump_ring_failure artifacts on fail.
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
E2E_DIR="$REPO_ROOT/testing/e2e"
SERIAL_A="${SERIAL_A:-emulator-5554}"
SERIAL_B="${SERIAL_B:-emulator-5556}"
N="${N:-20}"
export RING_FAIL_DIR="${RING_FAIL_DIR:-/tmp/saga-ring-fail}"
OUT_DIR="${OUT_DIR:-/tmp/ring-flake-regression}"
mkdir -p "$OUT_DIR" "$RING_FAIL_DIR"

RESULTS=()
record() { RESULTS+=("$1"); echo "$1"; }
# shellcheck source=e2e-common.sh
source "$E2E_DIR/e2e-common.sh"

cold_both() {
  clear_stuck_telephony_calls "$SERIAL_A" 2>/dev/null || true
  clear_stuck_telephony_calls "$SERIAL_B" 2>/dev/null || true
  adb -s "$SERIAL_A" shell am force-stop org.saga
  adb -s "$SERIAL_B" shell am force-stop org.saga
  sleep 1
  adb -s "$SERIAL_A" logcat -c 2>/dev/null || true
  adb -s "$SERIAL_B" logcat -c 2>/dev/null || true
}

setup_once() {
  ensure_local_relay
  for s in "$SERIAL_A" "$SERIAL_B"; do
    adb -s "$s" shell cmd role add-role-holder android.app.role.DIALER org.saga 2>/dev/null || true
    enable_saga_phone_account "$s" || true
  done
  push_identity "$SERIAL_A" "alice"
  push_identity "$SERIAL_B" "bob"
  push_iroh_relay_pref "$SERIAL_A"
  push_iroh_relay_pref "$SERIAL_B"
  bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_A" >/dev/null
  bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_B" >/dev/null
}

# Returns 0 on RING:PASS.
run_ring_once() {
  local case_id="$1" i="$2"
  cold_both
  clear_prefs "$SERIAL_A"
  clear_prefs "$SERIAL_B"
  case "$case_id" in
    4b)
      push_pref "$SERIAL_A" "$E2E_DIR/saga_encryption_peer.xml" saga_encryption_established.xml
      push_pref "$SERIAL_B" "$E2E_DIR/saga_encryption_peer_bob.xml" saga_encryption_established.xml
      push_pref "$SERIAL_A" "$E2E_DIR/saga_test_binding_perturb.xml" saga_test.xml
      push_pref "$SERIAL_B" "$E2E_DIR/saga_test_false.xml" saga_test.xml
      ;;
    6|10)
      push_pref "$SERIAL_A" "$E2E_DIR/saga_encryption_peer.xml" saga_encryption_established.xml
      push_pref "$SERIAL_A" "$E2E_DIR/saga_test_false.xml" saga_test.xml
      ;;
    *)
      echo "unknown case $case_id" >&2
      return 1
      ;;
  esac
  start_listener "$SERIAL_B"
  if ! wait_for_relay_online "$SERIAL_B" "bob-$case_id-$i"; then
    record "RING_REGRESS:$case_id#$i RELAY_FAIL"
    return 1
  fi
  if ! contact_call "$SERIAL_A" bob "$SERIAL_B" true; then
    record "RING_REGRESS:$case_id#$i DIAL_FAIL"
    stop_caller "$SERIAL_A" || true
    stop_caller "$SERIAL_B" || true
    return 1
  fi
  if wait_for_incoming_ringing "$SERIAL_B" "$case_id-$i" 45; then
    record "RING_REGRESS:$case_id#$i RING_PASS"
    stop_caller "$SERIAL_A" || true
    stop_caller "$SERIAL_B" || true
    return 0
  fi
  record "RING_REGRESS:$case_id#$i RING_FAIL"
  stop_caller "$SERIAL_A" || true
  stop_caller "$SERIAL_B" || true
  return 1
}

setup_once
echo "=== RING flake regression N=$N devices=$SERIAL_A/$SERIAL_B ==="

declare -A FAIL_COUNTS
FAIL_COUNTS[4b]=0
FAIL_COUNTS[6]=0
FAIL_COUNTS[10]=0

for case_id in 4b 6 10; do
  for i in $(seq 1 "$N"); do
    echo "--- Case $case_id run $i/$N ---"
    if ! run_ring_once "$case_id" "$i"; then
      FAIL_COUNTS[$case_id]=$((FAIL_COUNTS[$case_id] + 1))
    fi
  done
done

echo ""
echo "=== RING flake regression summary ==="
ok=1
for case_id in 4b 6 10; do
  f="${FAIL_COUNTS[$case_id]}"
  p=$((N - f))
  record "RING_REGRESS_SUMMARY case=$case_id pass=$p/$N fail=$f/$N"
  if (( f != 0 )); then
    ok=0
  fi
done
printf '%s\n' "${RESULTS[@]}" | tee "$OUT_DIR/summary.txt"
if (( ok == 1 )); then
  echo "RING_REGRESS:PASS 20/20 (or N/N) on Cases 6, 4b, 10"
  exit 0
fi
echo "RING_REGRESS:FAIL see $RING_FAIL_DIR"
exit 1
