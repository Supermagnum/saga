#!/usr/bin/env bash
# Run a single E2E case from clean boot (falsification / pre-suite verification).
# Usage: ./run-case-standalone.sh 3|4|5|6
set -euo pipefail

CASE_NUM="${1:?usage: run-case-standalone.sh CASE_NUMBER (3|4|5|6)}"

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
E2E_DIR="$REPO_ROOT/testing/e2e"
SERIAL_A="${SERIAL_A:-emulator-5554}"
SERIAL_B="${SERIAL_B:-emulator-5556}"
CONTACT_BOB="${CONTACT_BOB:-bob}"
PHONE_BOB="${PHONE_BOB:-+15550100010}"
PHONE_BOB_RG='\+15550100010'
LISTEN_MARKER='\[Saga Iroh Listen\] endpoint bound and accepting inbound'
RELAY_MARKER='\[Saga Iroh Listen\] relay online'

RESULTS=()
record() { RESULTS+=("$1"); echo "$1"; }

# shellcheck source=e2e-common.sh
source "$E2E_DIR/e2e-common.sh"

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

echo "=== Case $CASE_NUM standalone — clean boot ==="
provision_clean_baseline "$SERIAL_A" "$SERIAL_B"

case "$CASE_NUM" in
  3)
    clear_prefs "$SERIAL_A"
    push_pref "$SERIAL_A" "$E2E_DIR/saga_test_false.xml" saga_test.xml
    if enc_flag "$SERIAL_A" "$PHONE_BOB"; then
      record "CASE3:FAIL encryption flag already set before call"
      exit 1
    fi
    contact_call "$SERIAL_A" "$CONTACT_BOB" "$SERIAL_B" true
    if wait_for_handshake_log "$SERIAL_A" "settled to \[Encrypted\]" "encrypted handshake"; then
      if enc_flag "$SERIAL_A" "$PHONE_BOB"; then
        record "CASE3:PASS flag set after alice->bob handshake"
      else
        record "CASE3:FAIL encryption flag not set"
        exit 1
      fi
    else
      record "CASE3:FAIL"
      exit 1
    fi
    ;;
  4)
    clear_prefs "$SERIAL_A"
    push_pref "$SERIAL_A" "$E2E_DIR/saga_encryption_peer.xml" saga_encryption_established.xml
    push_pref "$SERIAL_A" "$E2E_DIR/saga_test_true.xml" saga_test.xml
    contact_call "$SERIAL_A" "$CONTACT_BOB" "$SERIAL_B" true
    if wait_for_handshake_log "$SERIAL_A" "settled to \[Downgraded\]" "downgrade handshake" \
       && check_log "$SERIAL_A" "Saga Downgrade Event.*recorded downgrade"; then
      record "CASE4:PASS downgrade state + local log entry"
    else
      record "CASE4:FAIL"
      adb -s "$SERIAL_A" logcat -d | rg "Downgrade|Handshake|settled" | tail -12 || true
      exit 1
    fi
    ;;
  5)
    clear_prefs "$SERIAL_A"
    push_pref "$SERIAL_A" "$E2E_DIR/saga_encryption_peer.xml" saga_encryption_established.xml
    push_pref "$SERIAL_A" "$E2E_DIR/saga_test_false.xml" saga_test.xml
    contact_call "$SERIAL_A" "$CONTACT_BOB" "$SERIAL_B" true
    if ! wait_for_handshake_log "$SERIAL_A" "settled to \[Encrypted\]" "encrypted baseline"; then
      record "CASE5:FAIL could not establish encrypted call for mid-call test"
      exit 1
    fi
    adb -s "$SERIAL_A" logcat -c
    midcall_test "$SERIAL_A" "$PHONE_BOB" true
    sleep 5
    if check_log "$SERIAL_A" "Mid-call re-handshake succeeded" \
       && ! check_log "$SERIAL_A" "Playing mid_call_security_warning exactly once"; then
      record "CASE5a:PASS re-handshake success, no mid-call tone"
      adb -s "$SERIAL_A" logcat -c
      midcall_test "$SERIAL_A" "$PHONE_BOB" false crypto
      sleep 6
      if check_log "$SERIAL_A" "Playing mid_call_security_warning exactly once" \
         && check_log "$SERIAL_A" "Saga Downgrade Event.*recorded downgrade contact=\[$PHONE_BOB_RG\]" \
         && check_log "$SERIAL_A" "settled to \[Downgraded\]" \
         && ! check_log "$SERIAL_A" "Playing call_unsecure exactly once"; then
        record "CASE5b:PASS crypto failure reuses Case 4 downgrade log + distinct mid-call tone"
        record "CASE5:PASS"
      else
        record "CASE5b:FAIL"
        record "CASE5:FAIL"
        exit 1
      fi
    else
      record "CASE5a:FAIL"
      record "CASE5:FAIL"
      exit 1
    fi
    ;;
  6)
    adb -s "$SERIAL_A" shell setprop gsm.operator.alpha CarrierA 2>/dev/null || true
    adb -s "$SERIAL_B" shell setprop gsm.operator.alpha CarrierB 2>/dev/null || true
    clear_prefs "$SERIAL_A"
    push_pref "$SERIAL_A" "$E2E_DIR/saga_test_false.xml" saga_test.xml
    contact_call "$SERIAL_A" "$CONTACT_BOB" "$SERIAL_B" true
    if wait_for_handshake_log "$SERIAL_A" "Playing call_secure exactly once" "secure cue" \
       && check_log "$SERIAL_A" "settled to \[Encrypted\]"; then
      record "CASE6:PASS encrypted with carrier props (emulator relay via WiFi)"
    else
      record "CASE6:FAIL"
      adb -s "$SERIAL_A" logcat -d | rg "Saga|Handshake|secure|settled" | tail -12 || true
      exit 1
    fi
    ;;
  *)
    echo "Unknown case: $CASE_NUM (supported: 3, 4, 5, 6)" >&2
    exit 1
    ;;
esac

echo ""
printf '%s\n' "${RESULTS[@]}"
