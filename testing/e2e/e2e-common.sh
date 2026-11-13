# Shared E2E helpers — source from run-e2e-full.sh / run-case-standalone.sh
# Relay gating: poll native endpoint.online() completion via JNI (relay_status.txt),
# not logcat markers. After force-stop, clear logcat once to discard stale lines only.

RELAY_POLL_INTERVAL_SECS="${RELAY_POLL_INTERVAL_SECS:-3}"
RELAY_POLL_MAX_SECS="${RELAY_POLL_MAX_SECS:-120}"
LISTEN_POLL_MAX_SECS="${LISTEN_POLL_MAX_SECS:-30}"
HANDSHAKE_POLL_MAX_SECS="${HANDSHAKE_POLL_MAX_SECS:-120}"
SAGA_IROH_RELAY_MODE="${SAGA_IROH_RELAY_MODE:-local}"
# Prefer LAN IP shared by host saga-peer + emulators (no 127.0.0.1 / 10.0.2.2 ads).
SAGA_IROH_RELAY_URL="${SAGA_IROH_RELAY_URL:-}"
SAGA_RELAY_RETRY_ON_FAIL="${SAGA_RELAY_RETRY_ON_FAIL:-0}"

resolve_lan_relay_url() {
  local e2e_dir="${E2E_DIR:-$(dirname "${BASH_SOURCE[0]}")}"
  local repo_root="${REPO_ROOT:-$(cd "$e2e_dir/../.." && pwd)}"
  if [[ -n "${SAGA_IROH_RELAY_URL:-}" ]]; then
    echo "$SAGA_IROH_RELAY_URL"
    return 0
  fi
  if [[ -f /tmp/saga-iroh-relay-url.txt ]]; then
    local cached
    cached="$(tr -d '\r\n' </tmp/saga-iroh-relay-url.txt)"
    if [[ -n "$cached" ]]; then
      echo "$cached"
      return 0
    fi
  fi
  bash "$repo_root/testing/iroh-relay/lan-relay-url.sh"
}

LISTEN_MARKER="${LISTEN_MARKER:-\[Saga Iroh Listen\] endpoint bound and accepting inbound}"
RELAY_MARKER="${RELAY_MARKER:-\[Saga Iroh Listen\] relay online}"

RELAY_READY_NATIVE=1
RELAY_PENDING_NATIVE=0
RELAY_FAILED_NATIVE=2

# Bound every logcat dump — API 29 emulators can hang forever on bare `adb logcat -d`.
ADB_LOGCAT_TIMEOUT_SECS="${ADB_LOGCAT_TIMEOUT_SECS:-8}"

adb_logcat_dump() {
  local serial="$1"
  timeout "$ADB_LOGCAT_TIMEOUT_SECS" adb -s "$serial" logcat -d 2>/dev/null || true
}

check_log() {
  adb_logcat_dump "$1" | rg -q "$2"
}

saga_process_running() {
  adb -s "$1" shell pidof org.saga 2>/dev/null | rg -q '[0-9]'
}

ensure_local_relay() {
  if [[ "$SAGA_IROH_RELAY_MODE" == "public" ]]; then
    record "RELAY:INFO using public N0 relays (no local relay)"
    return 0
  fi
  local repo_root="${REPO_ROOT:-$(cd "$E2E_DIR/../.." && pwd)}"
  bash "$repo_root/testing/iroh-relay/start-local-relay.sh"
  SAGA_IROH_RELAY_URL="$(resolve_lan_relay_url)"
  export SAGA_IROH_RELAY_URL
  record "RELAY:INFO local relay at $SAGA_IROH_RELAY_URL"
}

push_iroh_relay_pref() {
  local serial="$1"
  if [[ "$SAGA_IROH_RELAY_MODE" == "public" ]]; then
    adb -s "$serial" shell run-as org.saga rm -f shared_prefs/saga_iroh_relay.xml 2>/dev/null || true
    return 0
  fi
  if [[ -z "${SAGA_IROH_RELAY_URL:-}" ]]; then
    SAGA_IROH_RELAY_URL="$(resolve_lan_relay_url)"
    export SAGA_IROH_RELAY_URL
  fi
  # Generate prefs XML with the LAN advertise URL (never 127.0.0.1 / 10.0.2.2).
  local gen="/tmp/saga_iroh_relay_local_gen.xml"
  cat >"$gen" <<EOF
<?xml version='1.0' encoding='utf-8' standalone='yes' ?>
<map>
    <string name="relay_url">${SAGA_IROH_RELAY_URL}</string>
</map>
EOF
  push_pref "$serial" "$gen" saga_iroh_relay.xml
}

