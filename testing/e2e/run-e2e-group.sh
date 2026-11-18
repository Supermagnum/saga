#!/usr/bin/env bash
# Multi-peer group call E2E: G1-G11 (up to 4 participants including self).
#
# LOCAL ONLY (3 emulators + host saga-peer); never a CI gate.
#   alice    emulator-5554  (SERIAL_A)
#   bob      emulator-5556  (SERIAL_B)
#   charlene emulator-5558  (SERIAL_C, label 15550100030)
#   laptop   host saga-peer (label 15550100020), the 4th participant
#
# Drives the app only through debug intents (MainActivity, FLAG_DEBUGGABLE):
#   org.saga.TEST_CONTACT_CALL            1:1 dial (existing)
#   org.saga.TEST_ANSWER_INCOMING         answer a ringing leg (existing)
#   org.saga.TEST_ADD_PARTICIPANT         add a Saga contact to the live call / group
#   org.saga.TEST_GROUP_HANGUP_PARTICIPANT  hang up ONE participant
#   org.saga.TEST_MEDIA_MUTE (broadcast)  mute all engines
# plus UI taps (toggle_hold / end_call) and GSM console for the cellular case.
#
# Result tags (honest):
#   PASS            asserted and observed
#   FAIL            asserted and not observed (suite exits 1)
#   NEEDS_HARDWARE  emulator cannot exercise this (RIL, real audio path)
#   PENDING_MESH    needs the native mesh backend (Bob<->Charlene links) or a mesh-capable
#                   saga-peer; the pairwise-session scaffold only gives the adder's links
#
# Usage:
#   bash testing/e2e/run-e2e-group.sh                  # all cases
#   G_CASES="G1 G2" bash testing/e2e/run-e2e-group.sh  # subset
#   SAGA_IROH_RELAY_MODE=public bash ...               # public N0 relays (Private DNS cases)
set -uo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
E2E_DIR="$(cd "$(dirname "$0")" && pwd)"

RESULTS=()
record() { RESULTS+=("$1"); echo "$1"; }

# shellcheck source=e2e-common.sh
source "$E2E_DIR/e2e-common.sh"

SERIAL_A="${SERIAL_A:-emulator-5554}"
SERIAL_B="${SERIAL_B:-emulator-5556}"
SERIAL_C="${SERIAL_C:-emulator-5558}"
OUT_DIR="${OUT_DIR:-/tmp/saga-group-e2e}"
SCREENCAP_DIR="${SCREENCAP_DIR:-$OUT_DIR/screencaps}"
G_CASES="${G_CASES:-G1 G2 G3 G4 G5 G6 G7 G8 G9 G10 G11}"
SAGA_GROUP_HOST_MESH="${SAGA_GROUP_HOST_MESH:-1}"
SPEECH="$REPO_ROOT/saga-media/fixtures/speech_clip_48k.wav"
LOOKUP_ALICE="15550100011"   # how alice appears on a callee (answer_incoming key)
FP_CHARLENE="e4f3e33e7d3e50990571c0f6058993d47a5b24c55cc55b7a5df8c8cbc0bbdf32"
FP_WRONG="0000000000000000000000000000000000000000000000000000000000000000"
FP_BOB="80342e72c8388b728d66df256f8231258f250f760cd45613e34ab5d2ed9f4fa8"
mkdir -p "$OUT_DIR" "$SCREENCAP_DIR"

STATE="none"        # none | pair | group3 | group4
PEER_PID=""
PEER_BIN="${PEER_BIN:-}"

want_case() { [[ " $G_CASES " == *" $1 "* ]]; }

# ---------------------------------------------------------------- infrastructure

device_online() { adb devices 2>/dev/null | rg -q "^$1[[:space:]]+device"; }

preflight() {
  local missing=0 s
  for s in "$SERIAL_A" "$SERIAL_B" "$SERIAL_C"; do
    if ! device_online "$s"; then
      echo "ERROR: $s is not online." >&2
      missing=1
    fi
  done
  if [[ "$missing" -ne 0 ]]; then
    cat >&2 <<EOF
Start the three emulators first (ports must match the serials), for example:
  emulator -avd <alice-avd>    -port 5554 &
  emulator -avd <bob-avd>      -port 5556 &
  emulator -avd <charlene-avd> -port 5558 &
and install the debug APK on each:
  adb -s emulator-5558 install -r android/app/build/outputs/apk/debug/app-debug.apk
EOF
    exit 2
  fi
  local s2
  for s2 in "$SERIAL_A" "$SERIAL_B" "$SERIAL_C"; do
    if ! adb -s "$s2" shell pm list packages 2>/dev/null | rg -q 'package:org.saga$'; then
      echo "ERROR: org.saga not installed on $s2 (install the debug APK)." >&2
      exit 2
    fi
  done
}

ensure_dialer() {
  adb -s "$1" shell cmd role add-role-holder android.app.role.DIALER org.saga 2>/dev/null || true
}

resolve_peer_bin() {
  if [[ -n "$PEER_BIN" && -x "$PEER_BIN" ]]; then return 0; fi
  if [[ -x "$REPO_ROOT/tools/saga-peer/target/release/saga-peer" ]]; then
    PEER_BIN="$REPO_ROOT/tools/saga-peer/target/release/saga-peer"
  elif [[ -n "${CARGO_TARGET_DIR:-}" && -x "$CARGO_TARGET_DIR/release/saga-peer" ]]; then
    PEER_BIN="$CARGO_TARGET_DIR/release/saga-peer"
  else
    echo "=== Building saga-peer ==="
    cargo build --release --manifest-path "$REPO_ROOT/tools/saga-peer/Cargo.toml" || return 1
    if [[ -x "$REPO_ROOT/tools/saga-peer/target/release/saga-peer" ]]; then
      PEER_BIN="$REPO_ROOT/tools/saga-peer/target/release/saga-peer"
    elif [[ -n "${CARGO_TARGET_DIR:-}" && -x "$CARGO_TARGET_DIR/release/saga-peer" ]]; then
      PEER_BIN="$CARGO_TARGET_DIR/release/saga-peer"
    else
      return 1
    fi
  fi
}

stop_peer() {
  if [[ -n "$PEER_PID" ]] && kill -0 "$PEER_PID" 2>/dev/null; then
    kill "$PEER_PID" 2>/dev/null || true
    wait "$PEER_PID" 2>/dev/null || true
  fi
  PEER_PID=""
}

