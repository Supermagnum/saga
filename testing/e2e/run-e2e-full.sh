#!/usr/bin/env bash
# Full E2E suite (Cases 1-6 + Phase 3). Requires two emulators.
# Dials seeded contacts by display name (bob, alice, thor) only.
# Dev identity XML files are named by contact (saga_dev_identity_bob.xml etc.).
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
E2E_DIR="$REPO_ROOT/testing/e2e"
SERIAL_A="${SERIAL_A:-emulator-5554}"
SERIAL_B="${SERIAL_B:-emulator-5556}"
CONTACT_BOB="${CONTACT_BOB:-bob}"
CONTACT_ALICE="${CONTACT_ALICE:-alice}"
CONTACT_THOR="${CONTACT_THOR:-thor}"
PHONE_BOB="${PHONE_BOB:-+15550100010}"
PHONE_ALICE="${PHONE_ALICE:-+15550100011}"
PHONE_THOR="${PHONE_THOR:-+15550100012}"
# E.164 values contain '+' — escape for ripgrep regex assertions.
PHONE_BOB_RG='\+15550100010'
PHONE_ALICE_RG='\+15550100011'
APK="${APK:-$REPO_ROOT/android/app/build/outputs/apk/debug/app-debug.apk}"
LISTEN_MARKER='\[Saga Iroh Listen\] endpoint bound and accepting inbound'
RELAY_MARKER='\[Saga Iroh Listen\] relay online'

RESULTS=()
record() { RESULTS+=("$1"); echo "$1"; }

push_pref() {
  local serial="$1" localfile="$2" destname="$3"
  adb -s "$serial" push "$localfile" "/data/local/tmp/$destname" >/dev/null
  adb -s "$serial" shell run-as org.saga mkdir -p shared_prefs
  adb -s "$serial" shell run-as org.saga cp "/data/local/tmp/$destname" "shared_prefs/$destname"
}

clear_prefs() {
  adb -s "$1" shell run-as org.saga rm -f \
    shared_prefs/saga_test.xml \
    shared_prefs/saga_encryption_established.xml \
    shared_prefs/saga_downgrade_events.xml 2>/dev/null || true
}

push_identity() {
  local serial="$1" contact_name="$2"
  push_pref "$serial" "$E2E_DIR/saga_dev_identity_${contact_name}.xml" saga_dev_identity.xml
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
    return 0
  fi
  adb -s "$serial" logcat -c 2>/dev/null || true
  start_listener "$serial"
  for _ in 1 2 3 4 5; do
    if check_log "$serial" "$LISTEN_MARKER"; then
      return 0
    fi
    sleep 2
  done
  return 1
}

wait_for_relay_online() {
  local serial="$1" role="${2:-device}" clear_log="${3:-false}" max_attempts="${4:-30}"
  local attempt
  if [[ "$clear_log" == "true" ]]; then
    adb -s "$serial" logcat -c 2>/dev/null || true
  fi
  for attempt in $(seq 1 "$max_attempts"); do
    if check_log "$serial" "$RELAY_MARKER"; then
      return 0
    fi
    sleep 3
  done
  record "RELAY:WARN $role [$serial] relay not confirmed after $((max_attempts * 3))s"
  return 1
}

# Dial contact from caller. Restarts caller only; callee is never force-stopped here.
contact_call() {
  local caller="$1" contact_name="$2" callee="${3:-}"
  local encrypted="${4:-false}"
  if [[ -n "$callee" ]]; then
    ensure_callee_listening "$callee" || {
      record "LISTEN:FAIL callee [$callee] not listening before dial to [$contact_name]"
      return 1
    }
    wait_for_relay_online "$callee" "callee" false || true
  fi
  adb -s "$caller" logcat -c
  adb -s "$caller" shell am force-stop org.saga
  sleep 1
  if [[ "$encrypted" == "true" ]]; then
    adb -s "$caller" shell am start -a android.intent.action.MAIN \
      -c android.intent.category.LAUNCHER -n org.saga/.ui.MainActivity >/dev/null 2>&1 \
      || adb -s "$caller" shell monkey -p org.saga -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1
    sleep 2
    wait_for_relay_online "$caller" "caller" true 15 || true
  fi
  adb -s "$caller" shell am start -n org.saga/.ui.MainActivity \
    -a org.saga.TEST_CONTACT_CALL --es contact_name "$contact_name"
}

