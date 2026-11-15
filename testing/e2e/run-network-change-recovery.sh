#!/usr/bin/env bash
# Network-change recovery: after airplane / WiFi toggle, measure time to
# relay-ready and to a successful public-relay call (×N each).
# Target: recovery ≤10s (wall from network restore to native relay-ready).
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
E2E_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=e2e-common.sh
source "$E2E_DIR/e2e-common.sh"

SERIAL="${SERIAL:-emulator-5554}"
PEER_BIN="${PEER_BIN:-$REPO_ROOT/tools/saga-peer/target/release/saga-peer}"
OUT_DIR="${OUT_DIR:-/tmp/saga-network-change-recovery}"
N="${N:-5}"
RECOVERY_BUDGET_MS="${RECOVERY_BUDGET_MS:-10000}"
WAV_IN="${WAV_IN:-$REPO_ROOT/saga-media/fixtures/speech_clip_48k.wav}"
CONTACT_LAPTOP="${CONTACT_LAPTOP:-laptop}"

mkdir -p "$OUT_DIR" /tmp/saga-dns
printf 'nameserver 1.1.1.1\nnameserver 8.8.8.8\n' >/tmp/saga-dns/resolv.conf
: >"$OUT_DIR/summary.txt"
record() { echo "$1" | tee -a "$OUT_DIR/summary.txt"; }

run_peer() {
  if command -v bwrap >/dev/null 2>&1; then
    bwrap --dev-bind / / --ro-bind /tmp/saga-dns/resolv.conf /etc/resolv.conf -- "$@"
  else
    "$@"
  fi
}

now_ms() { date +%s%3N; }

wait_native_ready() {
  local serial="$1" budget_ms="$2" t0 t1
  t0=$(now_ms)
  while true; do
    if [[ "$(query_relay_ready_native "$serial")" == "$RELAY_READY_NATIVE" ]]; then
      t1=$(now_ms)
      echo $((t1 - t0))
      return 0
    fi
    t1=$(now_ms)
    if (( t1 - t0 >= budget_ms )); then
      echo $((t1 - t0))
      return 1
    fi
    sleep 0.25
  done
}

ensure_public_alice() {
  adb -s "$SERIAL" shell settings put global private_dns_mode hostname >/dev/null
  adb -s "$SERIAL" shell settings put global private_dns_specifier one.one.one.one >/dev/null
  adb -s "$SERIAL" install -r -t "$REPO_ROOT/android/app/build/outputs/apk/debug/app-debug.apk" >/dev/null
  bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL" >/dev/null
  adb -s "$SERIAL" shell am force-stop org.saga
  adb -s "$SERIAL" shell run-as org.saga rm -f shared_prefs/saga_iroh_relay.xml 2>/dev/null || true
  push_pref "$SERIAL" "$E2E_DIR/saga_encryption_peer.xml" saga_encryption_established.xml
  push_pref "$SERIAL" "$E2E_DIR/saga_dev_identity_alice.xml" saga_dev_identity.xml
  push_pref "$SERIAL" "$E2E_DIR/saga_test_false.xml" saga_test.xml
  enable_saga_phone_account "$SERIAL" || true
  adb -s "$SERIAL" shell am start -n org.saga/.ui.MainActivity >/dev/null 2>&1 || true
  export RELAY_POLL_MAX_SECS="${RELAY_POLL_MAX_SECS:-120}"
  wait_for_relay_online "$SERIAL" "alice-baseline" || return 1
}

airplane_cycle() {
  adb -s "$SERIAL" shell cmd connectivity airplane-mode enable >/dev/null
  sleep 2
  # Confirm relay dropped (or process still up but pending).
  adb -s "$SERIAL" shell cmd connectivity airplane-mode disable >/dev/null
}

wifi_cycle() {
  # Emulator WiFi toggle via svc (works on AVD).
  adb -s "$SERIAL" shell svc wifi disable >/dev/null 2>&1 || true
  sleep 2
  adb -s "$SERIAL" shell svc wifi enable >/dev/null 2>&1 || true
}