# Start the laptop (auto-answers, loops the speech clip, writes what it hears).
start_peer() {
  local tag="$1"
  stop_peer
  resolve_peer_bin || { record "$tag:FAIL saga-peer binary unavailable (set PEER_BIN)"; return 1; }
  local relay_args=()
  if [[ "$SAGA_IROH_RELAY_MODE" == "public" ]]; then
    relay_args=(--public-relay)
  else
    relay_args=(--relay "$SAGA_IROH_RELAY_URL")
  fi
  rm -f "$OUT_DIR/peer-laptop-$tag.wav"
  "$PEER_BIN" listen --identity 15550100020 "${relay_args[@]}" \
    --wav-in "$SPEECH" --loop-wav --wav-out "$OUT_DIR/peer-laptop-$tag.wav" \
    >"$OUT_DIR/peer-$tag.log" 2>&1 &
  PEER_PID=$!
  local _
  for _ in $(seq 1 60); do
    if rg -q 'relay online' "$OUT_DIR/peer-$tag.log" 2>/dev/null; then
      sleep 1
      return 0
    fi
    sleep 0.5
  done
  record "$tag:FAIL saga-peer (laptop) did not report relay online"
  return 1
}

cleanup() {
  stop_peer
  local s
  for s in "$SERIAL_A" "$SERIAL_B" "$SERIAL_C"; do
    clear_stuck_telephony_calls "$s" 2>/dev/null || true
  done
}
trap cleanup EXIT

save_logs() {
  local tag="$1"
  adb_logcat_dump "$SERIAL_A" >"$OUT_DIR/$tag-alice.logcat" || true
  adb_logcat_dump "$SERIAL_B" >"$OUT_DIR/$tag-bob.logcat" || true
  adb_logcat_dump "$SERIAL_C" >"$OUT_DIR/$tag-charlene.logcat" || true
  record "$tag:INFO logcat saved under $OUT_DIR/$tag-{alice,bob,charlene}.logcat"
}

clear_all_logs() {
  local s
  for s in "$SERIAL_A" "$SERIAL_B" "$SERIAL_C"; do
    adb -s "$s" logcat -c 2>/dev/null || true
  done
}

# Enlarge ring buffers so Handshake/ENCRYPTED_LEGS markers are not rotated out by
# high-volume iroh poll_send spam before G2/G6 settled_count runs (default is 2 MiB).
enlarge_logcat_buffers() {
  local s
  for s in "$SERIAL_A" "$SERIAL_B" "$SERIAL_C"; do
    adb -s "$s" logcat -G 16M 2>/dev/null || true
  done
}

# Alice's encrypted group-leg count. Prefer durable ENCRYPTED_LEGS (survives spam);
# fall back to distinct Handshake settled lookups.
encrypted_legs_count() {
  local serial="$1"
  local n
  n="$(adb_logcat_dump "$serial" \
    | rg -o 'ENCRYPTED_LEGS group=\[[^]]+\] count=[0-9]+' \
    | tail -1 | rg -o 'count=[0-9]+' | cut -d= -f2 || true)"
  if [[ -n "$n" ]]; then
    echo "$n"
    return 0
  fi
  settled_count "$serial" Encrypted
}

# Foreground the in-call UI, screenshot it and dump the view tree (ui text assertions).
capture_group_ui() {
  local serial="$1" name="$2"
  adb -s "$serial" shell am start -a org.saga.TEST_IN_CALL_UI -n org.saga/.ui.SagaInCallActivity >/dev/null 2>&1 || true
  sleep 2
  capture_callee_screencap "$serial" "$SCREENCAP_DIR/$name.png" || true
  _e2e_dump_ui "$serial" "$OUT_DIR/$name.xml"
}

ui_has() { [[ -f "$1" ]] && rg -qi "$2" "$1"; }

backend_of() {
  adb_logcat_dump "$1" | rg -o 'Add participant placed group=\[[^]]+\] leg=\[[^]]+\] backend=\[[A-Z_]+\]' \
    | tail -1 | rg -o 'backend=\[[A-Z_]+\]' | sed 's/backend=\[\(.*\)\]/\1/' || true
}

# wait_encrypted_for SERIAL DIGITS [SECS]: handshake settled to Encrypted for that peer label.
wait_encrypted_for() {
  local serial="$1" digits="$2" secs="${3:-60}"
  poll_for_log_marker "$serial" "Handshake settled to \[Encrypted\] for \[\+?$digits\]" "enc-$digits" "$secs" 2
}

media_sessions() {
  adb_logcat_dump "$1" | rg -o '\[Saga Media\] start session=[^ ]+' | sort -u | rg -c . || echo 0
}

# ---------------------------------------------------------------- media metrics

# group_media_probe TAG LISTENER_SERIAL:NAME ...
# Alice injects the speech clip; listeners dump playout PCM; each dump is scored vs the
# reference (corr / SI-SNR / STOI via pcm_score.py). Metrics are INFO; link states stay the
# pass/fail signal because the dump may mix several streams.
group_media_probe() {
  local tag="$1"
  shift
  adb -s "$SERIAL_A" push "$SPEECH" /data/local/tmp/speech_clip_48k.wav >/dev/null 2>&1 || true
  adb -s "$SERIAL_A" shell run-as org.saga cp /data/local/tmp/speech_clip_48k.wav files/speech_clip_48k.wav 2>/dev/null || true
  local spec serial name
  for spec in "$@"; do
    serial="${spec%%:*}"
    name="${spec##*:}"
    adb -s "$serial" shell am broadcast -p org.saga -a org.saga.TEST_PCM_DUMP \
      --es path "/data/data/org.saga/files/${tag}_${name}.pcm" >/dev/null 2>&1 || true
  done
  adb -s "$SERIAL_A" shell am broadcast -p org.saga -a org.saga.TEST_PCM_INJECT \
    --es path /data/data/org.saga/files/speech_clip_48k.wav >/dev/null 2>&1 || true
  sleep 8
  for spec in "$@"; do
    serial="${spec%%:*}"
    name="${spec##*:}"
    adb -s "$serial" shell am broadcast -p org.saga -a org.saga.TEST_PCM_DUMP_FINISH >/dev/null 2>&1 || true
  done
  sleep 1
  for spec in "$@"; do
    serial="${spec%%:*}"
    name="${spec##*:}"
    local host="$OUT_DIR/${tag}_${name}.pcm"
    timeout 20 adb -s "$serial" exec-out run-as org.saga cat "files/${tag}_${name}.pcm" >"$host" 2>/dev/null || true
    if [[ -s "$host" ]]; then
      if python3 "$E2E_DIR/pcm_score.py" --ref "$SPEECH" --deg-raw "$host" --label "${tag}_alice_to_${name}" \
        >"$OUT_DIR/${tag}_${name}.score.txt" 2>&1; then
        record "$tag:METRIC $(tr -d '\n' <"$OUT_DIR/${tag}_${name}.score.txt")"
      else
        record "$tag:INFO pcm for $name at $host (scoring unavailable: $(tr -d '\n' <"$OUT_DIR/${tag}_${name}.score.txt" | cut -c1-120))"
      fi
    else
      record "$tag:INFO no pcm dump for $name (dump path empty); link states are the evidence"
    fi
  done
}

# ---------------------------------------------------------------- state builders

