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

# shellcheck source=e2e-common.sh
source "$E2E_DIR/e2e-common.sh"

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
ensure_local_relay
install_both
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_A"
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_B"
push_identity "$SERIAL_A" "alice"
push_identity "$SERIAL_B" "bob"
push_iroh_relay_pref "$SERIAL_A"
push_iroh_relay_pref "$SERIAL_B"
stop_caller "$SERIAL_A"
adb -s "$SERIAL_B" shell am force-stop org.saga
sleep 1
adb -s "$SERIAL_A" logcat -c 2>/dev/null || true
adb -s "$SERIAL_B" logcat -c 2>/dev/null || true
start_listener "$SERIAL_A"
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_B"
ensure_callee_listening "$SERIAL_B" && record "LISTEN:PASS bob listening at suite start" || record "LISTEN:FAIL bob not listening at suite start"
wait_for_relay_online "$SERIAL_B" "bob (suite start)" || true
verify_contacts_seeded "$SERIAL_A" || true
verify_contacts_seeded "$SERIAL_B" || true
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
adb -s "$SERIAL_B" logcat -c 2>/dev/null || true
if contact_call "$SERIAL_A" "$CONTACT_BOB" "$SERIAL_B" true \
   && wait_for_incoming_ringing "$SERIAL_B" "bob" \
   && capture_callee_screencap "$SERIAL_B" "/tmp/saga-case2-bob-ringing.png" \
   && answer_incoming "$SERIAL_B" "15550100011" \
   && wait_for_handshake_log "$SERIAL_A" "settled to \[Encrypted\].*$PHONE_BOB_RG" "encrypted handshake" \
   && check_log "$SERIAL_A" "Playing call_secure exactly once" \
   && check_log "$SERIAL_B" "Saga Media Round-Trip.*decrypted_ok=true" \
   && enc_flag "$SERIAL_A" "$PHONE_BOB"; then
  record "CASE2:PASS alice->bob secure cue + encrypted + inbound ring + media + storage flag"
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
adb -s "$SERIAL_A" logcat -c 2>/dev/null || true
if contact_call "$SERIAL_B" "$CONTACT_ALICE" "$SERIAL_A" true \
   && wait_for_incoming_ringing "$SERIAL_A" "alice" \
   && capture_callee_screencap "$SERIAL_A" "/tmp/saga-case2b-alice-ringing.png" \
   && answer_incoming "$SERIAL_A" "15550100010" \
   && wait_for_handshake_log "$SERIAL_B" "settled to \[Encrypted\].*$PHONE_ALICE_RG" "encrypted handshake" \
   && check_log "$SERIAL_B" "Playing call_secure exactly once" \
   && check_log "$SERIAL_A" "Saga Media Round-Trip.*decrypted_ok=true" \
   && enc_flag "$SERIAL_B" "$PHONE_ALICE"; then
  record "CASE2b:PASS bob->alice secure cue + encrypted + inbound ring + media"
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
if enc_flag "$SERIAL_A" "$PHONE_BOB" 2>/dev/null; then
  record "CASE3:FAIL encryption flag already set before call"
else
  adb -s "$SERIAL_B" logcat -c 2>/dev/null || true
  contact_call "$SERIAL_A" "$CONTACT_BOB" "$SERIAL_B" true || record "CASE3:FAIL contact_call"
  if wait_for_incoming_ringing "$SERIAL_B" "bob (first-trust)" \
     && capture_callee_screencap "$SERIAL_B" "/tmp/saga-case3-bob-ringing.png" \
     && answer_incoming "$SERIAL_B" "15550100011" \
     && wait_for_handshake_log "$SERIAL_A" "settled to \[Encrypted\]" "encrypted handshake" \
     && check_log "$SERIAL_A" "Playing call_secure exactly once" \
     && check_log "$SERIAL_B" "CHECKPOINT incoming Connection setActive" \
     && enc_flag "$SERIAL_A" "$PHONE_BOB"; then
    record "CASE3:PASS first-trust ring + answer + encrypted + flag set"
  else
    record "CASE3:FAIL"
    adb -s "$SERIAL_A" logcat -d | rg "Handshake|encryption|Dial Target" | tail -8 || true
    adb -s "$SERIAL_B" logcat -d | rg "CHECKPOINT|Incoming|setActive" | tail -8 || true
  fi
fi
stop_caller "$SERIAL_A"