stop_caller() {
  adb -s "$1" shell am force-stop org.saga
}

restart() {
  adb -s "$1" shell am force-stop org.saga
  sleep 1
  start_listener "$1"
}

install_both() {
  (cd "$REPO_ROOT/android" && ./gradlew assembleDebug -q)
  for s in "$SERIAL_A" "$SERIAL_B"; do
    adb -s "$s" install -r "$APK" >/dev/null
    adb -s "$s" shell cmd role add-role-holder android.app.role.DIALER org.saga \
      || adb -s "$s" shell pm grant org.saga android.permission.CALL_PHONE
    adb -s "$s" shell pm grant org.saga android.permission.READ_CONTACTS 2>/dev/null || true
    adb -s "$s" shell pm grant org.saga android.permission.WRITE_CONTACTS 2>/dev/null || true
    adb -s "$s" shell cmd telecom set-phone-account-enabled \
      org.saga/org.saga.telecom.SagaConnectionService saga_iroh 0 2>/dev/null || true
  done
}

check_log() {
  adb -s "$1" logcat -d | rg -q "$2"
}

count_log() {
  adb -s "$1" logcat -d | rg -c "$2" || true
}

enc_flag() {
  adb -s "$1" shell run-as org.saga cat shared_prefs/saga_encryption_established.xml 2>/dev/null \
    | rg -Fq "name=\"$2\" value=\"true\""
}

# Dial a seeded contact by display name (bob, alice, thor).

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

asset_distinctness_check() {
  local raw_dir="$REPO_ROOT/android/app/src/main/res/raw"
  local unsecure_md5 midcall_md5 secure_md5
  unsecure_md5="$(md5sum "$raw_dir/call_unsecure.wav" | awk '{print $1}')"
  midcall_md5="$(md5sum "$raw_dir/mid_call_security_warning.wav" | awk '{print $1}')"
  secure_md5="$(md5sum "$raw_dir/call_secure.wav" | awk '{print $1}')"
  if [[ "$unsecure_md5" == "$midcall_md5" ]]; then
    record "ASSETS:FAIL mid_call_security_warning.wav matches call_unsecure.wav"
    return 1
  fi
  if [[ "$secure_md5" == "$unsecure_md5" ]]; then
    record "ASSETS:WARN call_secure matches call_unsecure (unexpected)"
  fi
  record "ASSETS:PASS mid-call tone is distinct from call_unsecure"
}

verify_contacts_seeded() {
  local serial="$1"
  if bash "$E2E_DIR/verify-contact-keys.sh" "$serial" >/dev/null 2>&1; then
    record "SEED:PASS bob/alice/thor keys verified on $serial"
    return 0
  fi
  record "SEED:FAIL contact key verification on $serial"
  return 1
}

echo "=== Build, install, seed contacts, provision identities ==="
install_both
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_A"
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_B"
push_identity "$SERIAL_A" "alice"
push_identity "$SERIAL_B" "bob"
stop_caller "$SERIAL_A"
adb -s "$SERIAL_B" shell am force-stop org.saga
sleep 1
start_listener "$SERIAL_A"
start_listener "$SERIAL_B"
sleep 2
verify_contacts_seeded "$SERIAL_A" || true
verify_contacts_seeded "$SERIAL_B" || true
ensure_callee_listening "$SERIAL_B" && record "LISTEN:PASS bob listening at suite start" || record "LISTEN:FAIL bob not listening at suite start"
wait_for_relay_online "$SERIAL_B" "bob" false && record "RELAY:PASS bob relay online at suite start" || record "RELAY:WARN bob relay not confirmed at suite start"
asset_distinctness_check || true

echo ""
echo "=== Case 1: Unencrypted cellular (alice calls thor, no Saga key) ==="
clear_prefs "$SERIAL_A"
restart "$SERIAL_A"
contact_call "$SERIAL_A" "$CONTACT_THOR"
sleep 8
CUE_COUNT=$(count_log "$SERIAL_A" "Playing call_unsecure exactly once")
if [[ "$CUE_COUNT" -eq 1 ]] && check_log "$SERIAL_A" "origin=\[CELLULAR\]"; then
  record "CASE1:PASS alice->thor cellular call_unsecure once + CELLULAR origin"