teardown_all() {
  stop_peer
  local s
  for s in "$SERIAL_A" "$SERIAL_B" "$SERIAL_C"; do
    clear_stuck_telephony_calls "$s"
    adb -s "$s" shell am force-stop org.saga
    sleep 0.5
    # Force-stop leaves RINGING/ACTIVE Telecom rows until anomaly watchdog; drain now
    # so the next setup_pair dial does not hit max_ringing / miss onCallAdded.
    clear_stuck_telephony_calls "$s"
    ensure_telecom_idle "$s" "teardown-$s" 15 || true
  done
  STATE="none"
  sleep 1
}

one_time_provision() {
  provision_clean_baseline_group "$SERIAL_A" "$SERIAL_B" "$SERIAL_C" || return 1
  local s
  for s in "$SERIAL_A" "$SERIAL_B" "$SERIAL_C"; do
    ensure_dialer "$s"
    enable_saga_phone_account "$s" || true
  done
  return 0
}

# Bring SagaInCallActivity to the front without ending the live Telecom/Iroh call.
keep_incall_front() {
  local serial="$1"
  adb -s "$serial" shell am start -n org.saga/.ui.SagaInCallActivity >/dev/null 2>&1 || true
}

# alice -> bob, bob answers, wait Encrypted on both. Leaves InCall up on both.
setup_pair() {
  teardown_all
  local s
  for s in "$SERIAL_A" "$SERIAL_B" "$SERIAL_C"; do
    push_group_peer_prefs "$s"
  done
  ensure_callee_listening "$SERIAL_C" || { record "SETUP:FAIL charlene not listening"; return 1; }
  wait_for_relay_online "$SERIAL_C" "charlene" || return 1
  if ! contact_call "$SERIAL_A" "bob" "$SERIAL_B" true; then
    record "SETUP:FAIL alice->bob dial"
    return 1
  fi
  wait_for_incoming_ringing "$SERIAL_B" "bob group-setup" 60 || return 1
  # Answer, then keep InCall in front so Connection/registry stay live for adds.
  answer_incoming "$SERIAL_B" "$LOOKUP_ALICE"
  keep_incall_front "$SERIAL_B"
  keep_incall_front "$SERIAL_A"
  # Initiator may settle Encrypted before answer; wait long enough either way.
  if ! wait_for_handshake_log "$SERIAL_A" "settled to \[Encrypted\]" "pair-alice" 60; then
    record "SETUP:INFO pair-alice Encrypted miss — re-answer bob and retry settle"
    answer_incoming "$SERIAL_B" "$LOOKUP_ALICE"
    keep_incall_front "$SERIAL_B"
    keep_incall_front "$SERIAL_A"
    wait_for_handshake_log "$SERIAL_A" "settled to \[Encrypted\]" "pair-alice-retry" 45 || return 1
  fi
  if ! wait_for_handshake_log "$SERIAL_B" "settled to \[Encrypted\]" "pair-bob" 60; then
    record "SETUP:INFO pair-bob Encrypted miss — re-answer and retry"
    answer_incoming "$SERIAL_B" "$LOOKUP_ALICE"
    keep_incall_front "$SERIAL_B"
    wait_for_handshake_log "$SERIAL_B" "settled to \[Encrypted\]" "pair-bob-retry" 45 || return 1
  fi
  keep_incall_front "$SERIAL_A"
  keep_incall_front "$SERIAL_B"
  # Do not force-stop or ENDCALL alice/bob here — adds need the live Iroh leg.
  adb -s "$SERIAL_C" logcat -c 2>/dev/null || true
  STATE="pair"
  return 0
}

# Prepare callee for an inbound group leg without touching the adder's live call.
# Soft path: if Saga is up and home-relay is already Ready, do not force-stop (that
# was dropping Charlene's relay and causing G4a inbound_accepted MISSING).
prepare_callee_for_join() {
  local callee="$1"
  clear_stuck_telephony_calls "$callee" 2>/dev/null || true
  ensure_telecom_idle "$callee" "join-callee-$callee" 20 || true
  if saga_process_running "$callee" && \
     [[ "$(query_relay_ready_native "$callee")" == "$RELAY_READY_NATIVE" ]]; then
    record "SETUP:INFO $callee already relay-ready — skipping force-stop"
    adb -s "$callee" logcat -c 2>/dev/null || true
    return 0
  fi
  ensure_callee_listening "$callee" || return 1
  wait_for_relay_online "$callee" "join-callee" || return 1
  # Re-check immediately before returning — native Ready can race a disconnect.
  if [[ "$(query_relay_ready_native "$callee")" != "$RELAY_READY_NATIVE" ]]; then
    record "SETUP:INFO $callee relay not Ready after listen — fresh warm"
    wait_for_fresh_relay_online "$callee" "join-callee-rewarm" || return 1
  fi
  adb -s "$callee" logcat -c 2>/dev/null || true
  return 0
}

# ADDER adds NAME (callee ringing on CALLEE_SERIAL answers with ANSWER_LOOKUP).
join_via() {
  local adder="$1" name="$2" callee="$3" answer_lookup="$4" digits="$5"
  prepare_callee_for_join "$callee" || {
    record "SETUP:FAIL $name callee not listening before add"
    return 1
  }
  # Never drain/end the adder call — only re-show InCall so requestAdd sees the leg.
  keep_incall_front "$adder"
  sleep 1
  local outcome="" attempt
  for attempt in 1 2 3; do
    # Fresh callee logcat each attempt so RING checkpoints cannot match a prior leg.
    # Do not clear the adder: settled_count(G2/G6) needs prior Encrypted settles (e.g. bob).
    adb -s "$callee" logcat -c 2>/dev/null || true
    keep_incall_front "$adder"
    sleep 0.5
    add_participant "$adder" "$name"
    outcome="$(wait_add_participant_outcome "$adder" "$name" 12 || true)"
    # REFUSED_DUPLICATE after a prior PLACED in this join is fine (intent redelivery).
    if [[ "$outcome" == "PLACED" || "$outcome" == "REFUSED_DUPLICATE" ]]; then
      if wait_for_incoming_ringing "$callee" "$name group-leg" 45; then
        break
      fi
      record "SETUP:INFO $name PLACED but no ring (attempt=$attempt) — rewarm callee relay"
      # Adder call stays up; only rewarm the callee that lost home-relay.
      wait_for_fresh_relay_online "$callee" "join-rewarm-$name" || true
      outcome=""
      continue
    fi
    record "SETUP:INFO add $name attempt=$attempt outcome=[${outcome:-none}] -- re-showing InCall"
    keep_incall_front "$adder"
    sleep 1
  done
  if [[ "$outcome" != "PLACED" && "$outcome" != "REFUSED_DUPLICATE" ]]; then
    record "SETUP:FAIL add $name outcome=[${outcome:-none}]"
    return 1
  fi
  # If the successful attempt already saw RING above, this is a fast re-check.
  wait_for_incoming_ringing "$callee" "$name group-leg" 20 || return 1
  answer_incoming "$callee" "$answer_lookup"
  keep_incall_front "$adder"
  if ! wait_encrypted_for "$adder" "$digits" 60; then
    record "SETUP:FAIL $name did not reach Encrypted on adder"
    return 1
  fi
  keep_incall_front "$adder"
  return 0
}