one_call_after() {
  local tag="$1" listen_log call_ok=0 t0 t1 wall
  listen_log="$OUT_DIR/${tag}-listen.log"
  : >"$listen_log"
  RUST_LOG=info run_peer "$PEER_BIN" listen \
    --identity 15550100020 \
    --public-relay \
    --wav-in "$WAV_IN" \
    --max-secs 8 \
    >"$listen_log" 2>&1 &
  local listen_pid=$!
  local ready=0
  for _ in $(seq 1 80); do
    if rg -q 'pkarr publish settle done|ready to accept' "$listen_log" 2>/dev/null; then
      ready=1
      break
    fi
    kill -0 "$listen_pid" 2>/dev/null || break
    sleep 0.25
  done
  if [[ "$ready" -ne 1 ]]; then
    kill "$listen_pid" 2>/dev/null || true
    wait "$listen_pid" 2>/dev/null || true
    echo "call_fail"
    return 1
  fi
  sleep 2
  adb -s "$SERIAL" logcat -c 2>/dev/null || true
  t0=$(now_ms)
  adb -s "$SERIAL" shell am start -n org.saga/.ui.MainActivity \
    -a org.saga.TEST_CONTACT_CALL --es contact_name "$CONTACT_LAPTOP" >/dev/null 2>&1 || true
  for _ in $(seq 1 30); do
    if adb_logcat_dump "$SERIAL" | rg -q 'Handshake settled to \[Encrypted\]'; then
      call_ok=1
      break
    fi
    if adb_logcat_dump "$SERIAL" | rg -q 'NOT_REACHABLE|failNotReachable'; then
      break
    fi
    sleep 1
  done
  t1=$(now_ms)
  wall=$((t1 - t0))
  kill "$listen_pid" 2>/dev/null || true
  wait "$listen_pid" 2>/dev/null || true
  adb -s "$SERIAL" shell input keyevent KEYCODE_ENDCALL >/dev/null 2>&1 || true
  clear_stuck_telephony_calls "$SERIAL" || true
  if [[ "$call_ok" -eq 1 ]]; then
    echo "call_ok:$wall"
    return 0
  fi
  echo "call_fail:$wall"
  return 1
}

run_scenario() {
  local name="$1" cycle_fn="$2"
  local i recover_ms call_res pass=0 fail=0 slow=0
  record "INFO: scenario=$name N=$N budget_ms=$RECOVERY_BUDGET_MS"
  for i in $(seq 1 "$N"); do
    local tag
    tag=$(printf '%s%02d' "$name" "$i")
    # Ensure we start from ready.
    if [[ "$(query_relay_ready_native "$SERIAL")" != "$RELAY_READY_NATIVE" ]]; then
      start_listener "$SERIAL"
      wait_for_relay_online "$SERIAL" "$tag-rewarm" || true
    fi
    "$cycle_fn"
    # From network restore → relay-ready.
    if recover_ms=$(wait_native_ready "$SERIAL" 60000); then
      :
    else
      recover_ms=${recover_ms:-60000}
      record "$tag:FAIL relay_recover_ms=$recover_ms (never ready)"
      fail=$((fail + 1))
      continue
    fi
    if (( recover_ms > RECOVERY_BUDGET_MS )); then
      slow=$((slow + 1))
      record "$tag:WARN relay_recover_ms=$recover_ms (>${RECOVERY_BUDGET_MS})"
    else
      record "$tag:INFO relay_recover_ms=$recover_ms"
    fi
    call_res=$(one_call_after "$tag" || true)
    if [[ "$call_res" == call_ok:* ]]; then
      record "$tag:PASS relay_recover_ms=$recover_ms $call_res"
      pass=$((pass + 1))
    else
      record "$tag:FAIL relay_recover_ms=$recover_ms $call_res"
      fail=$((fail + 1))
    fi
  done
  record "RESULT scenario=$name pass=$pass/$N fail=$fail slow_recover=$slow"
  [[ "$fail" -eq 0 ]]
}

if [[ ! -x "$PEER_BIN" ]]; then
  cargo build --release --manifest-path "$REPO_ROOT/tools/saga-peer/Cargo.toml"
fi

record "INFO: network-change recovery serial=$SERIAL"
ensure_public_alice
run_scenario "airplane" airplane_cycle
run_scenario "wifi" wifi_cycle
record "RESULT overall OK"