else
  record "CASE1:FAIL cue_count=$CUE_COUNT"
  adb -s "$SERIAL_A" logcat -d | rg "Saga Connect|Saga InCall|Dial Target" | tail -8 || true
fi
stop_caller "$SERIAL_A"

echo ""
echo "=== Case 2: Encrypted Iroh (alice calls bob) + media round-trip ==="
clear_prefs "$SERIAL_A"
push_pref "$SERIAL_A" "$E2E_DIR/saga_test_false.xml" saga_test.xml
restart "$SERIAL_A"
sleep 1
contact_call "$SERIAL_A" "$CONTACT_BOB" "$SERIAL_B" true
sleep 45
if check_log "$SERIAL_A" "Playing call_secure exactly once" \
   && check_log "$SERIAL_A" "settled to \[Encrypted\].*$PHONE_BOB_RG" \
   && check_log "$SERIAL_B" "Saga Media Round-Trip.*decrypted_ok=true" \
   && enc_flag "$SERIAL_A" "$PHONE_BOB"; then
  record "CASE2:PASS alice->bob secure cue + encrypted + media + storage flag"
else
  record "CASE2:FAIL"
  adb -s "$SERIAL_A" logcat -d | rg "Saga|Iroh|Media|MOCK_TOKEN|Handshake|Dial Target" | tail -12 || true
  adb -s "$SERIAL_B" logcat -d | rg "Saga|Iroh|Media|MOCK_TOKEN|listener" | tail -8 || true
fi
stop_caller "$SERIAL_A"

echo ""
echo "=== Case 2b: Encrypted Iroh (bob calls alice) ==="
clear_prefs "$SERIAL_B"
push_pref "$SERIAL_B" "$E2E_DIR/saga_test_false.xml" saga_test.xml
restart "$SERIAL_B"
sleep 1
contact_call "$SERIAL_B" "$CONTACT_ALICE" "$SERIAL_A" true
sleep 45
if check_log "$SERIAL_B" "Playing call_secure exactly once" \
   && check_log "$SERIAL_B" "settled to \[Encrypted\].*$PHONE_ALICE_RG" \
   && check_log "$SERIAL_A" "Saga Media Round-Trip.*decrypted_ok=true" \
   && enc_flag "$SERIAL_B" "$PHONE_ALICE"; then
  record "CASE2b:PASS bob->alice secure cue + encrypted + media"
else
  record "CASE2b:FAIL"
  adb -s "$SERIAL_B" logcat -d | rg "Saga|Iroh|Media|Handshake|Dial Target" | tail -12 || true
  adb -s "$SERIAL_A" logcat -d | rg "Saga|Iroh|Media|listener" | tail -8 || true
fi
stop_caller "$SERIAL_B"

echo ""
echo "=== Case 2c: Bob calls thor (no Saga key, cannot secure) ==="
clear_prefs "$SERIAL_B"
push_pref "$SERIAL_B" "$E2E_DIR/saga_test_false.xml" saga_test.xml
restart "$SERIAL_B"
sleep 1
contact_call "$SERIAL_B" "$CONTACT_THOR"
sleep 10
if check_log "$SERIAL_B" "Resolved contact \[thor\] to cellular" \
   && check_log "$SERIAL_B" "origin=\[CELLULAR\]" \
   && check_log "$SERIAL_B" "Playing call_unsecure exactly once"; then
  record "CASE2c:PASS bob->thor cellular (no key, cannot secure)"
else
  record "CASE2c:FAIL"
  adb -s "$SERIAL_B" logcat -d | rg "Dial Target|Saga Connect|origin|settled" | tail -10 || true
fi
stop_caller "$SERIAL_B"

echo ""
echo "=== Case 3: First-time trust (alice calls bob) ==="
clear_prefs "$SERIAL_A"
push_pref "$SERIAL_A" "$E2E_DIR/saga_test_false.xml" saga_test.xml
restart "$SERIAL_A"
sleep 1
if enc_flag "$SERIAL_A" "$PHONE_BOB" 2>/dev/null; then
  record "CASE3:FAIL encryption flag already set before call"