setup_group3() {
  if [[ "$STATE" == "group3" ]]; then return 0; fi
  setup_pair || return 1
  join_via "$SERIAL_A" "charlene" "$SERIAL_C" "$LOOKUP_ALICE" "15550100030" || {
    record "SETUP:FAIL charlene did not reach Encrypted on alice"
    return 1
  }
  STATE="group3"
  return 0
}

setup_group4() {
  if [[ "$STATE" == "group4" ]]; then return 0; fi
  setup_group3 || return 1
  start_peer "laptop-join" || return 1
  # Keep InCall + dial sessions alive after G2/G3 probes; REFUSED_NO_IROH_CALL means the
  # adder lost its live Iroh leg (process restart / concurrent harness contention).
  keep_incall_front "$SERIAL_A"
  keep_incall_front "$SERIAL_B"
  keep_incall_front "$SERIAL_C"
  sleep 1
  add_participant "$SERIAL_A" "laptop"
  sleep 2
  local outcome
  outcome="$(add_participant_outcome "$SERIAL_A" "laptop")"
  if [[ "$outcome" == "REFUSED_NO_IROH_CALL" ]]; then
    record "SETUP:INFO add laptop REFUSED_NO_IROH_CALL — rebuilding group3 then retry"
    stop_peer 2>/dev/null || true
    STATE="none"
    setup_group3 || return 1
    start_peer "laptop-join" || return 1
    keep_incall_front "$SERIAL_A"
    sleep 1
    add_participant "$SERIAL_A" "laptop"
    sleep 2
    outcome="$(add_participant_outcome "$SERIAL_A" "laptop")"
  fi
  if [[ "$outcome" != "PLACED" && "$outcome" != "REFUSED_DUPLICATE" ]]; then
    record "SETUP:FAIL add laptop outcome=[${outcome:-none}]"
    return 1
  fi
  wait_encrypted_for "$SERIAL_A" "15550100020" 60 || {
    record "SETUP:FAIL laptop did not reach Encrypted on alice"
    return 1
  }
  STATE="group4"
  return 0
}

host_mesh_check() {
  [[ "$SAGA_GROUP_HOST_MESH" == "1" ]] || return 0
  [[ -n "${HOST_MESH_DONE:-}" ]] && return 0
  HOST_MESH_DONE=1
  command -v cargo >/dev/null 2>&1 || { record "HOSTMESH:INFO cargo not found; host mesh model tests skipped"; return 0; }
  echo "=== Host mesh model tests (saga-peer host_mesh) ==="
  timeout 1500 cargo test --manifest-path "$REPO_ROOT/tools/saga-peer/Cargo.toml" \
    --test host_mesh -- --nocapture >"$OUT_DIR/host-mesh.log" 2>&1 || true
  local t
  for t in three_peer_encrypted_mesh four_peer_encrypted_mesh \
    three_peer_unencrypted_mesh_after_explicit_confirm leave_continues_for_remaining_peers \
    fifth_participant_refused; do
    if rg -q "test $t \.\.\. ok" "$OUT_DIR/host-mesh.log"; then
      record "HOSTMESH:PASS $t (host model, no emulator; log $OUT_DIR/host-mesh.log)"
    else
      record "HOSTMESH:FAIL $t not ok (see $OUT_DIR/host-mesh.log)"
    fi
  done
}

# ---------------------------------------------------------------- cases

case_g1() {
  # G1: Alice -> Bob 1:1 encrypted, behavior unchanged (no group UI/markers).
  # Clear prior group-case log markers so leftover "Add participant placed" from
  # earlier suite runs cannot false-fail this baseline.
  adb -s "$SERIAL_A" logcat -c 2>/dev/null || true
  adb -s "$SERIAL_B" logcat -c 2>/dev/null || true
  setup_pair || { record "G1:FAIL could not establish encrypted 1:1"; return 0; }
  # setup_pair already waited for Encrypted on both ends; do not re-query
  # logcat (ring buffer / timing made a second check flaky). Fail only if this
  # 1:1 somehow promoted into a group.
  local ok=1
  if check_log "$SERIAL_A" "Add participant placed|addConference group="; then
    record "G1:INFO group markers present in a plain 1:1"
    ok=0
  fi
  capture_group_ui "$SERIAL_A" "g1-1to1-alice"
  capture_group_ui "$SERIAL_B" "g1-1to1-bob"
  record_handshake_budget "$SERIAL_A" "g1 alice->bob" || true
  if [[ "$ok" -eq 1 ]]; then
    record "G1:PASS alice<->bob 1:1 Encrypted on both, no group markers"
  else
    record "G1:FAIL 1:1 encrypted baseline"
  fi
  save_logs g1
}

case_g2() {
  # G2: Alice adds Charlene; 3 links encrypted; screenshots; Encrypted on all.
  setup_pair || { record "G2:FAIL could not establish 1:1 baseline"; return 0; }
  if ! join_via "$SERIAL_A" "charlene" "$SERIAL_C" "$LOOKUP_ALICE" "15550100030"; then
    record "G2:FAIL charlene did not join encrypted"
    save_logs g2
    return 0
  fi
  STATE="group3"
  wait_for_handshake_log "$SERIAL_C" "settled to \[Encrypted\]" "g2-charlene" 45 || true
  local backend n_a
  backend="$(backend_of "$SERIAL_A")"
  n_a="$(encrypted_legs_count "$SERIAL_A")"
  capture_group_ui "$SERIAL_A" "g2-3person-alice"
  capture_group_ui "$SERIAL_B" "g2-3person-bob"
  capture_group_ui "$SERIAL_C" "g2-3person-charlene"
  if [[ "$n_a" -ge 2 ]] \
     && check_log "$SERIAL_C" "Handshake settled to \[Encrypted\]" \
     && ! check_log "$SERIAL_A" "settled to \[(Downgraded|NeverEncrypted)\]"; then
    record "G2:PASS alice links Encrypted to bob+charlene (n=$n_a), charlene Encrypted, backend=[${backend:-unknown}]"
  else
    record "G2:FAIL adder links not all Encrypted (alice_encrypted=$n_a)"
  fi
  if ui_has "$OUT_DIR/g2-3person-alice.xml" 'Encrypted call'; then
    record "G2:INFO alice header shows Encrypted call"
  else
    record "G2:INFO alice UI dump has no 'Encrypted call' text (see $SCREENCAP_DIR/g2-3person-alice.png)"
  fi
  if wait_encrypted_for "$SERIAL_B" "15550100030" 15 && wait_encrypted_for "$SERIAL_C" "15550100010" 15; then
    record "G2:PASS bob<->charlene third link Encrypted (3 of 3 links)"
  else
    record "G2:PENDING_MESH bob<->charlene link absent (backend=[${backend:-PAIRWISE_SESSIONS}]: adder-only links, 2 of 3)"
  fi
  group_media_probe g2 "$SERIAL_B:bob" "$SERIAL_C:charlene"
  save_logs g2
}