query_relay_ready_native() {
  local serial="$1"
  adb -s "$serial" shell am start -a org.saga.TEST_RELAY_QUERY \
    -n org.saga/.ui.MainActivity >/dev/null 2>&1 || true
  sleep 0.5
  # API 29 emulators can hang forever on bare `run-as cat`; bound the read.
  timeout 5 adb -s "$serial" shell run-as org.saga cat "files/${RELAY_STATUS_FILE:-relay_status.txt}" 2>/dev/null \
    | tr -d '\r\n' || true
}

poll_for_relay_native() {
  local serial="$1" label="$2"
  local max_secs="${3:-$RELAY_POLL_MAX_SECS}"
  local interval="${4:-$RELAY_POLL_INTERVAL_SECS}"
  local max_attempts=$((max_secs / interval))
  local attempt status
  for attempt in $(seq 1 "$max_attempts"); do
    if ! saga_process_running "$serial"; then
      sleep "$interval"
      continue
    fi
    status="$(query_relay_ready_native "$serial")"
    if [[ "$status" == "$RELAY_READY_NATIVE" ]]; then
      return 0
    fi
    if [[ "$status" == "$RELAY_FAILED_NATIVE" ]]; then
      record "RELAY:FAIL $label [$serial] native poll reported failed"
      return 1
    fi
    sleep "$interval"
  done
  return 1
}

poll_for_log_marker() {
  local serial="$1" pattern="$2" label="$3"
  local max_secs="${4:-$RELAY_POLL_MAX_SECS}"
  local interval="${5:-$RELAY_POLL_INTERVAL_SECS}"
  local max_attempts=$((max_secs / interval))
  local attempt
  for attempt in $(seq 1 "$max_attempts"); do
    if check_log "$serial" "$pattern"; then
      return 0
    fi
    sleep "$interval"
  done
  return 1
}

wait_for_relay_online() {
  local serial="$1" role="${2:-device}"
  if ! saga_process_running "$serial"; then
    record "RELAY:FAIL $role [$serial] Saga not running"
    return 1
  fi
  if poll_for_relay_native "$serial" "$role" "$RELAY_POLL_MAX_SECS"; then
    record "RELAY:PASS $role [$serial] relay online (native)"
    return 0
  fi
  if [[ "$SAGA_RELAY_RETRY_ON_FAIL" == "1" ]]; then
    record "RELAY:RETRY $role [$serial] one warm restart after native timeout"
    adb -s "$serial" shell am force-stop org.saga
    sleep 1
    adb -s "$serial" logcat -c 2>/dev/null || true
    start_listener "$serial"
    if poll_for_relay_native "$serial" "$role" "$RELAY_POLL_MAX_SECS"; then
      record "RELAY:PASS $role [$serial] relay online after retry (native)"
      return 0
    fi
  fi
  local native_status logcat_relay
  native_status="$(query_relay_ready_native "$serial")"
  logcat_relay="$(adb_logcat_dump "$serial" | rg -c '\[Saga Iroh Listen\] relay online' || true)"
  record "RELAY:FAIL $role [$serial] native=$native_status logcat_relay_lines=$logcat_relay after ${RELAY_POLL_MAX_SECS}s"
  return 1
}

# Use after force-stop: discard stale listen/relay lines from the previous process, then warm-start.
wait_for_fresh_relay_online() {
  local serial="$1" role="${2:-device}"
  adb -s "$serial" logcat -c 2>/dev/null || true
  start_listener "$serial"
  wait_for_relay_online "$serial" "$role"
}

# Wall-clock ms from first mock-token InitMessage to Handshake settled Encrypted
# (includes confirmation MACs). Prints ms or empty. Spec §8: 2 s target / 4 s hard.
measure_handshake_ms() {
  local serial="$1"
  local tmp
  tmp="$(mktemp)"
  adb_logcat_dump "$serial" >"$tmp" || true
  python3 - "$tmp" <<'PY'
import re, sys
from datetime import datetime
path = sys.argv[1]
start = end = None
pat = re.compile(r"^(\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3})")
with open(path, encoding="utf-8", errors="ignore") as f:
    for line in f:
        m = pat.match(line)
        if not m:
            continue
        ts = datetime.strptime(m.group(1), "%m-%d %H:%M:%S.%f")
        if start is None and "sending InitMessage" in line:
            start = ts
        if "Handshake settled to [Encrypted]" in line:
            end = ts
if start and end and end >= start:
    print(int((end - start).total_seconds() * 1000))
PY
  rm -f "$tmp"
}