else
  contact_call "$SERIAL_A" "$CONTACT_BOB" "$SERIAL_B" true
  sleep 45
  if check_log "$SERIAL_A" "settled to \[Encrypted\]" && enc_flag "$SERIAL_A" "$PHONE_BOB"; then
    record "CASE3:PASS flag set after alice->bob handshake"
  else
    record "CASE3:FAIL"
    adb -s "$SERIAL_A" logcat -d | rg "Handshake|encryption|Dial Target" | tail -8 || true
  fi
fi
stop_caller "$SERIAL_A"

echo ""
echo "=== Case 4: Downgrade on known contact (alice calls bob) ==="
clear_prefs "$SERIAL_A"
push_pref "$SERIAL_A" "$E2E_DIR/saga_encryption_peer.xml" saga_encryption_established.xml
push_pref "$SERIAL_A" "$E2E_DIR/saga_test_true.xml" saga_test.xml
restart "$SERIAL_A"
sleep 1
contact_call "$SERIAL_A" "$CONTACT_BOB" "$SERIAL_B" true
sleep 40
if check_log "$SERIAL_A" "settled to \[Downgraded\]" \
   && check_log "$SERIAL_A" "Saga Downgrade Event.*recorded downgrade"; then
  record "CASE4:PASS downgrade state + local log entry"
else
  record "CASE4:FAIL"
  adb -s "$SERIAL_A" logcat -d | rg "Downgrade|Handshake" | tail -10 || true
fi
stop_caller "$SERIAL_A"

echo ""
echo "=== Case 5: Mid-call rehandshake (alice calls bob) ==="
clear_prefs "$SERIAL_A"
push_pref "$SERIAL_A" "$E2E_DIR/saga_encryption_peer.xml" saga_encryption_established.xml
push_pref "$SERIAL_A" "$E2E_DIR/saga_test_false.xml" saga_test.xml
restart "$SERIAL_A"
sleep 1
contact_call "$SERIAL_A" "$CONTACT_BOB" "$SERIAL_B" true
sleep 45
if ! check_log "$SERIAL_A" "settled to \[Encrypted\]"; then
  record "CASE5:FAIL could not establish encrypted call for mid-call test"
else
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
      adb -s "$SERIAL_A" logcat -d | rg "Mid-Call|Downgrade" | tail -10 || true
    fi
  else
    record "CASE5a:FAIL"
    record "CASE5:FAIL"
  fi
fi
stop_caller "$SERIAL_A"

echo ""
echo "=== Case 6: Encrypted call with carrier props (emulator keeps WiFi for Iroh relay) ==="
adb -s "$SERIAL_A" shell setprop gsm.operator.alpha CarrierA 2>/dev/null || true
adb -s "$SERIAL_B" shell setprop gsm.operator.alpha CarrierB 2>/dev/null || true
clear_prefs "$SERIAL_A"
push_pref "$SERIAL_A" "$E2E_DIR/saga_test_false.xml" saga_test.xml
restart "$SERIAL_A"
restart "$SERIAL_B"
sleep 2
contact_call "$SERIAL_A" "$CONTACT_BOB" "$SERIAL_B" true
sleep 50
if check_log "$SERIAL_A" "Playing call_secure exactly once" \
   && check_log "$SERIAL_A" "settled to \[Encrypted\]"; then
  record "CASE6:PASS encrypted with carrier props (emulator relay via WiFi)"
else
  record "CASE6:FAIL"
  adb -s "$SERIAL_A" logcat -d | rg "Saga|Handshake|secure" | tail -10 || true
fi
stop_caller "$SERIAL_A"

echo ""
echo "=== Phase 3: Audio cue regression (static) ==="
if rg -n "applyConnectSnapshot|maybePlayConnectCue" "$REPO_ROOT/android/app/src/main/java" \
   | rg -q "SagaInCallActivity|SagaInCallService"; then
  record "PHASE3:PASS connect paths reach cue player"
else
  record "PHASE3:FAIL"
fi

echo ""
echo "=== Summary ==="
printf '%s\n' "${RESULTS[@]}"

FAIL_COUNT=$(printf '%s\n' "${RESULTS[@]}" | rg -c "FAIL" || true)
if [[ "$FAIL_COUNT" -gt 0 ]]; then
  exit 1
fi