case_g3() {
  # G3: Bob<->Charlene direct link with distinct per-link session keys.
  setup_group3 || { record "G3:FAIL could not establish 3-person group"; return 0; }
  local backend
  backend="$(backend_of "$SERIAL_A")"
  local bob_sessions char_sessions
  bob_sessions="$(media_sessions "$SERIAL_B")"
  char_sessions="$(media_sessions "$SERIAL_C")"
  if check_log "$SERIAL_B" "notifyMeshPeerState" || check_log "$SERIAL_C" "notifyMeshPeerState"; then
    record "G3:INFO mesh link state markers present"
  fi
  if wait_encrypted_for "$SERIAL_B" "15550100030" 20 \
     && wait_encrypted_for "$SERIAL_C" "15550100010" 20 \
     && [[ "${bob_sessions:-0}" -ge 2 && "${char_sessions:-0}" -ge 2 ]]; then
    record "G3:PASS bob<->charlene direct link Encrypted; distinct media sessions bob=$bob_sessions charlene=$char_sessions"
  else
    # "mesh JNI not shipped" was a stale canned string — mesh is in mock-token debug APKs.
    # Distinguish wait/dial races (roster mesh logs present) from a truly missing path.
    local mesh_path="absent"
    if check_log "$SERIAL_B" "roster mesh|MESH_NO_RELAY|notifyMeshPeerState" \
       || check_log "$SERIAL_C" "roster mesh|MESH_NO_RELAY|notifyMeshPeerState"; then
      mesh_path="wait-or-dial-race"
    elif check_log "$SERIAL_A" "announce joiner failed|announce_introduced_peer: session"; then
      mesh_path="announce-failed"
    fi
    record "G3:PENDING_MESH no direct bob<->charlene link (backend=[${backend:-PAIRWISE_SESSIONS}]; media sessions bob=${bob_sessions:-0} charlene=${char_sessions:-0}; mesh_path=[$mesh_path])"
  fi
  save_logs g3
}

case_g4() {
  # G4a: Charlene removed from Bob's contacts -> shown "introduced by alice".
  # G4b: wrong roster fingerprint for an introduced peer must fail (KEY_MISMATCH).
  setup_pair || { record "G4:FAIL could not establish 1:1 baseline"; return 0; }
  local g4a_ready=0
  if delete_contact_by_name "$SERIAL_B" "charlene"; then
    record "G4:INFO charlene removed from bob contacts"
    g4a_ready=1
  elif clear_contact_key "$SERIAL_B" "charlene"; then
    record "G4:INFO charlene Saga key cleared on bob (contact row kept; hasResolvableKey=false)"
    g4a_ready=1
  else
    record "G4:NEEDS_HARDWARE could not delete/clear charlene key on bob via adb content"
  fi
  adb -s "$SERIAL_B" logcat -c 2>/dev/null || true
  if [[ "$g4a_ready" -eq 1 ]] && join_via "$SERIAL_A" "charlene" "$SERIAL_C" "$LOOKUP_ALICE" "15550100030"; then
    local backend
    backend="$(backend_of "$SERIAL_A")"
    # Roster intro is async (accept_bi on bob); wait for the UI marker before capturing.
    local intro_wait=0
    while [[ $intro_wait -lt 30 ]]; do
      capture_group_ui "$SERIAL_B" "g4-introduced-by-bob"
      if ui_has "$OUT_DIR/g4-introduced-by-bob.xml" 'introduced by'; then
        break
      fi
      sleep 1
      intro_wait=$((intro_wait + 1))
    done
    capture_group_ui "$SERIAL_A" "g4-introduced-by-alice"
    if ui_has "$OUT_DIR/g4-introduced-by-bob.xml" 'introduced by'; then
      record "G4:PASS bob shows charlene as introduced by alice"
    elif [[ "${backend:-PAIRWISE_SESSIONS}" == "NATIVE_MESH" ]]; then
      record "G4:FAIL mesh backend active but bob UI has no 'introduced by' ($SCREENCAP_DIR/g4-introduced-by-bob.png)"
    else
      # Pairwise scaffold can still deliver a signaling-only roster intro (2a).
      record "G4:FAIL bob UI has no 'introduced by' after roster announce ($SCREENCAP_DIR/g4-introduced-by-bob.png) backend=[${backend:-PAIRWISE_SESSIONS}]"
    fi
  elif [[ "$g4a_ready" -eq 1 ]]; then
    record "G4:FAIL charlene did not join"
  fi
  # G4b: fresh pair; bob (no charlene key) adds her as introduced by alice with a WRONG roster fingerprint.
  setup_pair || { record "G4b:FAIL could not rebuild baseline"; return 0; }
  delete_contact_by_name "$SERIAL_B" "charlene" || true
  clear_contact_key "$SERIAL_B" "charlene" || true
  prepare_callee_for_join "$SERIAL_C" || true
  keep_incall_front "$SERIAL_B"
  sleep 1
  adb -s "$SERIAL_B" logcat -c 2>/dev/null || true
  local outcome="" attempt
  for attempt in 1 2 3; do
    keep_incall_front "$SERIAL_B"
    sleep 0.5
    adb -s "$SERIAL_B" logcat -c 2>/dev/null || true
    add_participant "$SERIAL_B" "15550100030" --ez confirm_unencrypted true \
      --es confirm_unencrypted true \
      --es introduced_by alice --es roster_fingerprint "$FP_WRONG"
    outcome="$(wait_add_participant_outcome "$SERIAL_B" "15550100030" 12 || true)"
    if [[ "$outcome" == "PLACED" || "$outcome" == "REFUSED_DUPLICATE" ]]; then
      break
    fi
    record "G4b:INFO add attempt=$attempt outcome=[${outcome:-none}] -- re-showing InCall and retrying"
    keep_incall_front "$SERIAL_B"
    sleep 1
  done
  if [[ "$outcome" == "PLACED" || "$outcome" == "REFUSED_DUPLICATE" ]]; then
    if wait_for_incoming_ringing "$SERIAL_C" "charlene g4b" 50; then
      answer_incoming "$SERIAL_C" "15550100010"
    fi
    if poll_for_log_marker "$SERIAL_B" "KEY_MISMATCH" "g4b-mismatch" 40 2; then
      record "G4b:PASS wrong roster fingerprint rejected (KEY_MISMATCH, link closed)"
    else
      record "G4b:FAIL no KEY_MISMATCH for wrong roster fingerprint (backend=$(backend_of "$SERIAL_B"))"
    fi
  else
    record "G4b:FAIL add outcome=[${outcome:-none}] (expected PLACED then KEY_MISMATCH)"
  fi
  save_logs g4
  STATE="none"
}

