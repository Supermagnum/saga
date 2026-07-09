# Shared E2E helpers — source from run-e2e-full.sh / run-case-standalone.sh
# Relay gating: poll native endpoint.online() completion via JNI (relay_status.txt),
# not logcat markers. After force-stop, clear logcat once to discard stale lines only.

RELAY_POLL_INTERVAL_SECS="${RELAY_POLL_INTERVAL_SECS:-3}"
RELAY_POLL_MAX_SECS="${RELAY_POLL_MAX_SECS:-120}"
LISTEN_POLL_MAX_SECS="${LISTEN_POLL_MAX_SECS:-30}"
HANDSHAKE_POLL_MAX_SECS="${HANDSHAKE_POLL_MAX_SECS:-120}"
SAGA_IROH_RELAY_MODE="${SAGA_IROH_RELAY_MODE:-local}"
SAGA_IROH_RELAY_URL="${SAGA_IROH_RELAY_URL:-http://10.0.2.2:3340}"
SAGA_RELAY_RETRY_ON_FAIL="${SAGA_RELAY_RETRY_ON_FAIL:-0}"

LISTEN_MARKER="${LISTEN_MARKER:-\[Saga Iroh Listen\] endpoint bound and accepting inbound}"
RELAY_MARKER="${RELAY_MARKER:-\[Saga Iroh Listen\] relay online}"

RELAY_READY_NATIVE=1
RELAY_PENDING_NATIVE=0
RELAY_FAILED_NATIVE=2

check_log() {
  adb -s "$1" logcat -d | rg -q "$2"
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
  record "RELAY:INFO local relay at $SAGA_IROH_RELAY_URL"
}

push_iroh_relay_pref() {
  local serial="$1"
  if [[ "$SAGA_IROH_RELAY_MODE" == "public" ]]; then
    adb -s "$serial" shell run-as org.saga rm -f shared_prefs/saga_iroh_relay.xml 2>/dev/null || true
    return 0
  fi
  push_pref "$serial" "$E2E_DIR/saga_iroh_relay_local.xml" saga_iroh_relay.xml
}

query_relay_ready_native() {
  local serial="$1"
  adb -s "$serial" shell am start -a org.saga.TEST_RELAY_QUERY \
    -n org.saga/.ui.MainActivity >/dev/null 2>&1 || true
  sleep 0.5
  adb -s "$serial" shell run-as org.saga cat "files/${RELAY_STATUS_FILE:-relay_status.txt}" 2>/dev/null \
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
  logcat_relay="$(adb -s "$serial" logcat -d 2>/dev/null | rg -c '\[Saga Iroh Listen\] relay online' || true)"
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
  if [[ -n "$callee" ]]; then
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
  adb -s "$caller" shell am start -n org.saga/.ui.MainActivity \
    -a org.saga.TEST_CONTACT_CALL --es contact_name "$contact_name"
}

stop_caller() {
  adb -s "$1" shell am force-stop org.saga
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
  adb -s "$1" logcat -d | rg -c "$2" || true
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
