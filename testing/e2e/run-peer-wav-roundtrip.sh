#!/usr/bin/env bash
# Emulator ↔ saga-peer (laptop 15550100020) WAV round-trip both directions.
# Real RoQ network media; peer uses --wav-in/--wav-out (no sound card).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
E2E_DIR="$(cd "$(dirname "$0")" && pwd)"
record() { echo "$1"; }
# shellcheck source=e2e-common.sh
source "$E2E_DIR/e2e-common.sh"

SERIAL="${SERIAL:-emulator-5554}"
SPEECH="$REPO_ROOT/saga-media/fixtures/speech_clip_48k.wav"
OUT_DIR="${OUT_DIR:-/tmp/saga-peer-wav}"
mkdir -p "$OUT_DIR"
APP_SPEECH="/data/data/org.saga/files/speech_clip_48k.wav"
APP_DUMP="/data/data/org.saga/files/saga_play_peer.pcm"
DEVICE_DUMP_REL="files/saga_play_peer.pcm"
RELAY_URL="${RELAY_URL:-http://127.0.0.1:3340}"
# Resolve binary: explicit PEER_BIN, else cargo target next to crate, else CARGO_TARGET_DIR.
if [[ -z "${PEER_BIN:-}" ]]; then
  if [[ -x "$REPO_ROOT/tools/saga-peer/target/release/saga-peer" ]]; then
    PEER_BIN="$REPO_ROOT/tools/saga-peer/target/release/saga-peer"
  elif [[ -n "${CARGO_TARGET_DIR:-}" && -x "$CARGO_TARGET_DIR/release/saga-peer" ]]; then
    PEER_BIN="$CARGO_TARGET_DIR/release/saga-peer"
  else
    PEER_BIN=""
  fi
fi

pcm_broadcast() {
  local action="$1"
  shift
  adb -s "$SERIAL" shell am broadcast -p org.saga -a "$action" "$@" >/dev/null
}

pull_app_file() {
  local rel="$1" host="$2"
  adb -s "$SERIAL" exec-out run-as org.saga cat "$rel" >"$host"
}

kill_peer() {
  if [[ -n "${PEER_PID:-}" ]] && kill -0 "$PEER_PID" 2>/dev/null; then
    kill "$PEER_PID" 2>/dev/null || true
    wait "$PEER_PID" 2>/dev/null || true
  fi
  PEER_PID=""
}
trap kill_peer EXIT

score_wav() {
  local label="$1" deg="$2"
  local deg_raw
  if [[ "$deg" == *.wav ]]; then
    deg_raw="${deg%.wav}.pcm"
    python3 -c "
import wave, sys
from pathlib import Path
w = wave.open(sys.argv[1], 'rb')
Path(sys.argv[2]).write_bytes(w.readframes(w.getnframes()))
" "$deg" "$deg_raw"
  else
    deg_raw="$deg"
  fi
  python3 "$E2E_DIR/pcm_score.py" --ref "$SPEECH" --deg-raw "$deg_raw" --label "$label" \
    --out-wav "${deg_raw%.pcm}.scored.wav"
}

ensure_local_relay

if [[ -z "$PEER_BIN" || ! -x "$PEER_BIN" ]]; then
  echo "=== Building saga-peer ==="
  cargo build --release --manifest-path "$REPO_ROOT/tools/saga-peer/Cargo.toml"
  if [[ -x "$REPO_ROOT/tools/saga-peer/target/release/saga-peer" ]]; then
    PEER_BIN="$REPO_ROOT/tools/saga-peer/target/release/saga-peer"
  elif [[ -n "${CARGO_TARGET_DIR:-}" && -x "$CARGO_TARGET_DIR/release/saga-peer" ]]; then
    PEER_BIN="$CARGO_TARGET_DIR/release/saga-peer"
  else
    echo "ERROR: saga-peer binary not found after build; set PEER_BIN=" >&2
    exit 1
  fi
fi

bash "$E2E_DIR/seed-test-contacts.sh" "$SERIAL"
adb -s "$SERIAL" push "$SPEECH" /data/local/tmp/speech_clip_48k.wav >/dev/null
adb -s "$SERIAL" shell run-as org.saga cp /data/local/tmp/speech_clip_48k.wav files/speech_clip_48k.wav

echo ""
echo "=== Dir 1: peer listens (wav-in loop) ← alice injects speech ==="
PEER_OUT1="$OUT_DIR/peer_from_alice.wav"
rm -f "$PEER_OUT1"
"$PEER_BIN" listen --identity 15550100020 --relay "$RELAY_URL" \
  --wav-in "$SPEECH" --wav-out "$PEER_OUT1" --loop-wav \
  >"$OUT_DIR/peer-listen1.log" 2>&1 &
PEER_PID=$!
sleep 2