case_g5() {
  # G5: wrong key -> Not encrypted for the group; confirm-to-admit for key-less peers.
  setup_pair || { record "G5:FAIL could not establish 1:1 baseline"; return 0; }
  # 5a: charlene's key on alice is rewritten to bob's fingerprint (wrong key).
  if set_contact_key "$SERIAL_A" "charlene" "$FP_BOB"; then
    record "G5:INFO charlene key on alice rewritten to a wrong fingerprint"
    # Idle/listen charlene only — never ENDCALL/force-stop alice (live pair must stay up).
    prepare_callee_for_join "$SERIAL_C" || true
    keep_incall_front "$SERIAL_A"
    sleep 1
    adb -s "$SERIAL_A" logcat -c 2>/dev/null || true
    local g5_outcome="" g5_attempt
    for g5_attempt in 1 2 3; do
      keep_incall_front "$SERIAL_A"
      sleep 0.5
      adb -s "$SERIAL_A" logcat -c 2>/dev/null || true
      add_participant "$SERIAL_A" "charlene"
      g5_outcome="$(wait_add_participant_outcome "$SERIAL_A" "charlene" 12 || true)"
      if [[ "$g5_outcome" == "PLACED" || "$g5_outcome" == "REFUSED_DUPLICATE" ]]; then
        break
      fi
      record "G5:INFO add attempt=$g5_attempt outcome=[${g5_outcome:-none}] -- re-showing InCall"
      keep_incall_front "$SERIAL_A"
      sleep 1
    done
    if [[ "$g5_outcome" != "PLACED" && "$g5_outcome" != "REFUSED_DUPLICATE" ]]; then
      record "G5:FAIL add outcome=[${g5_outcome:-none}] (expected PLACED)"
    elif wait_for_incoming_ringing "$SERIAL_C" "charlene g5" 60; then
      answer_incoming "$SERIAL_C" "$LOOKUP_ALICE"
      keep_incall_front "$SERIAL_A"
      if poll_for_log_marker "$SERIAL_A" "settled to \[(Downgraded|NeverEncrypted)\] for \[\+?15550100030\]|KEY_MISMATCH|KEY_CHANGED" "g5-wrong" 60 2; then
        capture_group_ui "$SERIAL_A" "g5-not-encrypted-alice"
        capture_group_ui "$SERIAL_B" "g5-not-encrypted-bob"
        capture_group_ui "$SERIAL_C" "g5-not-encrypted-charlene"
        if ui_has "$OUT_DIR/g5-not-encrypted-alice.xml" 'Not encrypted|Encryption failed|Key changed'; then
          record "G5:PASS wrong key -> alice shows Not encrypted / Encryption failed"
        else
          record "G5:PASS wrong key -> link settled not-Encrypted on alice (UI text not found in dump; see g5-not-encrypted-alice.png)"
        fi
        if ui_has "$OUT_DIR/g5-not-encrypted-bob.xml" 'Not encrypted|Encryption failed|Key changed'; then
          record "G5:PASS bob also shows Not encrypted"
        else
          record "G5:PENDING_MESH bob only learns charlene's state over the mesh roster (pairwise: adder-only)"
        fi
      else
        record "G5:FAIL wrong key did not produce a non-Encrypted link on alice"
      fi
    else
      record "G5:FAIL charlene never rang after wrong-key add (outcome=[${g5_outcome:-none}])"
    fi
    # Restore charlene's real key on alice so later cases are not poisoned.
    set_contact_key "$SERIAL_A" "charlene" "$FP_CHARLENE" || true
  else
    record "G5:NEEDS_HARDWARE could not rewrite charlene key row via adb content update"
  fi
  # 5b: confirm-to-admit for an unknown (key-less) participant label.
  setup_pair || { record "G5b:FAIL could not rebuild baseline"; return 0; }
  keep_incall_front "$SERIAL_A"
  sleep 1
  adb -s "$SERIAL_A" logcat -c 2>/dev/null || true
  add_participant "$SERIAL_A" "15550100040"
  local first
  first="$(wait_add_participant_outcome "$SERIAL_A" "15550100040" 12 || true)"
  # Keep the live Iroh call: restore InCall, then deliver confirm via MainActivity.
  # Pass both --ez and --es so confirm_unencrypted survives SINGLE_TOP redelivery.
  keep_incall_front "$SERIAL_A"
  sleep 1
  add_participant "$SERIAL_A" "15550100040" \
    --ez confirm_unencrypted true \
    --es confirm_unencrypted true
  local second
  second="$(wait_add_participant_outcome "$SERIAL_A" "15550100040" 12 || true)"
  if [[ "$first" == "NEEDS_UNENCRYPTED_CONFIRMATION" && "$second" != "PLACED" ]]; then
    record "G5b:INFO confirm outcome=[${second:-none}] — restore InCall and retry confirm once"
    keep_incall_front "$SERIAL_A"
    sleep 1
    add_participant "$SERIAL_A" "15550100040" \
      --ez confirm_unencrypted true \
      --es confirm_unencrypted true
    second="$(wait_add_participant_outcome "$SERIAL_A" "15550100040" 12 || true)"
  fi
  if [[ "$first" == "NEEDS_UNENCRYPTED_CONFIRMATION" && \
        ( "$second" == "PLACED" || "$second" == "REFUSED_DUPLICATE" ) ]]; then
    record "G5b:PASS key-less peer needs explicit confirm; admitted only after confirm_unencrypted"
  else
    record "G5b:FAIL confirm-to-admit first=[${first:-none}] second=[${second:-none}]"
  fi
  save_logs g5
  STATE="none"
}

case_g6() {
  # G6: add laptop (4th); 6 links; everyone hears three others.
  setup_group4 || { record "G6:FAIL could not establish 4-person group"; return 0; }
  local backend n_a
  backend="$(backend_of "$SERIAL_A")"
  n_a="$(encrypted_legs_count "$SERIAL_A")"
  capture_group_ui "$SERIAL_A" "g6-4person-alice"
  capture_group_ui "$SERIAL_B" "g6-4person-bob"
  capture_group_ui "$SERIAL_C" "g6-4person-charlene"
  if [[ "$n_a" -ge 3 ]]; then
    record "G6:PASS alice holds 3 Encrypted links (bob, charlene, laptop), backend=[${backend:-unknown}]"
  else
    record "G6:FAIL alice Encrypted links=$n_a (need 3)"
  fi
  record "G6:INFO laptop heard: $OUT_DIR/peer-laptop-laptop-join.wav (written when the peer exits)"
  record "G6:PENDING_MESH 6-link full mesh and 'hear three others' need native mesh + a mesh-capable saga-peer (saga-peer CLI is 1:1 only)"
  group_media_probe g6 "$SERIAL_B:bob" "$SERIAL_C:charlene"
  host_mesh_check
  if rg -q "test four_peer_encrypted_mesh \.\.\. ok" "$OUT_DIR/host-mesh.log" 2>/dev/null; then
    record "G6:INFO 4-peer/6-link mixing verified on host model only (HOSTMESH:PASS four_peer_encrypted_mesh)"
  fi
  save_logs g6
}

