#!/usr/bin/env bash
# E2E: keyed contact, user-initiated unencrypted dial -> tel:/cellular, call_unsecure, no downgrade log.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
E2E_DIR="$REPO_ROOT/testing/e2e"
SERIAL_A="${SERIAL_A:-emulator-5554}"
SERIAL_B="${SERIAL_B:-emulator-5556}"

# shellcheck source=e2e-common.sh
source "$E2E_DIR/e2e-common.sh"

record() { echo "$1"; }

record "=== Unencrypted keyed contact (alice -> bob) ==="
provision_clean_baseline "$SERIAL_A" "$SERIAL_B"
push_identity "$SERIAL_A" "alice"
push_identity "$SERIAL_B" "bob"
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_A" >/dev/null
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_B" >/dev/null

adb -s "$SERIAL_A" shell am force-stop org.saga
adb -s "$SERIAL_B" shell am force-stop org.saga
sleep 1
adb -s "$SERIAL_A" logcat -c 2>/dev/null || true
adb -s "$SERIAL_B" logcat -c 2>/dev/null || true

adb -s "$SERIAL_A" shell monkey -p org.saga -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || true
sleep 2

adb -s "$SERIAL_A" shell am start -a org.saga.TEST_CONTACT_CALL \
  --es contact_name bob --ez force_unencrypted true \
  -n org.saga/.ui.MainActivity >/dev/null 2>&1 || true
sleep 5

if adb -s "$SERIAL_A" logcat -d | rg -q "Resolved contact \[bob\] to cellular \\(explicit unencrypted\\)" \
   && adb -s "$SERIAL_A" logcat -d | rg -q "Placing cellular call" \
   && adb -s "$SERIAL_A" logcat -d | rg -q "Playing call_unsecure exactly once" \
   && ! adb -s "$SERIAL_A" logcat -d | rg -q "Saga Downgrade Event" \
   && ! adb -s "$SERIAL_B" logcat -d | rg -q "CHECKPOINT addNewIncomingCall|notifyIncomingCall|inbound accepted"; then
  record "PASS: explicit unencrypted keyed contact -> cellular + call_unsecure, no downgrade, no Iroh inbound on callee"
  exit 0
fi

record "FAIL: expected cellular path + call_unsecure without downgrade log"
adb -s "$SERIAL_A" logcat -d | rg "Saga Dial Target|Saga Dialer|call_unsecure|Downgrade" | tail -20 || true
exit 1
