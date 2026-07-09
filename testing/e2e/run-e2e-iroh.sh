#!/usr/bin/env bash
# E2E: Iroh dial path on emulator(s) — dials seeded contacts, not raw peer labels.
#
# SERIAL_A = alice, SERIAL_B = bob listener.
# Example: SERIAL_A=emulator-5554 SERIAL_B=emulator-5556 ./run-e2e-iroh.sh
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
E2E_DIR="$REPO_ROOT/testing/e2e"
SERIAL_A="${SERIAL_A:-emulator-5554}"
SERIAL_B="${SERIAL_B:-}"
CONTACT_BOB="${CONTACT_BOB:-bob}"
PHONE_BOB="${PHONE_BOB:-+15550100010}"

push_identity() {
  local serial="$1" contact="$2"
  adb -s "$serial" push "$E2E_DIR/saga_dev_identity_${contact}.xml" /data/local/tmp/id.xml >/dev/null
  adb -s "$serial" shell run-as org.saga mkdir -p shared_prefs
  adb -s "$serial" shell run-as org.saga cp /data/local/tmp/id.xml shared_prefs/saga_dev_identity.xml
}

push_saga_test_flag() {
  adb -s "$1" push "$2" /data/local/tmp/saga_test.xml >/dev/null
  adb -s "$1" shell run-as org.saga mkdir -p shared_prefs
  adb -s "$1" shell run-as org.saga cp /data/local/tmp/saga_test.xml shared_prefs/saga_test.xml
}

seed_encryption_history() {
  local serial="$1"
  adb -s "$serial" push "$E2E_DIR/saga_encryption_peer.xml" /data/local/tmp/saga_enc.xml >/dev/null
  adb -s "$serial" shell run-as org.saga mkdir -p shared_prefs
  adb -s "$serial" shell run-as org.saga sh -c \
    "cp /data/local/tmp/saga_enc.xml shared_prefs/saga_encryption_established.xml"
}

clear_saga_prefs() {
  adb -s "$1" shell run-as org.saga rm -f \
    shared_prefs/saga_test.xml shared_prefs/saga_encryption_established.xml 2>/dev/null || true
}

restart_saga() {
  adb -s "$1" shell am force-stop org.saga
  sleep 1
  adb -s "$1" shell monkey -p org.saga -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || true
  sleep 2
}

clear_logcat() { adb -s "$1" logcat -c; }

contact_call() {
  local serial="$1" contact_name="$2"
  adb -s "$serial" shell am start -n org.saga/.ui.MainActivity \
    -a org.saga.TEST_CONTACT_CALL --es contact_name "$contact_name"
}

check_log() {
  adb -s "$1" logcat -d | rg "$2"
}

end_call() {
  adb -s "$1" shell input keyevent KEYCODE_ENDCALL 2>/dev/null || true
  adb -s "$1" shell am force-stop org.saga 2>/dev/null || true
  sleep 2
}

echo "=== Provision emulator $SERIAL_A ==="
bash "$E2E_DIR/provision-emulator.sh" "$SERIAL_A"
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_A"

if [[ -n "$SERIAL_B" ]]; then
  echo "=== Provision listener emulator $SERIAL_B (bob) ==="
  bash "$E2E_DIR/provision-emulator.sh" "$SERIAL_B"
  push_identity "$SERIAL_B" "bob"
  bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_B"
  restart_saga "$SERIAL_B"
  sleep 3
fi

prepare_case() {
  clear_saga_prefs "$SERIAL_A"
  push_saga_test_flag "$SERIAL_A" "$1"
  restart_saga "$SERIAL_A"
  if [[ -n "$SERIAL_B" ]]; then
    restart_saga "$SERIAL_B"
    sleep 2
  fi
}

echo ""
echo "=== Case 1: alice calls bob — handshake success + call_secure ==="
prepare_case "$E2E_DIR/saga_test_false.xml"
clear_logcat "$SERIAL_A"
contact_call "$SERIAL_A" "$CONTACT_BOB"
sleep 12
if check_log "$SERIAL_A" "settled to \[Encrypted\].*$PHONE_BOB" \
   && check_log "$SERIAL_A" "Saga Connect Security Cue.*Playing call_secure exactly once"; then
  echo "PASS: Case 1"
else
  echo "FAIL: Case 1"
  adb -s "$SERIAL_A" logcat -d | rg "Saga Handshake|Saga Connect|Iroh Native|Dial Target" | tail -20
  exit 1
fi
end_call "$SERIAL_A"

echo ""
echo "=== Case 3: alice calls bob — forced failure, no prior history ==="
prepare_case "$E2E_DIR/saga_test_true.xml"
clear_logcat "$SERIAL_A"
contact_call "$SERIAL_A" "$CONTACT_BOB"
sleep 12
if check_log "$SERIAL_A" "settled to \[NeverEncrypted\]" \
   && check_log "$SERIAL_A" "Playing call_unsecure exactly once"; then
  echo "PASS: Case 3"
else
  echo "FAIL: Case 3"
  adb -s "$SERIAL_A" logcat -d | rg "Saga Handshake|Saga Connect" | tail -15
  exit 1
fi
end_call "$SERIAL_A"

echo ""
echo "=== Case 4: alice calls bob — downgrade on known contact ==="
clear_saga_prefs "$SERIAL_A"
seed_encryption_history "$SERIAL_A"
push_saga_test_flag "$SERIAL_A" "$E2E_DIR/saga_test_true.xml"
restart_saga "$SERIAL_A"
if [[ -n "$SERIAL_B" ]]; then restart_saga "$SERIAL_B"; sleep 2; fi
clear_logcat "$SERIAL_A"
contact_call "$SERIAL_A" "$CONTACT_BOB"
sleep 12
if check_log "$SERIAL_A" "settled to \[Downgraded\]" \
   && check_log "$SERIAL_A" "Playing call_unsecure exactly once"; then
  echo "PASS: Case 4"
else
  echo "FAIL: Case 4"
  adb -s "$SERIAL_A" logcat -d | rg "Saga Handshake|Saga Connect|Downgrade" | tail -15
  exit 1
fi

echo ""
echo "Cases 1, 3, 4 complete (contact dial path, no SIP)."
