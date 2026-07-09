#!/usr/bin/env bash
# Smoke: cellular tel: call through fixed intent path plays call_unsecure once.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
E2E_DIR="$REPO_ROOT/testing/e2e"
SERIAL="${SERIAL_A:-emulator-5554}"
APK="${APK:-$REPO_ROOT/android/app/build/outputs/apk/debug/app-debug.apk}"
PHONE="${PHONE_PLAIN:-+15550100001}"

bash "$E2E_DIR/provision-emulator.sh" "$SERIAL"
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL"

adb -s "$SERIAL" shell am force-stop org.saga
sleep 1
adb -s "$SERIAL" logcat -c

echo "=== Placing cellular call via ACTION_CALL tel:$PHONE ==="
adb -s "$SERIAL" shell am start -a android.intent.action.CALL -d "tel:$PHONE" -n org.saga/.ui.MainActivity
sleep 8

if adb -s "$SERIAL" logcat -d | rg -q "Playing call_unsecure exactly once"; then
  COUNT=$(adb -s "$SERIAL" logcat -d | rg -c "Playing call_unsecure exactly once" || true)
  if [[ "$COUNT" -eq 1 ]]; then
    echo "PASS: cellular call_unsecure played exactly once"
    exit 0
  fi
  echo "FAIL: call_unsecure played $COUNT times (expected 1)"
  exit 1
fi

echo "FAIL: call_unsecure not found in logcat"
adb -s "$SERIAL" logcat -d | rg "Saga InCall|Saga Connect|Main Activity|Telecom" | tail -20
exit 1