record_handshake_budget() {
  local serial="$1" label="$2"
  local ms
  ms="$(measure_handshake_ms "$serial" || true)"
  if [[ -z "$ms" ]]; then
    record "HANDSHAKE_TIME:FAIL $label (could not parse InitMessage→Encrypted)"
    return 1
  fi
  if (( ms <= 2000 )); then
    record "HANDSHAKE_TIME:PASS $label ${ms}ms (≤2s target, ≤4s hard)"
  elif (( ms <= 4000 )); then
    record "HANDSHAKE_TIME:PASS $label ${ms}ms (within 4s hard; over 2s soft target)"
  else
    record "HANDSHAKE_TIME:FAIL $label ${ms}ms (over 4s hard budget)"
    return 1
  fi
}

wait_for_handshake_log() {
  local serial="$1" pattern="$2" label="${3:-handshake}"
  local max_secs="${4:-$HANDSHAKE_POLL_MAX_SECS}"
  if poll_for_log_marker "$serial" "$pattern" "$label" "$max_secs"; then
    return 0
  fi
  record "HANDSHAKE:FAIL $label not seen on [$serial] after ${max_secs}s"
  return 1
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
  if saga_process_running "$serial" && check_log "$serial" "$LISTEN_MARKER"; then
    return 0
  fi
  if saga_process_running "$serial"; then
    adb -s "$serial" shell am force-stop org.saga
    sleep 1
  fi
  adb -s "$serial" logcat -c 2>/dev/null || true
  start_listener "$serial"
  if poll_for_log_marker "$serial" "$LISTEN_MARKER" "listen" "$LISTEN_POLL_MAX_SECS"; then
    return 0
  fi
  return 1
}

# Force-stop caller, clear stale relay lines, warm-start, poll until relay-online, then clear for dial logs.
warm_encrypted_caller() {
  local serial="$1"
  adb -s "$serial" shell am force-stop org.saga
  sleep 1
  adb -s "$serial" logcat -c 2>/dev/null || true
  start_listener "$serial"
  wait_for_relay_online "$serial" "caller"
}

# Callee is never force-stopped here. Caller force-stop only inside warm_encrypted_caller when encrypted.
contact_call() {
  local caller="$1" contact_name="$2" callee="${3:-}"
  local encrypted="${4:-false}"
  clear_stuck_telephony_calls "$caller"
  if [[ -n "$callee" ]]; then
    clear_stuck_telephony_calls "$callee"
    ensure_callee_listening "$callee" || {
      record "LISTEN:FAIL callee [$callee] not listening before dial to [$contact_name]"
      return 1
    }
    if ! saga_process_running "$callee"; then
      record "LISTEN:FAIL callee [$callee] not running after listen setup"
      return 1
    fi
    wait_for_relay_online "$callee" "callee" || return 1
  fi
  if [[ "$encrypted" == "true" ]]; then
    warm_encrypted_caller "$caller" || return 1
    adb -s "$caller" logcat -c
  else
    adb -s "$caller" logcat -c
    adb -s "$caller" shell am force-stop org.saga
    sleep 1
  fi
  # Re-check callee relay immediately before dial: warm_encrypted_caller takes several
  # seconds during which the callee can lose its home relay (RELAY_POLL used to latch
  # Ready forever → dial with no inbound accept → RING:FAIL first_missing=inbound_accepted).
  if [[ -n "$callee" ]]; then
    if ! wait_for_relay_online "$callee" "callee-pre-dial"; then
      record "RELAY:FAIL callee [$callee] not relay-ready immediately before dial"
      return 1
    fi
    # Fresh callee logcat so RING checkpoints cannot match a prior call in this process.
    adb -s "$callee" logcat -c 2>/dev/null || true
  fi
  adb -s "$caller" shell am start -n org.saga/.ui.MainActivity \
    -a org.saga.TEST_CONTACT_CALL --es contact_name "$contact_name"
}

stop_caller() {
  clear_stuck_telephony_calls "$1"
  adb -s "$1" shell am force-stop org.saga
}