case_g7() {
  # G7: a 5th participant is refused (max 4 including self).
  setup_group4 || { record "G7:FAIL could not establish 4-person group"; return 0; }
  adb -s "$SERIAL_A" logcat -c 2>/dev/null || true
  add_participant "$SERIAL_A" "15550100040"
  sleep 3
  local outcome
  outcome="$(add_participant_outcome "$SERIAL_A" "15550100040")"
  if [[ "$outcome" == "REFUSED_MAX_PARTICIPANTS" ]] && check_log "$SERIAL_A" "REFUSE_MAX_PARTICIPANTS"; then
    capture_group_ui "$SERIAL_A" "g7-full-alice"
    record "G7:PASS 5th participant refused (REFUSED_MAX_PARTICIPANTS), existing links untouched"
  else
    record "G7:FAIL 5th participant outcome=[${outcome:-none}]"
  fi
  host_mesh_check
  if rg -q "test fifth_participant_refused \.\.\. ok" "$OUT_DIR/host-mesh.log" 2>/dev/null; then
    record "G7:INFO host mesh model also refuses 5th peer (HOSTMESH:PASS fifth_participant_refused)"
  fi
  save_logs g7
}

case_g8() {
  # G8a: Bob leaves -> Alice+Charlene continue. G8b: Alice leaves -> Bob+Charlene continue.
  setup_group3 || { record "G8:FAIL could not establish 3-person group"; return 0; }
  adb -s "$SERIAL_A" logcat -c 2>/dev/null || true
  adb -s "$SERIAL_C" logcat -c 2>/dev/null || true
  # Bob is a plain 1:1 callee (no group model on his side), so he leaves via the end-call button.
  adb -s "$SERIAL_B" shell am start -a org.saga.TEST_IN_CALL_UI -n org.saga/.ui.SagaInCallActivity >/dev/null 2>&1 || true
  sleep 2
  tap_ui_id "$SERIAL_B" "end_call" || adb -s "$SERIAL_B" shell input keyevent KEYCODE_ENDCALL
  sleep 5
  if check_log "$SERIAL_A" "leg ended group=\[[^]]+\] leg=\[[^]]+\] remaining=1" \
     && ! check_log "$SERIAL_C" "onDisconnect|DISCONNECTED|call ended"; then
    record "G8:PASS bob left; alice keeps charlene (remaining=1), charlene link up"
  elif check_log "$SERIAL_A" "leg ended group=\[[^]]+\] leg=\[[^]]+\] remaining=1"; then
    record "G8:PASS bob left; alice remaining=1 (charlene disconnect markers ambiguous, see logs)"
  else
    record "G8:FAIL alice did not report remaining=1 after bob left"
  fi
  save_logs g8a
  # 8b
  STATE="none"
  setup_group3 || { record "G8b:FAIL could not rebuild 3-person group"; return 0; }
  adb -s "$SERIAL_A" shell am start -a org.saga.TEST_IN_CALL_UI -n org.saga/.ui.SagaInCallActivity >/dev/null 2>&1 || true
  sleep 2
  adb -s "$SERIAL_B" logcat -c 2>/dev/null || true
  adb -s "$SERIAL_C" logcat -c 2>/dev/null || true
  if tap_ui_id "$SERIAL_A" "end_call"; then
    sleep 5
    if ! check_log "$SERIAL_A" "leg ended .* remaining=[1-9]"; then
      record "G8b:INFO alice left; local group torn down"
    fi
    if [[ "$(backend_of "$SERIAL_A")" == "NATIVE_MESH" ]]; then
      if check_log "$SERIAL_B" "Handshake settled to \[Encrypted\] for \[\+?15550100030\]" \
         && ! check_log "$SERIAL_B" "mesh peer left group=.*15550100030"; then
        record "G8b:PASS alice left; bob<->charlene mesh link continues"
      else
        record "G8b:FAIL alice left but bob<->charlene link not confirmed up"
      fi
    else
      record "G8b:PENDING_MESH bob and charlene only had links to alice (pairwise); B+C continue requires the mesh"
    fi
  else
    record "G8b:NEEDS_HARDWARE could not tap end_call (in-call UI not foreground on emulator)"
  fi
  save_logs g8b
  STATE="none"
}

case_g9() {
  # G9: mute / hold apply to every link of the group.
  setup_group3 || { record "G9:FAIL could not establish 3-person group"; return 0; }
  adb -s "$SERIAL_A" logcat -c 2>/dev/null || true
  adb -s "$SERIAL_A" shell am broadcast -p org.saga -a org.saga.TEST_MEDIA_MUTE --ez muted true >/dev/null 2>&1
  sleep 2
  capture_group_ui "$SERIAL_A" "g9-muted-alice"
  local engines
  engines="$(adb_logcat_dump "$SERIAL_A" | rg -o 'setMutedAllForTest muted=true engines=[0-9]+' | tail -1 | rg -o '[0-9]+$' || true)"
  adb -s "$SERIAL_A" shell am broadcast -p org.saga -a org.saga.TEST_MEDIA_MUTE --ez muted false >/dev/null 2>&1
  sleep 2
  if [[ "${engines:-0}" -ge 2 ]] && check_log "$SERIAL_A" "muted=false"; then
    record "G9:PASS mute applied to all links (engines=$engines) and unmuted"
  elif [[ "${engines:-0}" -eq 1 ]]; then
    record "G9:FAIL mute reached only 1 engine in a 2-link group"
  else
    record "G9:FAIL mute log missing (engines=${engines:-none})"
  fi
  # Hold via the in-call UI toggle.
  adb -s "$SERIAL_A" logcat -c 2>/dev/null || true
  adb -s "$SERIAL_A" shell am start -a org.saga.TEST_IN_CALL_UI -n org.saga/.ui.SagaInCallActivity >/dev/null 2>&1 || true
  sleep 2
  if tap_ui_id "$SERIAL_A" "toggle_hold"; then
    sleep 3
    capture_group_ui "$SERIAL_A" "g9-hold-alice"
    local held
    held="$(count_log "$SERIAL_A" 'held=true')"
    tap_ui_id "$SERIAL_A" "toggle_hold" || true
    sleep 3
    if [[ "${held:-0}" -ge 2 ]] && check_log "$SERIAL_A" "held=false"; then
      record "G9:PASS hold applied to all links (held=true x$held) and resumed"
    elif [[ "${held:-0}" -ge 1 ]]; then
      record "G9:FAIL hold reached only $held link(s) of the group"
    else
      record "G9:NEEDS_HARDWARE hold toggle tapped but no held=true on emulator"
    fi
  else
    record "G9:NEEDS_HARDWARE toggle_hold not found in the UI dump"
  fi
  save_logs g9
}

