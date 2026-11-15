#!/usr/bin/env bash
# DEBUG forensics: force SIM substitution with a chosen tel SSP; capture RIL/modem dial digits.
# Requires debuggable APK, PhoneAccount DISABLED, dialer role held.
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
E2E_DIR="$REPO_ROOT/testing/e2e"
SERIAL="${SERIAL:-emulator-5558}"
SSP="${SSP:-iroh}"
APK="${APK:-$REPO_ROOT/android/app/build/outputs/apk/debug/app-debug.apk}"
CONTACT="${CONTACT:-bob}"

RESULTS=()
record() { RESULTS+=("$1"); echo "$1"; }

# shellcheck source=e2e-common.sh
source "$E2E_DIR/e2e-common.sh"

ensure_disabled() {
  if adb -s "$SERIAL" shell dumpsys telecom | rg -q '\[\[X\] PhoneAccount: ComponentInfo\{org\.saga/org\.saga\.telecom\.SagaConnectionService\}'; then
    # Toggle off via Settings UI helper from onboarding script if present
    adb -s "$SERIAL" shell am start -a android.telecom.action.CHANGE_PHONE_ACCOUNTS >/dev/null 2>&1
    sleep 1
    adb -s "$SERIAL" shell input keyevent KEYCODE_BACK >/dev/null 2>&1 || true
    sleep 0.3
    adb -s "$SERIAL" shell am start -a android.telecom.action.CHANGE_PHONE_ACCOUNTS >/dev/null 2>&1
    sleep 1
    local dump="/tmp/modem-ui.xml"
    adb -s "$SERIAL" shell uiautomator dump /sdcard/saga_ui.xml >/dev/null 2>&1 || true
    adb -s "$SERIAL" pull /sdcard/saga_ui.xml "$dump" >/dev/null 2>&1 || true
    local all
    all="$(python3 - "$dump" <<'PY'
import re,sys
xml=open(sys.argv[1],encoding='utf-8',errors='ignore').read()
for m in re.finditer(r'text="All calling accounts"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"',xml):
  x1,y1,x2,y2=map(int,m.groups()); print(f'{(x1+x2)//2} {(y1+y2)//2}'); break
else:
  for m in re.finditer(r'bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"[^>]*text="All calling accounts"',xml):
    x1,y1,x2,y2=map(int,m.groups()); print(f'{(x1+x2)//2} {(y1+y2)//2}'); break
PY
)" || true
    if [[ -n "${all:-}" ]]; then
      adb -s "$SERIAL" shell input tap $all
      sleep 1
      adb -s "$SERIAL" shell uiautomator dump /sdcard/saga_ui.xml >/dev/null 2>&1 || true
      adb -s "$SERIAL" pull /sdcard/saga_ui.xml "$dump" >/dev/null 2>&1 || true
    fi
    local sw
    sw="$(python3 - "$dump" <<'PY'
import re,sys
xml=open(sys.argv[1],encoding='utf-8',errors='ignore').read()
for m in re.finditer(r'class="android\.widget\.Switch"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"',xml):
  x1,y1,x2,y2=map(int,m.groups()); print(f'{(x1+x2)//2} {(y1+y2)//2}'); break
for m in re.finditer(r'bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"[^>]*class="android\.widget\.Switch"',xml):
  x1,y1,x2,y2=map(int,m.groups()); print(f'{(x1+x2)//2} {(y1+y2)//2}'); break
PY
)" || true
    if [[ -n "${sw:-}" ]]; then
      adb -s "$SERIAL" shell input tap $sw
      sleep 1
    fi
  fi
  if adb -s "$SERIAL" shell dumpsys telecom | rg -q '\[\[ \] PhoneAccount: ComponentInfo\{org\.saga/org\.saga\.telecom\.SagaConnectionService\}'; then
    record "ACCOUNT: disabled"
  else
    record "ACCOUNT:WARN still enabled — force path may not substitute SIM"
  fi
}

echo "=== Modem SSP forensics ssp=[$SSP] serial=[$SERIAL] ==="
adb -s "$SERIAL" install -r "$APK" >/dev/null
adb -s "$SERIAL" shell cmd role add-role-holder android.app.role.DIALER org.saga 2>/dev/null || true
adb -s "$SERIAL" shell pm grant org.saga android.permission.CALL_PHONE 2>/dev/null || true
adb -s "$SERIAL" shell pm grant org.saga android.permission.READ_CONTACTS 2>/dev/null || true
adb -s "$SERIAL" shell pm grant org.saga android.permission.WRITE_CONTACTS 2>/dev/null || true
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL" >/dev/null
push_identity "$SERIAL" "alice"
push_iroh_relay_pref "$SERIAL"
ensure_disabled

