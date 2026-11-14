#!/usr/bin/env bash
# Host↔host public N0 relay soak using two saga-peer processes (no emulator NAT).
# Usage:
#   bash testing/e2e/run-public-relay-peer-soak.sh
#   DIR=a2b N=10 bash testing/e2e/run-public-relay-peer-soak.sh
# Reports rings/connects N/N, discovery/connect timings, relay URLs, errors.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PEER_BIN="${PEER_BIN:-$REPO_ROOT/tools/saga-peer/target/release/saga-peer}"
OUT_DIR="${OUT_DIR:-/tmp/saga-public-relay-peer-soak}"
N="${N:-10}"
# a2b = alice calls bob; b2a = bob calls alice; both = both directions
DIR="${DIR:-both}"
WAV_IN="${WAV_IN:-$REPO_ROOT/saga-media/fixtures/speech_clip_48k.wav}"
MAX_SECS="${MAX_SECS:-8}"

mkdir -p "$OUT_DIR" /tmp/saga-dns
: >"$OUT_DIR/summary.txt"
# Some ISP "safe browsing" resolvers (e.g. Telenor Nettvern) hijack *.relay.n0.iroh.link
# to a captive block page → TLS UnknownIssuer and online() never completes. Prefer
# Cloudflare/Google DNS for this soak when bwrap is available.
printf 'nameserver 1.1.1.1\nnameserver 8.8.8.8\n' >/tmp/saga-dns/resolv.conf
USE_BWRAP_DNS="${USE_BWRAP_DNS:-auto}"
if [[ "$USE_BWRAP_DNS" == "auto" ]]; then
  if command -v bwrap >/dev/null 2>&1; then
    USE_BWRAP_DNS=1
  else
    USE_BWRAP_DNS=0
  fi
fi

run_peer() {
  # run_peer <args...> — optionally wrap in bwrap with clean DNS
  if [[ "$USE_BWRAP_DNS" == "1" ]]; then
    bwrap --dev-bind / / --ro-bind /tmp/saga-dns/resolv.conf /etc/resolv.conf -- "$@"
  else
    "$@"
  fi
}

if [[ ! -x "$PEER_BIN" ]]; then
  echo "building saga-peer…"
  cargo build --release --manifest-path "$REPO_ROOT/tools/saga-peer/Cargo.toml"
fi

echo "INFO: USE_BWRAP_DNS=$USE_BWRAP_DNS (bypass ISP DNS hijack of N0 relays)" | tee -a "$OUT_DIR/summary.txt"

run_one() {
  local tag="$1" caller_id="$2" callee_id="$3" peer_arg="$4"
  local listen_log="$OUT_DIR/${tag}-listen.log"
  local call_log="$OUT_DIR/${tag}-call.log"
  : >"$listen_log"
  : >"$call_log"

  RUST_LOG=info run_peer "$PEER_BIN" listen \
    --identity "$callee_id" \
    --public-relay \
    --wav-in "$WAV_IN" \
    --wav-out "$OUT_DIR/${tag}-callee-out.wav" \
    --loop-wav \
    --max-secs "$MAX_SECS" \
    >"$listen_log" 2>&1 &
  local listen_pid=$!

  # Wait until listener reports relay online (+ pkarr settle).
  local ready=0
  for _ in $(seq 1 120); do
    if rg -q 'relay online after|pkarr publish settle done|ready to accept' "$listen_log" 2>/dev/null; then
      ready=1
      break
    fi
    if ! kill -0 "$listen_pid" 2>/dev/null; then
      break
    fi
    sleep 0.25
  done
  if [[ "$ready" -ne 1 ]]; then
    echo "$tag:FAIL listener never ready"
    kill "$listen_pid" 2>/dev/null || true
    wait "$listen_pid" 2>/dev/null || true
    return 1
  fi

  local t0
  t0=$(date +%s%3N)
  set +e
  RUST_LOG=info run_peer "$PEER_BIN" call \
    --identity "$caller_id" \
    --peer "$peer_arg" \
    --public-relay \
    --wav-in "$WAV_IN" \
    --wav-out "$OUT_DIR/${tag}-caller-out.wav" \
    --loop-wav \
    --max-secs "$MAX_SECS" \
    >"$call_log" 2>&1
  local call_rc=$?
  set -e
  local t1
  t1=$(date +%s%3N)
  local wall_ms=$((t1 - t0))

  # Allow listener to finish / be killed.
  sleep 1
  kill "$listen_pid" 2>/dev/null || true
  wait "$listen_pid" 2>/dev/null || true

  local home_listen home_call connect_ms security err online_ms
  home_listen=$(rg -o 'home=\[([^]]*)\]' -r '$1' "$listen_log" | head -1 || true)
  home_call=$(rg -o 'home=\[([^]]*)\]' -r '$1' "$call_log" | head -1 || true)
  online_ms=$(rg -o 'relay online after ([^ ]+)' -r '$1' "$listen_log" | head -1 || true)
  connect_ms=$(rg -o 'voice connected in ([^ ]+)' -r '$1' "$call_log" | head -1 || true)
  security=$(rg -o 'done security=(\S+)' -r '$1' "$call_log" | head -1 || true)
  err=$(rg -i 'error|failed|NOT_REACHABLE|No addressing|UnknownIssuer' "$call_log" | head -3 | tr '\n' '|' || true)

  if [[ "$call_rc" -eq 0 ]] && rg -q 'Encrypted|voice connected' "$call_log"; then
    echo "$tag:PASS wall_ms=$wall_ms connect=$connect_ms online=$online_ms security=${security:-?} listen_home=$home_listen call_home=$home_call"
    return 0
  fi
  echo "$tag:FAIL rc=$call_rc wall_ms=$wall_ms listen_home=$home_listen call_home=$home_call err=${err:0:200}"
  return 1
}

pass=0
fail=0
echo "=== public-relay peer soak N=$N DIR=$DIR ===" | tee -a "$OUT_DIR/summary.txt"

run_dir() {
  local prefix="$1" caller="$2" callee="$3" peer="$4"
  local i
  for i in $(seq 1 "$N"); do
    local tag="${prefix}-$(printf '%02d' "$i")"
    local line
    if line=$(run_one "$tag" "$caller" "$callee" "$peer"); then
      pass=$((pass + 1))
    else
      fail=$((fail + 1))
      line="${line:-$tag:FAIL}"
    fi
    echo "$line" | tee -a "$OUT_DIR/summary.txt"
  done
}

case "$DIR" in
  a2b)
    run_dir a2b 15550100011 15550100010 15550100010
    ;;
  b2a)
    run_dir b2a 15550100010 15550100011 15550100011
    ;;
  both)
    run_dir a2b 15550100011 15550100010 15550100010
    run_dir b2a 15550100010 15550100011 15550100011
    ;;
  *)
    echo "DIR must be a2b|b2a|both"
    exit 2
    ;;
esac

total=$((pass + fail))
echo "RESULT pass=$pass/$total fail=$fail" | tee -a "$OUT_DIR/summary.txt"
# Timings digest
echo "=== connect timings ===" | tee -a "$OUT_DIR/summary.txt"
rg 'voice connected in' "$OUT_DIR"/*-call.log 2>/dev/null | tee -a "$OUT_DIR/summary.txt" || true
echo "=== home relays ===" | tee -a "$OUT_DIR/summary.txt"
rg 'relay online after' "$OUT_DIR"/*.log 2>/dev/null | tee -a "$OUT_DIR/summary.txt" || true

[[ "$fail" -eq 0 ]]
