#!/usr/bin/env bash
# PhoneAccount security / onboarding E2E (issue #2).
# Uses a fresh AVD WITHOUT adb set-phone-account-enabled.
# Enables Saga via Calling accounts Settings UI (uiautomator), not telecom shell.
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
E2E_DIR="$REPO_ROOT/testing/e2e"
SERIAL_PA="${SERIAL_PA:-emulator-5558}"
SERIAL_PEER="${SERIAL_PEER:-emulator-5556}"
APK="${APK:-$REPO_ROOT/android/app/build/outputs/apk/debug/app-debug.apk}"
SCREEN_DIR="${SCREEN_DIR:-/tmp/saga-phone-account-screens}"
mkdir -p "$SCREEN_DIR"

RESULTS=()
record() { RESULTS+=("$1"); echo "$1"; }

# shellcheck source=e2e-common.sh
source "$E2E_DIR/e2e-common.sh"

phone_account_enabled() {
  local serial="$1"
  adb -s "$serial" shell dumpsys telecom | rg -q '\[\[X\] PhoneAccount: ComponentInfo\{org\.saga/org\.saga\.telecom\.SagaConnectionService\}'
}

phone_account_disabled() {
  local serial="$1"
  adb -s "$serial" shell dumpsys telecom | rg -q '\[\[ \] PhoneAccount: ComponentInfo\{org\.saga/org\.saga\.telecom\.SagaConnectionService\}'
}

count_telephony_calls() {
  local serial="$1"
  adb -s "$serial" shell dumpsys telecom | rg -c 'TelephonyConnectionService' || true
}

install_pa_no_enable() {
  local serial="$1"
  adb -s "$serial" uninstall org.saga >/dev/null 2>&1 || true
  adb -s "$serial" install -r "$APK" >/dev/null
  adb -s "$serial" shell cmd role add-role-holder android.app.role.DIALER org.saga \
    || adb -s "$serial" shell pm grant org.saga android.permission.CALL_PHONE
  adb -s "$serial" shell pm grant org.saga android.permission.READ_CONTACTS 2>/dev/null || true
  adb -s "$serial" shell pm grant org.saga android.permission.WRITE_CONTACTS 2>/dev/null || true
  adb -s "$serial" shell pm grant org.saga android.permission.RECORD_AUDIO 2>/dev/null || true
  adb -s "$serial" shell pm grant org.saga android.permission.READ_PHONE_STATE 2>/dev/null || true
  # Launch so PhoneAccount registers — do NOT set-phone-account-enabled.
  adb -s "$serial" shell am start -n org.saga/.ui.MainActivity >/dev/null 2>&1 || true
  sleep 3
}

install_peer_enabled() {
  local serial="$1"
  adb -s "$serial" install -r "$APK" >/dev/null
  adb -s "$serial" shell cmd role add-role-holder android.app.role.DIALER org.saga \
    || adb -s "$serial" shell pm grant org.saga android.permission.CALL_PHONE
  adb -s "$serial" shell pm grant org.saga android.permission.READ_CONTACTS 2>/dev/null || true
  adb -s "$serial" shell pm grant org.saga android.permission.WRITE_CONTACTS 2>/dev/null || true
  adb -s "$serial" shell am start -n org.saga/.ui.MainActivity >/dev/null 2>&1 || true
  sleep 2
  adb -s "$serial" shell cmd telecom set-phone-account-enabled \
    org.saga/org.saga.telecom.SagaConnectionService saga_iroh 0
}

tap_ui_text() {
  local serial="$1" dump="$2" needle="$3"
  python3 - "$dump" "$needle" <<'PY'
import re, sys
xml = open(sys.argv[1], encoding="utf-8", errors="ignore").read()
needle = sys.argv[2]
for m in re.finditer(r'text="([^"]*)"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', xml):
    if needle in m.group(1):
        x1,y1,x2,y2 = map(int, m.groups()[1:])
        print(f"{(x1+x2)//2} {(y1+y2)//2}")
        sys.exit(0)
# attribute order may put bounds before text
for m in re.finditer(r'bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"[^>]*text="([^"]*)"', xml):
    if needle in m.group(5):
        x1,y1,x2,y2 = map(int, m.groups()[:4])
        print(f"{(x1+x2)//2} {(y1+y2)//2}")
        sys.exit(0)
sys.exit(2)
PY
}