adb -s "$SERIAL" shell am force-stop org.saga
sleep 1
adb -s "$SERIAL" logcat -c
# Also clear radio buffer if present
adb -s "$SERIAL" logcat -b radio -c 2>/dev/null || true

adb -s "$SERIAL" shell am start -n org.saga/.ui.MainActivity >/dev/null 2>&1
sleep 2

# Expected keypad conversion (host-side mirror of PhoneNumberUtils letter map)
EXPECTED="$(python3 - "$SSP" <<'PY'
import sys
ssp=sys.argv[1]
keymap={
'a':'2','b':'2','c':'2','d':'3','e':'3','f':'3',
'g':'4','h':'4','i':'4','j':'5','k':'5','l':'5',
'm':'6','n':'6','o':'6','p':'7','q':'7','r':'7','s':'7',
't':'8','u':'8','v':'8','w':'9','x':'9','y':'9','z':'9'
}
converted=''.join(keymap.get(c.lower(),c) for c in ssp)
# stripSeparators-ish: keep digits * # + , ; N W
keep=set('0123456789*#+,;NnWw')
digits=''.join(c for c in converted if c in keep)
print(digits)
PY
)"
record "EXPECT: ssp=[$SSP] afterKeypad+strip≈[$EXPECTED]"

T0_MS="$(date +%s%3N)"
adb -s "$SERIAL" shell am start -n org.saga/.ui.MainActivity \
  -a org.saga.TEST_FORCE_IROH_SIM_PATH \
  --es contact_name "$CONTACT" \
  --es ssp "$SSP" >/dev/null 2>&1
sleep 8
T1_MS="$(date +%s%3N)"

LOG_MAIN="/tmp/saga-modem-main-${SSP//[^a-zA-Z0-9]/_}.log"
LOG_RADIO="/tmp/saga-modem-radio-${SSP//[^a-zA-Z0-9]/_}.log"
adb -s "$SERIAL" logcat -d > "$LOG_MAIN"
adb -s "$SERIAL" logcat -b radio -d > "$LOG_RADIO" 2>/dev/null || true

echo "=== App markers ==="
rg -n 'DEBUG_FORCE_SIM_PATH|IROH_ABORT_SIM_SUBSTITUTION|placeCall issued|TelephonyConnectionService|SET_CONNECTING|afterKeypad' "$LOG_MAIN" | head -40 || true

echo "=== Dial / RIL / modem candidates (main) ==="
rg -ni 'dial|rilj|ATD|AT\+|gsmCall|OutgoingCall|number=|handle=tel:|requestDial|CLCC|4764|\*#' "$LOG_MAIN" | head -60 || true

echo "=== Radio buffer ==="
rg -ni 'dial|ATD|number|ril|gsm|call|4764|\*#' "$LOG_RADIO" | head -60 || true

# Extract likely dialed number strings
MODEM_DIGITS="$(
  {
    rg -o 'ATD[^;\r\n]+' "$LOG_RADIO" 2>/dev/null || true
    rg -o 'ATD[^;\r\n]+' "$LOG_MAIN" 2>/dev/null || true
    rg -oN 'requestDial[^\\n]*' "$LOG_MAIN" 2>/dev/null || true
    rg -oN 'dialing (number )?[^ \\n]+' "$LOG_MAIN" 2>/dev/null || true
    rg -oN 'tel:[^ \\]\\n,]+' "$LOG_MAIN" 2>/dev/null || true
    rg -oN 'number=[+0-9*#]+' "$LOG_MAIN" 2>/dev/null || true
    rg -oN 'handle=tel:[^ \\]\\n,]+' "$LOG_MAIN" 2>/dev/null || true
  } | head -40
)"
record "MODEM_CANDIDATES:"
while IFS= read -r line; do
  [[ -n "$line" ]] && record "  $line"
done <<< "$MODEM_DIGITS"

ABORT_LINE="$(rg -n 'IROH_ABORT_SIM_SUBSTITUTION' "$LOG_MAIN" | head -1 || true)"
PLACE_LINE="$(rg -n 'DEBUG_FORCE_SIM_PATH placeCallAtElapsed' "$LOG_MAIN" | head -1 || true)"
record "PLACE_LOG: ${PLACE_LINE:-none}"
record "ABORT_LOG: ${ABORT_LINE:-none}"

# dumpsys for active calls
record "DUMPSYS:"
adb -s "$SERIAL" shell dumpsys telecom | rg -n 'Call id=|tpac=|handle=|state=|Telephony|org.saga' | head -40 || true

echo ""
echo "=== Summary ssp=[$SSP] wall_ms=$((T1_MS-T0_MS)) ==="
printf '%s\n' "${RESULTS[@]}"