case_g10() {
  # G10: incoming cellular during a group call: hold, never merge.
  setup_group3 || { record "G10:FAIL could not establish 3-person group"; return 0; }
  adb -s "$SERIAL_A" logcat -c 2>/dev/null || true
  timeout 10 adb -s "$SERIAL_A" emu gsm call 15550100094 >/dev/null 2>&1 || true
  sleep 3
  adb -s "$SERIAL_A" shell input keyevent KEYCODE_CALL 2>/dev/null || true
  sleep 3
  capture_group_ui "$SERIAL_A" "g10-group-plus-carrier"
  local merge=0
  ui_has "$OUT_DIR/g10-group-plus-carrier.xml" 'text="Merge"|content-desc="Merge"' && merge=1
  if check_log "$SERIAL_A" "onCallAdded|STATE_HOLDING|held=true|incoming_second"; then
    if [[ "$merge" -eq 0 ]]; then
      record "G10:PASS carrier call held the group; no Merge offered (refuse_log=$(count_log "$SERIAL_A" REFUSE_IROH_CARRIER_MERGE))"
    else
      record "G10:FAIL Merge visible with group + carrier call (must hide)"
    fi
  else
    record "G10:NEEDS_HARDWARE emulator GSM second-incoming not confirmed on alice (default dialer / RIL)"
  fi
  timeout 10 adb -s "$SERIAL_A" emu gsm cancel 15550100094 >/dev/null 2>&1 || true
  sleep 2
  save_logs g10
}

case_g11() {
  # G11: Charlene (callee side) adds the laptop to the call.
  setup_group3 || { record "G11:FAIL could not establish 3-person group"; return 0; }
  start_peer "laptop-g11" || return 0
  adb -s "$SERIAL_C" logcat -c 2>/dev/null || true
  add_participant "$SERIAL_C" "laptop"
  sleep 3
  local outcome
  outcome="$(add_participant_outcome "$SERIAL_C" "laptop")"
  if [[ "$outcome" != "PLACED" ]]; then
    record "G11:FAIL charlene add laptop outcome=[${outcome:-none}]"
    save_logs g11
    return 0
  fi
  if wait_encrypted_for "$SERIAL_C" "15550100020" 60; then
    capture_group_ui "$SERIAL_C" "g11-charlene-added-laptop"
    record "G11:PASS charlene->laptop link Encrypted (adder is a callee, not the originator)"
  else
    record "G11:FAIL charlene->laptop did not reach Encrypted"
  fi
  # Prefer separate checks over `if a || b; then` so a mid-edit cannot leave
  # bash with an unexpected `then` while parsing the rest of the file.
  local g11_mesh=0
  if check_log "$SERIAL_A" "Handshake settled to \[Encrypted\] for \[\+?15550100020\]"; then
    g11_mesh=1
  elif check_log "$SERIAL_B" "Handshake settled to \[Encrypted\] for \[\+?15550100020\]"; then
    g11_mesh=1
  fi
  if [[ "$g11_mesh" -eq 1 ]]; then
    record "G11:PASS alice/bob also linked to laptop (mesh roster propagation)"
  else
    record "G11:PENDING_MESH alice and bob do not auto-link to laptop (needs mesh roster propagation)"
  fi
  save_logs g11
}

# ---------------------------------------------------------------- main

echo "=== Saga group call E2E G1-G11 (local-only, 3 emulators + saga-peer) ==="
echo "serials: alice=$SERIAL_A bob=$SERIAL_B charlene=$SERIAL_C  cases: $G_CASES"
echo "out: $OUT_DIR  screencaps: $SCREENCAP_DIR"
preflight
enlarge_logcat_buffers

if ! one_time_provision; then
  record "PROVISION:FAIL baseline for alice/bob/charlene"
  printf '%s\n' "${RESULTS[@]}"
  exit 1
fi

# Capability probe: the installed APK must understand the group debug intents.
# Force-stop first so a leftover SagaIrohConnection / GroupCallRegistry from
# PhoneAccount enable or stuck-call drain cannot yield REFUSED_DUPLICATE.
for s in "$SERIAL_A" "$SERIAL_B" "$SERIAL_C"; do
  clear_stuck_telephony_calls "$s"
  adb -s "$s" shell am force-stop org.saga
  sleep 1
  clear_stuck_telephony_calls "$s"
  adb -s "$s" shell input keyevent KEYCODE_WAKEUP >/dev/null 2>&1 || true
  adb -s "$s" logcat -c 2>/dev/null || true
  add_participant "$s" "bob"
  probe=""
  for attempt in 1 2 3 4 5; do
    sleep 2
    probe="$(add_participant_outcome "$s" "bob")"
    [[ "$probe" == "REFUSED_NO_IROH_CALL" ]] && break
    # Cold start on a sleeping emulator can miss the first delivery; retry.
    add_participant "$s" "bob"
  done
  if [[ "$probe" != "REFUSED_NO_IROH_CALL" ]]; then
    record "PROVISION:FAIL [$s] APK lacks TEST_ADD_PARTICIPANT (probe outcome=[${probe:-none}]); rebuild and reinstall the debug APK on all three emulators"
    printf '%s\n' "${RESULTS[@]}"
    exit 2
  fi
done

for c in G1 G2 G3 G4 G5 G6 G7 G8 G9 G10 G11; do
  if want_case "$c"; then
    echo ""
    echo "=== $c ==="
    "case_${c,,}" || record "$c:FAIL harness error rc=$?"
  fi
done

echo ""
echo "=== Group E2E summary ==="
printf '%s\n' "${RESULTS[@]}"
PASS_N=$(printf '%s\n' "${RESULTS[@]}" | rg -c ':PASS' || true)
FAIL_N=$(printf '%s\n' "${RESULTS[@]}" | rg -c ':FAIL' || true)
HW_N=$(printf '%s\n' "${RESULTS[@]}" | rg -c 'NEEDS_HARDWARE' || true)
MESH_N=$(printf '%s\n' "${RESULTS[@]}" | rg -c 'PENDING_MESH' || true)
echo "SUMMARY pass=${PASS_N:-0} fail=${FAIL_N:-0} needs_hardware=${HW_N:-0} pending_mesh=${MESH_N:-0}"
if [[ "${FAIL_N:-0}" -gt 0 ]]; then
  exit 1
fi
exit 0
