#!/usr/bin/env bash
# Standalone re-verification for inbound/callee cases (2, 2b) after addNewIncomingCall bridge.
# Usage: ./run-case-standalone-inbound.sh 2|2b
set -euo pipefail

CASE="${1:?usage: run-case-standalone-inbound.sh 2|2b}"

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
E2E_DIR="$REPO_ROOT/testing/e2e"
SERIAL_A="${SERIAL_A:-emulator-5554}"
SERIAL_B="${SERIAL_B:-emulator-5556}"
CONTACT_BOB="${CONTACT_BOB:-bob}"
CONTACT_ALICE="${CONTACT_ALICE:-alice}"
PHONE_BOB="${PHONE_BOB:-+15550100010}"
PHONE_ALICE="${PHONE_ALICE:-+15550100011}"
PHONE_BOB_RG='15550100010'
PHONE_ALICE_RG='15550100011'
SCREENCAP_DIR="${SCREENCAP_DIR:-/tmp/saga-inbound-screencaps}"

# shellcheck source=e2e-common.sh
source "$E2E_DIR/e2e-common.sh"

record() { echo "$1"; }

mkdir -p "$SCREENCAP_DIR"

echo "=== Inbound case $CASE standalone — clean boot ==="
provision_clean_baseline "$SERIAL_A" "$SERIAL_B"
for s in "$SERIAL_A" "$SERIAL_B"; do
  adb -s "$s" shell cmd role add-role-holder android.app.role.DIALER org.saga 2>/dev/null || true
done

case "$CASE" in
  2)
    CALLER="$SERIAL_A"
    CALLEE="$SERIAL_B"
    CONTACT="$CONTACT_BOB"
    CALLER_LOOKUP="15550100011"
    CALLER_PHONE_RG='\+15550100010'
    CALLEE_NAME="bob"
    ;;
  2b)
    CALLER="$SERIAL_B"
    CALLEE="$SERIAL_A"
    CONTACT="$CONTACT_ALICE"
    CALLER_LOOKUP="15550100010"
    CALLER_PHONE_RG='\+15550100011'
    CALLEE_NAME="alice"
    ;;
  *)
    echo "Unknown case $CASE (use 2 or 2b)"
    exit 1
    ;;
esac

clear_prefs "$CALLER"
push_pref "$CALLER" "$E2E_DIR/saga_test_false.xml" saga_test.xml
adb -s "$CALLEE" logcat -c 2>/dev/null || true

contact_call "$CALLER" "$CONTACT" "$CALLEE" true || {
  record "CASE${CASE}:FAIL contact_call"
  exit 1
}

if ! wait_for_incoming_ringing "$CALLEE" "$CALLEE_NAME"; then
  record "CASE${CASE}:FAIL callee ringing checkpoints"
  adb -s "$CALLEE" logcat -d | rg "CHECKPOINT|Incoming|InCallService|ConnectionService" | tail -20 || true
  exit 1
fi

CAP="$SCREENCAP_DIR/case${CASE}-callee-ringing.png"
capture_callee_screencap "$CALLEE" "$CAP"
record "CASE${CASE}:INFO screencap saved to $CAP"

answer_incoming "$CALLEE" "$CALLER_LOOKUP"
sleep 2

if wait_for_handshake_log "$CALLER" "settled to \[Encrypted\].*$CALLER_PHONE_RG" "caller encrypted" \
   && check_log "$CALLER" "Playing call_secure exactly once" \
   && check_log "$CALLEE" "Saga Media Round-Trip.*decrypted_ok=true" \
   && check_log "$CALLEE" "CHECKPOINT incoming Connection setActive"; then
  record "CASE${CASE}:PASS inbound ring + answer + encrypted + media"
  exit 0
fi

record "CASE${CASE}:FAIL post-answer handshake/media"
adb -s "$CALLER" logcat -d | rg "Handshake|call_secure|Dial Target" | tail -12 || true
adb -s "$CALLEE" logcat -d | rg "Media|setActive|Handshake" | tail -12 || true
exit 1