echo ""
echo "=== Case 4: Downgrade on known contact (alice calls bob) ==="
clear_prefs "$SERIAL_A"
push_pref "$SERIAL_A" "$E2E_DIR/saga_encryption_peer.xml" saga_encryption_established.xml
push_pref "$SERIAL_B" "$E2E_DIR/saga_encryption_peer_bob.xml" saga_encryption_established.xml
push_pref "$SERIAL_A" "$E2E_DIR/saga_test_true.xml" saga_test.xml
adb -s "$SERIAL_B" logcat -c 2>/dev/null || true
if contact_call "$SERIAL_A" "$CONTACT_BOB" "$SERIAL_B" true \
   && wait_for_incoming_ringing "$SERIAL_B" "bob (downgrade)" \
   && capture_callee_screencap "$SERIAL_B" "/tmp/saga-case4-bob-ringing.png" \
   && answer_incoming "$SERIAL_B" "15550100011" \
   && wait_for_handshake_log "$SERIAL_A" "settled to \[Downgraded\]" "downgrade handshake" \
   && check_log "$SERIAL_A" "Saga Downgrade Event.*recorded downgrade" \
   && check_log "$SERIAL_B" "Handshake settled to \[Downgraded\]"; then
  record "CASE4:PASS ring + answer + downgrade state + local log entry"
else
  record "CASE4:FAIL"
  adb -s "$SERIAL_A" logcat -d | rg "Downgrade|Handshake|settled" | tail -12 || true
  adb -s "$SERIAL_B" logcat -d | rg "CHECKPOINT|Downgrade|setActive|Handshake" | tail -8 || true
fi
stop_caller "$SERIAL_A"

echo ""
echo "=== Case 5: Mid-call rehandshake (alice calls bob) ==="
clear_prefs "$SERIAL_A"
push_pref "$SERIAL_A" "$E2E_DIR/saga_encryption_peer.xml" saga_encryption_established.xml
push_pref "$SERIAL_A" "$E2E_DIR/saga_test_false.xml" saga_test.xml
adb -s "$SERIAL_B" logcat -c 2>/dev/null || true
if contact_call "$SERIAL_A" "$CONTACT_BOB" "$SERIAL_B" true \
   && wait_for_incoming_ringing "$SERIAL_B" "bob (mid-call setup)" \
   && capture_callee_screencap "$SERIAL_B" "/tmp/saga-case5-bob-ringing.png" \
   && answer_incoming "$SERIAL_B" "15550100011" \
   && wait_for_handshake_log "$SERIAL_A" "settled to \[Encrypted\]" "encrypted baseline" \
   && check_log "$SERIAL_B" "CHECKPOINT incoming Connection setActive"; then
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
      adb -s "$SERIAL_A" logcat -d | rg "Mid-Call|Downgrade" | tail -10 || true
    fi
  else
    record "CASE5a:FAIL"
    record "CASE5:FAIL"
  fi
else
  record "CASE5:FAIL could not establish encrypted call for mid-call test"
  adb -s "$SERIAL_A" logcat -d | rg "Handshake|Encrypted|settled" | tail -8 || true
  adb -s "$SERIAL_B" logcat -d | rg "CHECKPOINT|setActive|Incoming" | tail -8 || true
fi
stop_caller "$SERIAL_A"

echo ""
echo "=== Case 6: Encrypted call with carrier props (emulator keeps WiFi for Iroh relay) ==="
adb -s "$SERIAL_A" shell setprop gsm.operator.alpha CarrierA 2>/dev/null || true
adb -s "$SERIAL_B" shell setprop gsm.operator.alpha CarrierB 2>/dev/null || true
clear_prefs "$SERIAL_A"
push_pref "$SERIAL_A" "$E2E_DIR/saga_test_false.xml" saga_test.xml
adb -s "$SERIAL_B" logcat -c 2>/dev/null || true
if contact_call "$SERIAL_A" "$CONTACT_BOB" "$SERIAL_B" true \
   && wait_for_incoming_ringing "$SERIAL_B" "bob (carrier props)" \
   && capture_callee_screencap "$SERIAL_B" "/tmp/saga-case6-bob-ringing.png" \
   && answer_incoming "$SERIAL_B" "15550100011" \
   && wait_for_handshake_log "$SERIAL_A" "Playing call_secure exactly once" "secure cue" \
   && check_log "$SERIAL_A" "settled to \[Encrypted\]" \
   && check_log "$SERIAL_B" "CHECKPOINT incoming Connection setActive"; then
  record "CASE6:PASS ring + answer + encrypted with carrier props (WiFi kept for relay)"
else
  record "CASE6:FAIL"
  adb -s "$SERIAL_A" logcat -d | rg "Saga|Handshake|secure|settled" | tail -10 || true
  adb -s "$SERIAL_B" logcat -d | rg "CHECKPOINT|setActive|secure" | tail -8 || true
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