ensure_callee_listening "$SERIAL"
warm_encrypted_caller "$SERIAL" || true
adb -s "$SERIAL" push "$SPEECH" /data/local/tmp/speech_clip_48k.wav >/dev/null
adb -s "$SERIAL" shell run-as org.saga cp /data/local/tmp/speech_clip_48k.wav files/speech_clip_48k.wav
pcm_broadcast org.saga.TEST_PCM_DUMP --es path "$APP_DUMP"
pcm_broadcast org.saga.TEST_PCM_INJECT --es path "$APP_SPEECH"
adb -s "$SERIAL" logcat -c 2>/dev/null || true
pcm_broadcast org.saga.TEST_PCM_DUMP --es path "$APP_DUMP"
pcm_broadcast org.saga.TEST_PCM_INJECT --es path "$APP_SPEECH"
adb -s "$SERIAL" shell am start -n org.saga/.ui.MainActivity \
  -a org.saga.TEST_CONTACT_CALL --es contact_name laptop

wait_for_handshake_log "$SERIAL" "settled to \[Encrypted\]" "enc-peer1" 60 || true
wait_for_handshake_log "$SERIAL" "\\[Saga Media\\] start session=" "media-peer1" 30 || true
sleep 6
adb -s "$SERIAL" shell am start -a org.saga.TEST_IN_CALL_UI -n org.saga/.ui.SagaInCallActivity >/dev/null 2>&1 || true
sleep 1
adb -s "$SERIAL" shell screencap -p /sdcard/saga_alice_peer.png >/dev/null 2>&1 || true
adb -s "$SERIAL" pull /sdcard/saga_alice_peer.png "$OUT_DIR/alice-peer-active.png" >/dev/null 2>&1 || true
pcm_broadcast org.saga.TEST_PCM_DUMP_FINISH
stop_caller "$SERIAL" || true
sleep 2
kill_peer
sleep 1

if [[ -f "$PEER_OUT1" ]]; then
  score_wav "alice_to_peer" "$PEER_OUT1"
else
  echo "FAIL: missing $PEER_OUT1"
  cat "$OUT_DIR/peer-listen1.log" || true
fi
pull_app_file "$DEVICE_DUMP_REL" "$OUT_DIR/peer_to_alice.pcm" || true
if [[ -f "$OUT_DIR/peer_to_alice.pcm" ]] && [[ -s "$OUT_DIR/peer_to_alice.pcm" ]]; then
  python3 "$E2E_DIR/pcm_score.py" --ref "$SPEECH" --deg-raw "$OUT_DIR/peer_to_alice.pcm" \
    --label "peer_to_alice" --out-wav "$OUT_DIR/peer_to_alice.wav"
fi

echo ""
echo "=== Dir 2: peer calls alice (wav-in) while alice dumps ==="
ensure_callee_listening "$SERIAL"
wait_for_relay_online "$SERIAL" "alice-listen" || true
pcm_broadcast org.saga.TEST_PCM_DUMP --es path "$APP_DUMP"
pcm_broadcast org.saga.TEST_PCM_INJECT

PEER_OUT2="$OUT_DIR/peer_from_alice_call.wav"
rm -f "$PEER_OUT2"
"$PEER_BIN" call --identity 15550100020 --peer 15550100011 --relay "$RELAY_URL" \
  --wav-in "$SPEECH" --wav-out "$PEER_OUT2" --loop-wav \
  >"$OUT_DIR/peer-call2.log" 2>&1 &
PEER_PID=$!

wait_for_incoming_ringing "$SERIAL" "alice-from-peer" 60 || true
answer_incoming "$SERIAL" "15550100020"
wait_for_handshake_log "$SERIAL" "\\[Saga Media\\] start session=" "media-peer2" 45 || true
sleep 6
adb -s "$SERIAL" shell am start -a org.saga.TEST_IN_CALL_UI -n org.saga/.ui.SagaInCallActivity >/dev/null 2>&1 || true
sleep 1
adb -s "$SERIAL" shell screencap -p /sdcard/saga_alice_peer2.png >/dev/null 2>&1 || true
adb -s "$SERIAL" pull /sdcard/saga_alice_peer2.png "$OUT_DIR/alice-peer-inbound-active.png" >/dev/null 2>&1 || true
pcm_broadcast org.saga.TEST_PCM_DUMP_FINISH
stop_caller "$SERIAL" || true
sleep 2
kill_peer
sleep 1

pull_app_file "$DEVICE_DUMP_REL" "$OUT_DIR/peer_to_alice_inbound.pcm" || true
if [[ -f "$OUT_DIR/peer_to_alice_inbound.pcm" ]] && [[ -s "$OUT_DIR/peer_to_alice_inbound.pcm" ]]; then
  python3 "$E2E_DIR/pcm_score.py" --ref "$SPEECH" --deg-raw "$OUT_DIR/peer_to_alice_inbound.pcm" \
    --label "peer_to_alice_inbound" --out-wav "$OUT_DIR/peer_to_alice_inbound.wav"
fi
if [[ -f "$PEER_OUT2" ]]; then
  score_wav "alice_mic_to_peer" "$PEER_OUT2"
fi

echo ""
echo "=== Peer WAV artifacts in $OUT_DIR ==="
ls -la "$OUT_DIR"
trap - EXIT
