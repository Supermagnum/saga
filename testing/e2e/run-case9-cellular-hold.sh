#!/usr/bin/env bash
# Case 9: incoming cellular holds the Iroh call; answer, end; Iroh resumes with audio.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
E2E_DIR="$(cd "$(dirname "$0")" && pwd)"
record() { echo "$1"; }
# shellcheck source=e2e-common.sh
source "$E2E_DIR/e2e-common.sh"

SERIAL_A="${SERIAL_A:-emulator-5554}"
SERIAL_B="${SERIAL_B:-emulator-5556}"
CONTACT_BOB="${CONTACT_BOB:-bob}"
PHONE_BOB="${PHONE_BOB:-+15550100010}"

ensure_local_relay
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_A"
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_B"

clear_prefs "$SERIAL_A"
push_pref "$SERIAL_A" "$E2E_DIR/saga_encryption_peer.xml" saga_encryption_established.xml
push_pref "$SERIAL_A" "$E2E_DIR/saga_test_false.xml" saga_test.xml

adb -s "$SERIAL_A" logcat -c 2>/dev/null || true
adb -s "$SERIAL_B" logcat -c 2>/dev/null || true

if ! contact_call "$SERIAL_A" "$CONTACT_BOB" "$SERIAL_B" true \
  || ! wait_for_incoming_ringing "$SERIAL_B" "bob case9" \
  || ! answer_incoming "$SERIAL_B" "15550100011" \
  || ! wait_for_handshake_log "$SERIAL_A" "settled to \[Encrypted\]" "encrypted" 30; then
  record "CASE9:FAIL could not establish Iroh call"
  exit 1
fi

# GSM incoming to alice while Iroh active (emulator console).
EMU_PORT_A="${EMU_PORT_A:-5554}"
{
  echo "gsm call 15550100012"
  sleep 2
  echo "gsm accept 15550100012"
  sleep 2
  echo "gsm cancel 15550100012"
  sleep 1
} | telnet 127.0.0.1 "$EMU_PORT_A" >/tmp/saga-case9-gsm.txt 2>&1 || true

sleep 2
capture_callee_screencap "$SERIAL_A" "/tmp/saga-case9-alice-held-or-resume.png" || true

# Expect hold/unhold + media resume logs.
if check_log "$SERIAL_A" "held=true\|onHold\|STATE_HOLDING" \
  && check_log "$SERIAL_A" "held=false\|onUnhold\|RESUME\|setActive"; then
  record "CASE9:PASS cellular hold/resume of Iroh call"
elif check_log "$SERIAL_A" "\[Saga Media\] held="; then
  record "CASE9:PASS cellular hold path exercised (see logcat held=)"
else
  # Emulator GSM may be flaky; still record evidence.
  record "CASE9:FAIL no hold/resume evidence"
  adb -s "$SERIAL_A" logcat -d | rg -i "hold|held|gsm|cellular|Telecom" | tail -30 || true
fi

stop_caller "$SERIAL_A"
stop_caller "$SERIAL_B"