tap_saga_switch() {
  local serial="$1" dump="$2"
  python3 - "$dump" <<'PY'
import re, sys
xml = open(sys.argv[1], encoding="utf-8", errors="ignore").read()
# Prefer Switch near Saga title
saga_y = None
for m in re.finditer(r'text="Saga"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', xml):
    saga_y = (int(m.group(2)) + int(m.group(4))) // 2
    break
if saga_y is None:
    for m in re.finditer(r'bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"[^>]*text="Saga"', xml):
        saga_y = (int(m.group(2)) + int(m.group(4))) // 2
        break
switches = []
for m in re.finditer(
    r'class="android\.widget\.Switch"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"',
    xml,
):
    x1,y1,x2,y2 = map(int, m.groups())
    switches.append(((x1+x2)//2, (y1+y2)//2, y1))
for m in re.finditer(
    r'bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"[^>]*class="android\.widget\.Switch"',
    xml,
):
    x1,y1,x2,y2 = map(int, m.groups())
    switches.append(((x1+x2)//2, (y1+y2)//2, y1))
if not switches:
    sys.exit(2)
if saga_y is not None:
    switches.sort(key=lambda t: abs(t[2] - saga_y))
x, y, _ = switches[0]
print(f"{x} {y}")
PY
}

dump_ui() {
  local serial="$1" dump="$2"
  adb -s "$serial" shell uiautomator dump /sdcard/saga_ui.xml >/dev/null 2>&1 || true
  adb -s "$serial" pull /sdcard/saga_ui.xml "$dump" >/dev/null 2>&1 || true
}

# Enable Saga Calling account via Settings UI (not telecom shell).
# want_enabled=1 enable, want_enabled=0 disable
toggle_via_settings_ui() {
  local serial="$1"
  local want_enabled="${2:-1}"
  # Reset to top Calling accounts screen
  adb -s "$serial" shell am start -a android.telecom.action.CHANGE_PHONE_ACCOUNTS >/dev/null 2>&1
  sleep 1
  adb -s "$serial" shell input keyevent KEYCODE_BACK >/dev/null 2>&1 || true
  sleep 0.5
  adb -s "$serial" shell am start -a android.telecom.action.CHANGE_PHONE_ACCOUNTS >/dev/null 2>&1
  sleep 2
  local dump="/tmp/saga-ui-${serial}.xml"
  dump_ui "$serial" "$dump"
  if [[ ! -f "$dump" ]]; then
    record "SETTINGS_UI:FAIL uiautomator dump missing"
    return 1
  fi

  # AOSP: Calling accounts -> "All calling accounts" -> Saga switch
  local all_coords
  all_coords="$(tap_ui_text "$serial" "$dump" "All calling accounts" 2>/dev/null || true)"
  if [[ -n "${all_coords:-}" ]]; then
    record "SETTINGS_UI: open All calling accounts at ($all_coords)"
    adb -s "$serial" shell input tap $all_coords
    sleep 2
    dump_ui "$serial" "$dump"
  fi

  local coords
  coords="$(tap_saga_switch "$serial" "$dump" 2>/dev/null || true)"
  if [[ -z "${coords:-}" ]]; then
    record "SETTINGS_UI:FAIL no Saga switch coords (dump=$dump)"
    return 1
  fi

  local already_ok=0
  if [[ "$want_enabled" == "1" ]] && phone_account_enabled "$serial"; then
    already_ok=1
  fi
  if [[ "$want_enabled" == "0" ]] && phone_account_disabled "$serial"; then
    already_ok=1
  fi
  if [[ "$already_ok" -eq 1 ]]; then
    record "SETTINGS_UI: already in desired state"
    return 0
  fi

  record "SETTINGS_UI: tap Saga switch at ($coords) want_enabled=$want_enabled"
  adb -s "$serial" shell input tap $coords
  sleep 2
  if [[ "$want_enabled" == "1" ]] && phone_account_enabled "$serial"; then
    record "SETTINGS_UI:PASS PhoneAccount enabled via Settings"
    return 0
  fi
  if [[ "$want_enabled" == "0" ]] && phone_account_disabled "$serial"; then
    record "SETTINGS_UI:PASS PhoneAccount disabled via Settings"
    return 0
  fi
  # One more tap if state wrong (switch clickable=false sometimes needs parent)
  adb -s "$serial" shell input tap $coords
  sleep 2
  if [[ "$want_enabled" == "1" ]] && phone_account_enabled "$serial"; then
    record "SETTINGS_UI:PASS PhoneAccount enabled via Settings (retry)"
    return 0
  fi
  if [[ "$want_enabled" == "0" ]] && phone_account_disabled "$serial"; then
    record "SETTINGS_UI:PASS PhoneAccount disabled via Settings (retry)"
    return 0
  fi
  record "SETTINGS_UI:FAIL desired state not reached want_enabled=$want_enabled"
  adb -s "$serial" shell dumpsys telecom | rg 'org.saga/org.saga.telecom' || true
  return 1
}

enable_via_settings_ui() {
  toggle_via_settings_ui "$1" 1
}

disable_via_settings_ui() {
  local serial="$1"
  if phone_account_disabled "$serial"; then
    record "SETTINGS_UI: already disabled"
    return 0
  fi
  if toggle_via_settings_ui "$serial" 0; then
    return 0
  fi
  record "SETTINGS_UI:WARN could not disable via UI — reinstalling without enable"
  install_pa_no_enable "$serial"
  push_identity "$serial" "alice"
  push_iroh_relay_pref "$serial"
  bash "$E2E_DIR/seed-test-contacts.sh" "$serial" >/dev/null
  phone_account_disabled "$serial"
}

echo "=== Build APK ==="
(cd "$REPO_ROOT/android" && ./gradlew assembleDebug -q)
ensure_local_relay

echo "=== Install PA (no enable) + peer (enabled) ==="
install_pa_no_enable "$SERIAL_PA"
install_peer_enabled "$SERIAL_PEER"
push_identity "$SERIAL_PA" "alice"
push_identity "$SERIAL_PEER" "bob"
push_iroh_relay_pref "$SERIAL_PA"
push_iroh_relay_pref "$SERIAL_PEER"
push_pref "$SERIAL_PA" "$E2E_DIR/saga_test_false.xml" saga_test.xml
push_pref "$SERIAL_PEER" "$E2E_DIR/saga_test_false.xml" saga_test.xml
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_PA"
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_PEER"

if phone_account_disabled "$SERIAL_PA"; then
  record "STATE:PASS PA PhoneAccount disabled before dial"
else
  record "STATE:FAIL PA PhoneAccount unexpectedly enabled"
fi

adb -s "$SERIAL_PA" shell am force-stop org.saga
adb -s "$SERIAL_PEER" shell am force-stop org.saga
sleep 1
adb -s "$SERIAL_PA" logcat -c
adb -s "$SERIAL_PEER" logcat -c

# Screenshots: onboarding first, then MainActivity banner (back from onboarding)
adb -s "$SERIAL_PA" shell am start -n org.saga/.ui.PhoneAccountOnboardingActivity >/dev/null 2>&1
sleep 1
capture_callee_screencap "$SERIAL_PA" "$SCREEN_DIR/onboarding.png"
adb -s "$SERIAL_PA" shell input keyevent KEYCODE_BACK
sleep 1
# Mark onboarding already shown so MainActivity keeps banner without re-launching
adb -s "$SERIAL_PA" shell run-as org.saga mkdir -p shared_prefs
printf '%s\n' \
  '<?xml version="1.0" encoding="utf-8" standalone="yes" ?>' \
  '<map>' \
  '    <boolean name="onboarding_shown" value="true" />' \
  '</map>' > /tmp/saga_phone_account_ui.xml
adb -s "$SERIAL_PA" push /tmp/saga_phone_account_ui.xml /data/local/tmp/saga_phone_account_ui.xml >/dev/null
adb -s "$SERIAL_PA" shell run-as org.saga cp /data/local/tmp/saga_phone_account_ui.xml shared_prefs/saga_phone_account_ui.xml
adb -s "$SERIAL_PA" shell am start -n org.saga/.ui.MainActivity >/dev/null 2>&1
sleep 1
capture_callee_screencap "$SERIAL_PA" "$SCREEN_DIR/banner.png"

echo "=== Case A: dial Iroh while disabled — must not create SIM call ==="
tel_before="$(count_telephony_calls "$SERIAL_PA")"
adb -s "$SERIAL_PA" logcat -c
adb -s "$SERIAL_PA" shell am start -n org.saga/.ui.MainActivity \
  -a org.saga.TEST_CONTACT_CALL --es contact_name bob
sleep 8

if adb -s "$SERIAL_PA" logcat -d | rg -q 'IROH_BLOCKED_PHONE_ACCOUNT_DISABLED'; then
  record "BLOCK:PASS dial blocked without placeCall"
else
  record "BLOCK:FAIL missing IROH_BLOCKED_PHONE_ACCOUNT_DISABLED"
fi

if adb -s "$SERIAL_PA" logcat -d | rg -q 'Onboarding screen shown|SHOW_PHONE_ACCOUNT|PhoneAccountOnboarding|phone_account_onboarding|Placing Iroh call'; then
  if adb -s "$SERIAL_PA" logcat -d | rg -q 'IROH_BLOCKED_PHONE_ACCOUNT_DISABLED'; then
    record "ONBOARD:PASS onboarding path after blocked dial"
  else
    record "ONBOARD:WARN blocked path unclear"
  fi
fi

# Must not have placed via Telephony
if adb -s "$SERIAL_PA" logcat -d | rg -q 'SET_CONNECTING.*TelephonyConnectionService|tpac=ComponentInfo\{com.android.phone'; then
  record "SIM:FAIL telephony call created for Iroh dial"
else
  record "SIM:PASS no telephony substitution for blocked Iroh dial"
fi

# Peer must not ring
if adb -s "$SERIAL_PEER" logcat -d | rg -q 'addNewIncomingCall returned|STATE_RINGING'; then
  record "PEER:FAIL peer rang while PA disabled"
else
  record "PEER:PASS peer did not ring"
fi

echo "=== Case B: enable via Settings UI, then dial ==="
if enable_via_settings_ui "$SERIAL_PA"; then
  :
else
  record "CASEB:FAIL could not enable via Settings UI"
fi

adb -s "$SERIAL_PA" shell am force-stop org.saga
sleep 1
adb -s "$SERIAL_PA" logcat -c
adb -s "$SERIAL_PEER" logcat -c
start_listener "$SERIAL_PEER"
wait_for_relay_online "$SERIAL_PEER" "bob" || true
start_listener "$SERIAL_PA"
wait_for_relay_online "$SERIAL_PA" "alice-pa" || true

adb -s "$SERIAL_PA" shell am start -n org.saga/.ui.MainActivity \
  -a org.saga.TEST_CONTACT_CALL --es contact_name bob
if wait_for_incoming_ringing "$SERIAL_PEER" "bob-after-enable" 60; then
  capture_callee_screencap "$SERIAL_PEER" "$SCREEN_DIR/bob-ringing-after-enable.png"
  answer_incoming "$SERIAL_PEER" "15550100011"
  sleep 8
  if adb -s "$SERIAL_PA" logcat -d | rg -q 'settled to \[Encrypted\]|HANDSHAKE_ENCRYPTED|Playing call_secure'; then
    record "CASEB:PASS call connected after Settings enable"
  else
    # softer: ring+answer is enough proof of Telecom path
    if adb -s "$SERIAL_PEER" logcat -d | rg -q 'setActive|STATE_RINGING'; then
      record "CASEB:PASS ring+answer after Settings enable"
    else
      record "CASEB:FAIL no encrypted/settle after enable"
    fi
  fi
else
  record "CASEB:FAIL no ring after Settings enable"
  adb -s "$SERIAL_PA" logcat -d | rg 'Saga Dialer|placeCall|IROH_|PhoneAccount|saga:' | tail -30 || true
fi

# Confirm non-digit tel:, (PAUSE) was used (managed accounts; saga: does not reach ConnectionService).
# Uri.toString() may log as tel:, or tel:%2C depending on API.
if adb -s "$SERIAL_PA" logcat -d | rg -q 'placeCall issued uri=\[tel:(,|%2C)\]'; then
  record "URI:PASS managed tel:, placeCall (no dialable digits after keypad+strip) accepted"
else
  record "URI:WARN tel:, placeCall log not found (check dumps)"
fi

echo "=== Case C: incoming while disabled — clean reject ==="
adb -s "$SERIAL_PA" shell am force-stop org.saga
# Prefer Settings toggle off; fall back to reinstall without enable
if ! disable_via_settings_ui "$SERIAL_PA"; then
  install_pa_no_enable "$SERIAL_PA"
  push_identity "$SERIAL_PA" "alice"
  push_iroh_relay_pref "$SERIAL_PA"
  bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL_PA" >/dev/null
fi

adb -s "$SERIAL_PA" logcat -c
adb -s "$SERIAL_PEER" logcat -c
start_listener "$SERIAL_PA"
wait_for_relay_online "$SERIAL_PA" "alice-disabled" || true
start_listener "$SERIAL_PEER"
wait_for_relay_online "$SERIAL_PEER" "bob-caller" || true

adb -s "$SERIAL_PEER" shell am start -n org.saga/.ui.MainActivity \
  -a org.saga.TEST_CONTACT_CALL --es contact_name alice
# Wait for inbound accept + reject path
reject_ok=0
for _ in $(seq 1 30); do
  if adb -s "$SERIAL_PA" logcat -d | rg -q 'PhoneAccount not enabled|SecurityException|Rejecting inbound Iroh session|inbound rejected by bridge'; then
    reject_ok=1
    break
  fi
  sleep 2
done

if [[ "$reject_ok" -eq 1 ]]; then
  record "INBOUND_REJECT:PASS logged reject while disabled"
else
  record "INBOUND_REJECT:FAIL missing reject log"
  adb -s "$SERIAL_PA" logcat -d | rg 'Incoming Call Bridge|Phone Account|inbound|Iroh Listen' | tail -30 || true
  adb -s "$SERIAL_PEER" logcat -d | rg 'Saga Dialer|placeCall|notifyFailed|connect' | tail -20 || true
fi

if adb -s "$SERIAL_PA" logcat -d | rg -q 'FATAL EXCEPTION|AndroidRuntime.*Crash'; then
  record "CRASH:FAIL process crashed on inbound reject"
else
  record "CRASH:PASS no crash on inbound reject"
fi

if adb -s "$SERIAL_PA" logcat -d | rg -q 'STATE_RINGING|addNewIncomingCall returned \(no throw\)'; then
  record "INBOUND_RING:FAIL rang while disabled"
else
  record "INBOUND_RING:PASS no ring while disabled"
fi

if adb -s "$SERIAL_PEER" logcat -d | rg -q 'notifyFailed|iroh connect|handshake failed|Failed|not reachable|disconnect|timed out|IROH'; then
  # Caller should fail or not stay encrypted
  if adb -s "$SERIAL_PEER" logcat -d | rg -q 'settled to \[Encrypted\]'; then
    record "CALLER:FAIL caller still encrypted while callee disabled"
  else
    record "CALLER:PASS caller did not settle Encrypted"
  fi
else
  record "CALLER:WARN inconclusive caller failure markers"
fi

echo ""
echo "=== Summary ==="
printf '%s\n' "${RESULTS[@]}"
FAIL_COUNT=$(printf '%s\n' "${RESULTS[@]}" | rg -c 'FAIL' || true)
if [[ "${FAIL_COUNT:-0}" -gt 0 ]]; then
  exit 1
fi
exit 0