# Ordered inbound-bridge checkpoints (first missing = where the ring path died).
RING_CHECKPOINT_PATTERNS=(
  "inbound accepted"
  "notifyIncomingCall"
  "CHECKPOINT addNewIncomingCall returned"
  "CHECKPOINT onCreateIncomingConnection entered"
  "CHECKPOINT Connection STATE_RINGING"
  "CHECKPOINT onCallAdded.*telecomState=\\[2\\]"
)
RING_CHECKPOINT_NAMES=(
  "inbound_accepted"
  "notifyIncomingCall"
  "addNewIncomingCall_returned"
  "onCreateIncomingConnection"
  "STATE_RINGING"
  "onCallAdded_RINGING"
)

# On RING:FAIL dump logcat (both devices from dial), dumpsys telecom, and first missing checkpoint.
dump_ring_failure() {
  local callee="$1" label="${2:-callee}"
  local caller="${3:-${SERIAL_A:-}}"
  local stamp
  stamp="$(date +%Y%m%d-%H%M%S)"
  local out_dir="${RING_FAIL_DIR:-/tmp/saga-ring-fail}"
  mkdir -p "$out_dir"
  local base="$out_dir/${stamp}-${label//[^a-zA-Z0-9_-]/_}"
  local first_missing="(all present or none matched)"
  local i name pat status ts_line
  record "RING:DIAG first-missing scan label=[$label] callee=[$callee]"
  for i in "${!RING_CHECKPOINT_NAMES[@]}"; do
    name="${RING_CHECKPOINT_NAMES[$i]}"
    pat="${RING_CHECKPOINT_PATTERNS[$i]}"
    if check_log "$callee" "$pat"; then
      ts_line="$(adb_logcat_dump "$callee" | rg "$pat" | head -1 || true)"
      status="PRESENT"
      record "RING:DIAG checkpoint=$name status=$status ts=${ts_line:0:32}"
    else
      status="MISSING"
      record "RING:DIAG checkpoint=$name status=$status"
      if [[ "$first_missing" == "(all present or none matched)" ]]; then
        first_missing="$name"
      fi
    fi
  done
  record "RING:DIAG first_missing=$first_missing"
  echo "$first_missing" >"${base}.first_missing"
  adb_logcat_dump "$callee" >"${base}.callee.logcat" || true
  if [[ -n "$caller" ]]; then
    adb_logcat_dump "$caller" >"${base}.caller.logcat" || true
  fi
  timeout 10 adb -s "$callee" shell dumpsys telecom >"${base}.callee.telecom" 2>/dev/null || true
  if [[ -n "$caller" ]]; then
    timeout 10 adb -s "$caller" shell dumpsys telecom >"${base}.caller.telecom" 2>/dev/null || true
  fi
  # Compact bridge excerpt for the summary log.
  {
    echo "=== RING FAIL DIAG label=$label first_missing=$first_missing ==="
    if rg -q "relay disconnected|relay no longer connected|Lost connection to relay" \
         "${base}.callee.logcat" 2>/dev/null; then
      echo "--- callee relay drop (likely cause when first_missing=inbound_accepted) ---"
      rg -n "relay disconnected|relay no longer connected|Lost connection to relay|Failed to connect to relay|relay online|relay reconnected" \
        "${base}.callee.logcat" 2>/dev/null | head -40 || true
    fi
    echo "--- callee bridge lines ---"
    rg -n "inbound accepted|inbound connecting|notifyIncomingCall|CHECKPOINT|PhoneAccount|Incoming Call Bridge|ConnectionService|InCallService|Aborting|SecurityException|REJECT|FAIL" \
      "${base}.callee.logcat" 2>/dev/null | head -80 || true
    if [[ -n "$caller" && -f "${base}.caller.logcat" ]]; then
      echo "--- caller dial/connect lines ---"
      rg -n "Dial|connect|Handshake|placeCall|Aborting|IROH_ABORT|outbound|InitMessage" \
        "${base}.caller.logcat" 2>/dev/null | head -40 || true
    fi
    echo "--- callee dumpsys telecom (Calls / PhoneAccount) ---"
    rg -n -i "Call:|mState=|PhoneAccount|org.saga|RINGING|ACTIVE|DISCONNECTED|isEnabled" \
      "${base}.callee.telecom" 2>/dev/null | head -60 || true
  } | tee "${base}.summary.txt"
  record "RING:DIAG artifacts under $base.*"
}

