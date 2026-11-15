#!/usr/bin/env bash
# Host saga-peer (laptop) → emulator (alice) over public N0 relays ×N.
# Reverse of run-public-relay-emu-peer-soak.sh.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
E2E_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=e2e-common.sh
source "$E2E_DIR/e2e-common.sh"

SERIAL="${SERIAL:-emulator-5554}"
PEER_BIN="${PEER_BIN:-$REPO_ROOT/tools/saga-peer/target/release/saga-peer}"
OUT_DIR="${OUT_DIR:-/tmp/saga-public-relay-peer-emu}"
N="${N:-10}"
WAV_IN="${WAV_IN:-$REPO_ROOT/saga-media/fixtures/speech_clip_48k.wav}"
MAX_SECS="${MAX_SECS:-8}"
PHONE_ALICE="${PHONE_ALICE:-15550100011}"

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

if [[ ! -x "$PEER_BIN" ]]; then
  cargo build --release --manifest-path "$REPO_ROOT/tools/saga-peer/Cargo.toml"
fi

export SAGA_IROH_RELAY_MODE=public
record "INFO: peer→emulator public soak N=$N serial=$SERIAL"

adb -s "$SERIAL" shell settings put global private_dns_mode hostname >/dev/null
adb -s "$SERIAL" shell settings put global private_dns_specifier one.one.one.one >/dev/null
record "INFO: private DNS hostname=one.one.one.one"

adb -s "$SERIAL" install -r -t "$REPO_ROOT/android/app/build/outputs/apk/debug/app-debug.apk" >/dev/null
bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL"
adb -s "$SERIAL" shell am force-stop org.saga
adb -s "$SERIAL" shell am start -n org.saga/.ui.MainActivity >/dev/null 2>&1 || true
sleep 1
enable_saga_phone_account "$SERIAL" || true
adb -s "$SERIAL" shell run-as org.saga rm -f shared_prefs/saga_iroh_relay.xml 2>/dev/null || true
push_pref "$SERIAL" "$E2E_DIR/saga_encryption_peer.xml" saga_encryption_established.xml
push_pref "$SERIAL" "$E2E_DIR/saga_dev_identity_alice.xml" saga_dev_identity.xml
push_pref "$SERIAL" "$E2E_DIR/saga_test_false.xml" saga_test.xml

export RELAY_POLL_MAX_SECS="${RELAY_POLL_MAX_SECS:-180}"
adb -s "$SERIAL" shell run-as org.saga rm -f shared_prefs/saga_iroh_relay.xml 2>/dev/null || true
adb -s "$SERIAL" shell am force-stop org.saga
sleep 1
adb -s "$SERIAL" logcat -c 2>/dev/null || true
start_listener "$SERIAL"
if ! wait_for_relay_online "$SERIAL" "alice-public-listen"; then
  record "RESULT pass=0/$N fail=$N (alice public relay never ready)"
  exit 1
fi

pass=0
fail=0
for i in $(seq 1 "$N"); do
  tag=$(printf 'peer%02d' "$i")
  if ! saga_process_running "$SERIAL"; then
    record "INFO: $tag rewarming alice (process gone)"
    adb -s "$SERIAL" shell run-as org.saga rm -f shared_prefs/saga_iroh_relay.xml 2>/dev/null || true
    start_listener "$SERIAL"
    if ! wait_for_relay_online "$SERIAL" "alice-public-rewarm"; then
      record "$tag:FAIL alice relay not ready (public)"
      fail=$((fail + 1))
      continue
    fi
  fi

  call_log="$OUT_DIR/${tag}-call.log"
  : >"$call_log"
  adb -s "$SERIAL" logcat -c 2>/dev/null || true
  t0=$(date +%s%3N)
  RUST_LOG=info run_peer "$PEER_BIN" call \
    --identity 15550100020 \
    --peer "$PHONE_ALICE" \
    --public-relay \
    --wav-in "$WAV_IN" \
    --wav-out "$OUT_DIR/${tag}-peer-out.wav" \
    --loop-wav \
    --max-secs "$MAX_SECS" \
    >"$call_log" 2>&1 &
  call_pid=$!

  ok=0
  if wait_for_incoming_ringing "$SERIAL" "alice-from-peer-$tag" 45; then
    answer_incoming "$SERIAL" "15550100020" || true
    for _ in $(seq 1 45); do
      if adb_logcat_dump "$SERIAL" | rg -q 'Handshake settled to \[Encrypted\]'; then
        ok=1
        break
      fi
      if ! kill -0 "$call_pid" 2>/dev/null; then
        break
      fi
      sleep 1
    done
  fi
  t1=$(date +%s%3N)
  wall=$((t1 - t0))

  kill "$call_pid" 2>/dev/null || true
  wait "$call_pid" 2>/dev/null || true
  adb -s "$SERIAL" shell input keyevent KEYCODE_ENDCALL >/dev/null 2>&1 || true
  clear_stuck_telephony_calls "$SERIAL" || true

  home=$(rg -o 'home=\[([^]]*)\]' -r '$1' "$call_log" | head -1 || true)
  if [[ "$ok" -eq 1 ]]; then
    record "$tag:PASS wall_ms=$wall peer_home=$home"
    pass=$((pass + 1))
  else
    err=$(adb_logcat_dump "$SERIAL" | rg 'NOT_REACHABLE|No addressing|connect attempt|UnknownIssuer|iroh connect' | head -3 | tr '\n' '|' || true)
    record "$tag:FAIL wall_ms=$wall peer_home=$home err=${err:0:220}"
    fail=$((fail + 1))
  fi
done

record "RESULT pass=$pass/$N fail=$fail"
[[ "$fail" -eq 0 ]]
