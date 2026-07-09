#!/usr/bin/env bash
# Focused probe: alice (5554) calls contact "bob" on callee (5556).
# Confirms callee listen-state BEFORE dial, contact resolution (not raw peer label),
# and captures connect/handshake logs. Does not force-stop the callee.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
E2E_DIR="$REPO_ROOT/testing/e2e"
SERIAL_A="${SERIAL_A:-emulator-5554}"
SERIAL_B="${SERIAL_B:-emulator-5556}"
CONTACT_BOB="${CONTACT_BOB:-bob}"
PHONE_BOB="${PHONE_BOB:-+15550100010}"
APK="${APK:-$REPO_ROOT/android/app/build/outputs/apk/debug/app-debug.apk}"
LISTEN_MARKER='\[Saga Iroh Listen\] endpoint bound and accepting inbound'
RELAY_MARKER='\[Saga Iroh Listen\] relay online'

record() { echo "$1"; }

# shellcheck source=e2e-common.sh
source "$E2E_DIR/e2e-common.sh"

push_identity() {
  local serial="$1" contact="$2"
  push_pref "$serial" "$E2E_DIR/saga_dev_identity_${contact}.xml" saga_dev_identity.xml
}

echo "=== Build + install ==="
(cd "$REPO_ROOT/android" && ./gradlew assembleDebug -q)
adb -s "$SERIAL_A" install -r "$APK" >/dev/null
adb -s "$SERIAL_B" install -r "$APK" >/dev/null
for s in "$SERIAL_A" "$SERIAL_B"; do
  adb -s "$s" shell cmd telecom set-phone-account-enabled \
    org.saga/org.saga.telecom.SagaConnectionService saga_iroh 0 2>/dev/null || true
  adb -s "$s" shell pm grant org.saga android.permission.READ_CONTACTS 2>/dev/null || true
  adb -s "$s" shell pm grant org.saga android.permission.WRITE_CONTACTS 2>/dev/null || true
done

echo "=== Seed contacts + provision identities (by contact name) ==="
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_A"
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_B"
push_identity "$SERIAL_A" "alice"
push_identity "$SERIAL_B" "bob"

echo "=== Clear stale test prefs on alice (force_fail must be off) ==="
adb -s "$SERIAL_A" shell run-as org.saga rm -f shared_prefs/saga_test.xml 2>/dev/null || true
push_pref "$SERIAL_A" "$E2E_DIR/saga_test_false.xml" saga_test.xml

echo "=== Start bob (callee) — cold start, no force-stop after this until probe ends ==="
adb -s "$SERIAL_B" shell am force-stop org.saga
sleep 1
adb -s "$SERIAL_B" logcat -c 2>/dev/null || true
start_listener "$SERIAL_B"
ensure_callee_listening "$SERIAL_B" && echo "CALLEE_LISTEN:PASS $SERIAL_B listening before dial" \
  || { echo "CALLEE_LISTEN:FAIL $SERIAL_B never bound Iroh endpoint before dial" >&2; exit 1; }
wait_for_relay_online "$SERIAL_B" "bob (probe baseline)" || exit 1

echo ""
echo "=== Alice calls contact [$CONTACT_BOB] ==="
contact_call "$SERIAL_A" "$CONTACT_BOB" "$SERIAL_B" true
sleep 50

echo ""
echo "=== RESOLVER: alice must dial via contact, not raw peer label ==="
if adb -s "$SERIAL_A" logcat -d | rg -q "Resolved direct Iroh peer"; then
  echo "RESOLVER:FAIL alice used direct peer label — contact resolution bypassed"
else
  echo "RESOLVER:PASS no direct-peer bypass on alice"
fi
if adb -s "$SERIAL_A" logcat -d | rg -q "Resolved contact \[.*\] to Iroh"; then
  echo "RESOLVER:PASS contact -> Iroh resolution seen"
else
  echo "RESOLVER:FAIL contact resolution log missing"
fi

echo ""
echo "=== CONNECT: alice outbound ==="
adb -s "$SERIAL_A" logcat -d | rg "connect_peer|iroh connected|connect failed|Outgoing Iroh|settled to|Playing call_secure|TEST_IROH_CALL rejected" | tail -15

echo ""
echo "=== INBOUND: bob callee ==="
adb -s "$SERIAL_B" logcat -d | rg "Saga Iroh Listen|accepted connection|inbound|Media Round-Trip|MOCK_TOKEN" | tail -15

echo ""
echo "=== VERDICT ==="
PASS=true
check_log "$SERIAL_B" "$LISTEN_MARKER" || { echo "FAIL: bob never listened"; PASS=false; }
check_log "$SERIAL_A" "Resolved contact \[.*\] to Iroh" || { echo "FAIL: contact resolution"; PASS=false; }
if adb -s "$SERIAL_A" logcat -d | rg -q "Resolved direct Iroh peer"; then
  echo "FAIL: harness dialed raw peer label"; PASS=false
fi
if check_log "$SERIAL_A" "settled to \[Encrypted\].*$PHONE_BOB" \
   && check_log "$SERIAL_B" "Saga Media Round-Trip.*decrypted_ok=true"; then
  echo "CALL:PASS alice->bob encrypted + media round-trip"
else
  echo "CALL:FAIL connection or handshake did not complete"
  PASS=false
fi
$PASS && exit 0 || exit 1
