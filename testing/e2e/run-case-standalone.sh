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
mkdir -p /tmp/saga-inbound-screencaps

# alice (5554) calls bob (5556) in cases 3–6; inbound answer uses alice's dev endpoint label.
CALLEE="$SERIAL_B"
CALLER_LOOKUP="15550100011"

ring_answer_bob() {
  local case_label="$1" screencap_path="$2"
  if ! wait_for_incoming_ringing "$CALLEE" "bob ($case_label)"; then
    record "CASE${CASE_NUM}:FAIL callee ringing checkpoints"
    adb -s "$CALLEE" logcat -d | rg "CHECKPOINT|Incoming|InCallService" | tail -15 || true
    return 1
  fi
  capture_callee_screencap "$CALLEE" "$screencap_path"
  answer_incoming "$CALLEE" "$CALLER_LOOKUP"
  sleep 2
  return 0
}

case "$CASE_NUM" in
  3)
    clear_prefs "$SERIAL_A"
    push_pref "$SERIAL_A" "$E2E_DIR/saga_test_false.xml" saga_test.xml
    if enc_flag "$SERIAL_A" "$PHONE_BOB"; then
      record "CASE3:FAIL encryption flag already set before call"
      exit 1
    fi
    adb -s "$SERIAL_B" logcat -c 2>/dev/null || true
    contact_call "$SERIAL_A" "$CONTACT_BOB" "$SERIAL_B" true || {
      record "CASE3:FAIL contact_call"
      exit 1
    }
    if ! wait_for_incoming_ringing "$SERIAL_B" "bob (first-trust callee)"; then
      record "CASE3:FAIL callee ringing checkpoints"
      adb -s "$SERIAL_B" logcat -d | rg "CHECKPOINT|Incoming|InCallService" | tail -15 || true
      exit 1
    fi
    capture_callee_screencap "$SERIAL_B" "/tmp/saga-inbound-screencaps/case3-bob-ringing.png"
    answer_incoming "$SERIAL_B" "15550100011"
    sleep 2
    if wait_for_handshake_log "$SERIAL_A" "settled to \[Encrypted\]" "encrypted handshake" \
       && check_log "$SERIAL_A" "Playing call_secure exactly once" \
       && check_log "$SERIAL_B" "CHECKPOINT incoming Connection setActive" \
       && enc_flag "$SERIAL_A" "$PHONE_BOB"; then
      record "CASE3:PASS first-trust ring + answer + encrypted + flag set"
    else
      record "CASE3:FAIL"
      adb -s "$SERIAL_A" logcat -d | rg "Handshake|encryption|call_secure" | tail -8 || true
      adb -s "$SERIAL_B" logcat -d | rg "CHECKPOINT|Handshake|setActive" | tail -8 || true
      exit 1
    fi
    ;;
  4)
    clear_prefs "$SERIAL_A"
    push_pref "$SERIAL_A" "$E2E_DIR/saga_encryption_peer.xml" saga_encryption_established.xml
    push_pref "$SERIAL_B" "$E2E_DIR/saga_encryption_peer_bob.xml" saga_encryption_established.xml
    push_pref "$SERIAL_A" "$E2E_DIR/saga_test_true.xml" saga_test.xml
    adb -s "$CALLEE" logcat -c 2>/dev/null || true
    contact_call "$SERIAL_A" "$CONTACT_BOB" "$SERIAL_B" true || {
      record "CASE4:FAIL contact_call"
      exit 1
    }
    ring_answer_bob "downgrade callee" "/tmp/saga-inbound-screencaps/case4-bob-ringing.png" || exit 1
    if wait_for_handshake_log "$SERIAL_A" "settled to \[Downgraded\]" "downgrade handshake" \
       && check_log "$SERIAL_A" "Saga Downgrade Event.*recorded downgrade" \
       && check_log "$SERIAL_B" "Handshake settled to \[Downgraded\]"; then
      record "CASE4:PASS ring + answer + downgrade state + local log entry"
    else
      record "CASE4:FAIL"
      adb -s "$SERIAL_A" logcat -d | rg "Downgrade|Handshake|settled" | tail -12 || true
      adb -s "$SERIAL_B" logcat -d | rg "CHECKPOINT|Downgrade|setActive" | tail -8 || true
      exit 1
    fi
    ;;
  5)
    clear_prefs "$SERIAL_A"
    push_pref "$SERIAL_A" "$E2E_DIR/saga_encryption_peer.xml" saga_encryption_established.xml
    push_pref "$SERIAL_A" "$E2E_DIR/saga_test_false.xml" saga_test.xml
    adb -s "$CALLEE" logcat -c 2>/dev/null || true
    contact_call "$SERIAL_A" "$CONTACT_BOB" "$SERIAL_B" true || {
      record "CASE5:FAIL contact_call"
      exit 1
    }
    ring_answer_bob "mid-call setup callee" "/tmp/saga-inbound-screencaps/case5-bob-ringing.png" || exit 1
    if ! wait_for_handshake_log "$SERIAL_A" "settled to \[Encrypted\]" "encrypted baseline" \
       || ! check_log "$SERIAL_B" "CHECKPOINT incoming Connection setActive"; then
      record "CASE5:FAIL could not establish encrypted call for mid-call test"
      adb -s "$SERIAL_A" logcat -d | rg "Handshake|Encrypted|settled" | tail -8 || true
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
        record "CASE5:PASS ring + answer + mid-call re-handshake paths"
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
    adb -s "$CALLEE" logcat -c 2>/dev/null || true
    contact_call "$SERIAL_A" "$CONTACT_BOB" "$SERIAL_B" true || {
      record "CASE6:FAIL contact_call"
      exit 1
    }
    ring_answer_bob "carrier-props callee" "/tmp/saga-inbound-screencaps/case6-bob-ringing.png" || exit 1
    if wait_for_handshake_log "$SERIAL_A" "Playing call_secure exactly once" "secure cue" \
       && check_log "$SERIAL_A" "settled to \[Encrypted\]" \
       && check_log "$SERIAL_B" "CHECKPOINT incoming Connection setActive"; then
      record "CASE6:PASS ring + answer + encrypted with carrier props (WiFi kept for relay)"
    else
      record "CASE6:FAIL"
      adb -s "$SERIAL_A" logcat -d | rg "Saga|Handshake|secure|settled" | tail -12 || true
      adb -s "$SERIAL_B" logcat -d | rg "CHECKPOINT|setActive|secure" | tail -8 || true
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
