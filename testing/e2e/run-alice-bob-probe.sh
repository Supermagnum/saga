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

LISTEN_MARKER='\[Saga Iroh Listen\] endpoint bound and accepting inbound'
RELAY_MARKER='\[Saga Iroh Listen\] relay online'

check_log() { adb -s "$1" logcat -d | rg -q "$2"; }

wait_for_relay_online() {
  local serial="$1" role="${2:-device}" clear_log="${3:-false}" max_attempts="${4:-30}"
  local attempt
  if [[ "$clear_log" == "true" ]]; then
    adb -s "$serial" logcat -c 2>/dev/null || true
  fi
  for attempt in $(seq 1 "$max_attempts"); do
    if check_log "$serial" "$RELAY_MARKER"; then
      echo "RELAY:PASS $role [$serial] relay online (attempt $attempt)"
      return 0
    fi
    sleep 3
  done
  echo "RELAY:WARN $role [$serial] relay not confirmed after $((max_attempts * 3))s" >&2
  return 1
}

push_identity() {
  local serial="$1" contact="$2"
  adb -s "$serial" push "$E2E_DIR/saga_dev_identity_${contact}.xml" /data/local/tmp/id.xml >/dev/null
  adb -s "$serial" shell run-as org.saga mkdir -p shared_prefs
  adb -s "$serial" shell run-as org.saga cp /data/local/tmp/id.xml shared_prefs/saga_dev_identity.xml
}

start_listener() {
  local serial="$1"
  adb -s "$serial" shell am start -a android.intent.action.MAIN \
    -c android.intent.category.LAUNCHER -n org.saga/.ui.MainActivity >/dev/null 2>&1 \
    || adb -s "$serial" shell monkey -p org.saga -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1
  sleep 3
}

ensure_callee_listening() {
  local serial="$1"
  if check_log "$serial" "$LISTEN_MARKER"; then
    echo "CALLEE_LISTEN:PASS $serial already listening (pre-dial)"
    return 0
  fi
  echo "CALLEE_LISTEN:WARN $serial not listening yet — warm-start (no force-stop)"
  start_listener "$serial"
  local attempt
  for attempt in 1 2 3 4 5; do
    if check_log "$serial" "$LISTEN_MARKER"; then
      echo "CALLEE_LISTEN:PASS $serial listening after warm-start (attempt $attempt)"
      return 0
    fi
    sleep 2
  done
  echo "CALLEE_LISTEN:FAIL $serial never bound Iroh endpoint before dial" >&2
  adb -s "$serial" logcat -d | rg "SagaIrohCore|Saga Iroh Listen|Saga Application|dev identity" | tail -15 >&2
  return 1
}

contact_call_from() {
  local caller="$1" contact_name="$2" callee="$3"
  ensure_callee_listening "$callee"
  wait_for_relay_online "$callee" "callee" false || true
  adb -s "$caller" logcat -c
  adb -s "$caller" shell am force-stop org.saga
  sleep 1
  adb -s "$caller" shell am start -a android.intent.action.MAIN \
    -c android.intent.category.LAUNCHER -n org.saga/.ui.MainActivity >/dev/null 2>&1 \
    || adb -s "$caller" shell monkey -p org.saga -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1
  sleep 2
  wait_for_relay_online "$caller" "caller" true 15 || true
  adb -s "$caller" shell am start -n org.saga/.ui.MainActivity \
    -a org.saga.TEST_CONTACT_CALL --es contact_name "$contact_name"
}

wait_for_bob_relay() {
  wait_for_relay_online "$SERIAL_B" "bob" false || true
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
adb -s "$SERIAL_A" push "$E2E_DIR/saga_test_false.xml" /data/local/tmp/saga_test.xml >/dev/null
adb -s "$SERIAL_A" shell run-as org.saga cp /data/local/tmp/saga_test.xml shared_prefs/saga_test.xml

echo "=== Start bob (callee) — cold start, no force-stop after this until probe ends ==="
adb -s "$SERIAL_B" shell am force-stop org.saga
sleep 1
adb -s "$SERIAL_B" logcat -c
start_listener "$SERIAL_B"
ensure_callee_listening "$SERIAL_B"
wait_for_bob_relay "$SERIAL_B"

echo ""
echo "=== Alice calls contact [$CONTACT_BOB] ==="
contact_call_from "$SERIAL_A" "$CONTACT_BOB" "$SERIAL_B"
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