# Callee-side ringing verification (addNewIncomingCall bridge).
wait_for_incoming_ringing() {
  local serial="$1" label="${2:-callee}"
  local max_secs="${3:-45}"
  local interval=2
  local elapsed=0
  while (( elapsed < max_secs )); do
    if check_log "$serial" "CHECKPOINT addNewIncomingCall returned" \
       && check_log "$serial" "CHECKPOINT onCreateIncomingConnection entered" \
       && check_log "$serial" "CHECKPOINT Connection STATE_RINGING" \
       && check_log "$serial" "CHECKPOINT onCallAdded.*telecomState=\[2\]"; then
      record "RING:PASS $label [$serial] incoming UI checkpoints"
      return 0
    fi
    sleep "$interval"
    elapsed=$((elapsed + interval))
  done
  record "RING:FAIL $label [$serial] after ${max_secs}s"
  dump_ring_failure "$serial" "$label" "${SERIAL_A:-}"
  return 1
}

capture_callee_screencap() {
  local serial="$1" dest="$2"
  adb -s "$serial" shell screencap -p /sdcard/saga_ring.png >/dev/null 2>&1 || true
  adb -s "$serial" pull /sdcard/saga_ring.png "$dest" >/dev/null 2>&1 || true
  if [[ -f "$dest" ]]; then
    record "SCREENCAP: saved [$dest] ($(stat -c%s "$dest" 2>/dev/null || echo '?') bytes)"
  else
    record "SCREENCAP:WARN pull failed for [$serial]"
  fi
}

answer_incoming() {
  local serial="$1" lookup_key="$2"
  adb -s "$serial" shell am start -n org.saga/.ui.MainActivity \
    -a org.saga.TEST_ANSWER_INCOMING --es lookup_key "$lookup_key" >/dev/null 2>&1 || true
}

# After answer: Encrypted may already have settled while RINGING; wait for send/playout arm.
wait_for_media_answered() {
  local callee="$1" caller="$2" label="${3:-media-answered}"
  local max_secs="${4:-30}"
  if ! wait_for_handshake_log "$callee" \
    "local answered|JB flushed|onLocalAnswered" "$label-callee" "$max_secs"; then
    return 1
  fi
  if ! wait_for_handshake_log "$caller" \
    "answered signal received|peer answered signal observed|audio I/O started" \
    "$label-caller" "$max_secs"; then
    return 1
  fi
  record "ANSWERED:PASS $label send/playout armed"
  return 0
}

restart() {
  adb -s "$1" shell am force-stop org.saga
  sleep 1
  start_listener "$1"
}

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

enc_flag() {
  adb -s "$1" shell run-as org.saga cat shared_prefs/saga_encryption_established.xml 2>/dev/null \
    | rg -Fq "name=\"$2\" value=\"true\""
}

count_log() {
  adb_logcat_dump "$1" | rg -c "$2" || true
}

provision_clean_baseline() {
  local serial_a="$1" serial_b="$2"
  ensure_local_relay
  adb -s "$serial_a" shell am force-stop org.saga
  adb -s "$serial_b" shell am force-stop org.saga
  sleep 1
  adb -s "$serial_a" logcat -c 2>/dev/null || true
  adb -s "$serial_b" logcat -c 2>/dev/null || true
  push_identity "$serial_a" "alice"
  push_identity "$serial_b" "bob"
  push_iroh_relay_pref "$serial_a"
  push_iroh_relay_pref "$serial_b"
  bash "$E2E_DIR/seed-test-contacts.sh" "$serial_a"
  bash "$E2E_DIR/seed-test-contacts.sh" "$serial_b"
  ensure_callee_listening "$serial_b" || return 1
  wait_for_relay_online "$serial_b" "bob (callee baseline)" || return 1
}

# --- PhoneAccount enable (API 29 lacks set-phone-account-enabled) ---

phone_account_enabled() {
  local serial="$1"
  adb -s "$serial" shell dumpsys telecom | rg -q '\[\[X\] PhoneAccount: ComponentInfo\{org\.saga/org\.saga\.telecom\.SagaConnectionService\}'
}

phone_account_disabled() {
  local serial="$1"
  adb -s "$serial" shell dumpsys telecom | rg -q '\[\[ \] PhoneAccount: ComponentInfo\{org\.saga/org\.saga\.telecom\.SagaConnectionService\}'
}

_e2e_dump_ui() {
  local serial="$1" dump="$2"
  adb -s "$serial" shell uiautomator dump /sdcard/saga_ui.xml >/dev/null 2>&1 || true
  adb -s "$serial" pull /sdcard/saga_ui.xml "$dump" >/dev/null 2>&1 || true
}

_e2e_tap_ui_text() {
  local dump="$1" needle="$2"
  python3 - "$dump" "$needle" <<'PY'
import re, sys
xml = open(sys.argv[1], encoding="utf-8", errors="ignore").read()
needle = sys.argv[2]
for m in re.finditer(r'text="([^"]*)"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', xml):
    if needle in m.group(1):
        x1,y1,x2,y2 = map(int, m.groups()[1:])
        print(f"{(x1+x2)//2} {(y1+y2)//2}")
        sys.exit(0)
for m in re.finditer(r'bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"[^>]*text="([^"]*)"', xml):
    if needle in m.group(5):
        x1,y1,x2,y2 = map(int, m.groups()[:4])
        print(f"{(x1+x2)//2} {(y1+y2)//2}")
        sys.exit(0)
sys.exit(2)
PY
}

_e2e_tap_saga_switch() {
  local dump="$1"
  python3 - "$dump" <<'PY'
import re, sys
xml = open(sys.argv[1], encoding="utf-8", errors="ignore").read()
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

# Hang up leftover cellular/emulator GSM calls that block Telecom placeCall
# ("Aborting call since there's no room").
clear_stuck_telephony_calls() {
  local serial="$1"
  local port
  port="$(echo "$serial" | rg -o '[0-9]+$' || true)"
  if [[ -n "$port" ]]; then
    # Cancel common E2E fixture numbers (thor/alice/bob).
    for n in +15550100012 +15550100011 +15550100010 15550100012 15550100011 15550100010; do
      timeout 3 adb -s "$serial" emu gsm cancel "$n" >/dev/null 2>&1 || true
    done
  fi
  timeout 3 adb -s "$serial" shell input keyevent KEYCODE_ENDCALL >/dev/null 2>&1 || true
  # API 29 often hangs forever on this shell cmd — always bound it.
  timeout 5 adb -s "$serial" shell cmd telecom cleanup-stuck-calls >/dev/null 2>&1 || true
}

enable_saga_phone_account() {
  local serial="$1"
  adb -s "$serial" shell am start -n org.saga/.ui.MainActivity >/dev/null 2>&1 || true
  sleep 1
  # API 30+ supports this; API 29 prints "No shell command implementation".
  local cmd_out
  cmd_out="$(adb -s "$serial" shell cmd telecom set-phone-account-enabled \
    org.saga/org.saga.telecom.SagaConnectionService saga_iroh 0 2>&1 || true)"
  sleep 1
  if phone_account_enabled "$serial"; then
    if echo "$cmd_out" | rg -qi 'No shell command|Unknown'; then
      record "PA:PASS already enabled [$serial]"
    else
      record "PA:PASS enabled via telecom cmd [$serial]"
    fi
    return 0
  fi
  record "PA:INFO telecom cmd unavailable — enabling via Settings UI [$serial]"
  local dump="/tmp/saga-pa-ui-${serial}.xml"
  adb -s "$serial" shell am start -a android.telecom.action.CHANGE_PHONE_ACCOUNTS >/dev/null 2>&1
  sleep 2
  _e2e_dump_ui "$serial" "$dump"
  local all_coords
  all_coords="$(_e2e_tap_ui_text "$dump" "All calling accounts" 2>/dev/null || true)"
  if [[ -n "${all_coords:-}" ]]; then
    adb -s "$serial" shell input tap $all_coords
    sleep 2
    _e2e_dump_ui "$serial" "$dump"
  fi
  local coords
  coords="$(_e2e_tap_saga_switch "$dump" 2>/dev/null || true)"
  if [[ -z "${coords:-}" ]]; then
    record "PA:FAIL no Saga switch on Settings UI [$serial]"
    return 1
  fi
  if phone_account_enabled "$serial"; then
    record "PA:PASS already enabled [$serial]"
    return 0
  fi
  adb -s "$serial" shell input tap $coords
  sleep 2
  if phone_account_enabled "$serial"; then
    record "PA:PASS enabled via Settings UI [$serial]"
    return 0
  fi
  adb -s "$serial" shell input tap $coords
  sleep 2
  if phone_account_enabled "$serial"; then
    record "PA:PASS enabled via Settings UI retry [$serial]"
    return 0
  fi
  record "PA:FAIL could not enable PhoneAccount [$serial]"
  return 1
}
