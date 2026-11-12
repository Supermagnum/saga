#!/usr/bin/env bash
# Start a local iroh-relay in --dev mode (HTTP on port 3340).
# --dev binds [::]:3340 (all interfaces). Advertise the host LAN IP so
# saga-peer and Android emulators share one relay URL (no 127.0.0.1 / 10.0.2.2
# in EndpointAddr advertisements).
set -euo pipefail

PORT="${SAGA_IROH_RELAY_PORT:-3340}"
PID_FILE="${SAGA_IROH_RELAY_PID_FILE:-/tmp/saga-iroh-relay.pid}"
LOG_FILE="${SAGA_IROH_RELAY_LOG:-/tmp/saga-iroh-relay.log}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
LAN_URL="$("$SCRIPT_DIR/lan-relay-url.sh")"
echo "$LAN_URL" > /tmp/saga-iroh-relay-url.txt

if [[ -f "$PID_FILE" ]]; then
  old_pid="$(cat "$PID_FILE" 2>/dev/null || true)"
  if [[ -n "$old_pid" ]] && kill -0 "$old_pid" 2>/dev/null; then
    echo "iroh-relay already running (pid=$old_pid) advertise=$(cat /tmp/saga-iroh-relay-url.txt)"
    exit 0
  fi
fi

if ! command -v iroh-relay >/dev/null 2>&1 && [[ ! -x "$HOME/.cargo/bin/iroh-relay" ]]; then
  echo "Installing iroh-relay 1.0 (server feature) via cargo ..."
  cargo install iroh-relay --version 1.0.2 --features server --locked
fi

IROH_RELAY_BIN="$(command -v iroh-relay || echo "$HOME/.cargo/bin/iroh-relay")"
if [[ ! -x "$IROH_RELAY_BIN" ]]; then
  echo "iroh-relay binary not found at $IROH_RELAY_BIN" >&2
  exit 1
fi

# Explicit bind to all interfaces (0.0.0.0) so LAN peers + emulators can connect.
# --dev still disables TLS.
CONFIG_FILE="${SAGA_IROH_RELAY_CONFIG:-/tmp/saga-iroh-relay.toml}"
cat >"$CONFIG_FILE" <<EOF
enable_relay = true
enable_quic_addr_discovery = false
http_bind_addr = "0.0.0.0:${PORT}"
EOF

echo "Starting iroh-relay --dev bind=0.0.0.0:${PORT} advertise=${LAN_URL} log: $LOG_FILE"
nohup "$IROH_RELAY_BIN" --dev --config-path "$CONFIG_FILE" >"$LOG_FILE" 2>&1 &
echo $! >"$PID_FILE"
sleep 2

if ! kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
  echo "iroh-relay failed to start; see $LOG_FILE" >&2
  tail -20 "$LOG_FILE" >&2 || true
  exit 1
fi

for _ in 1 2 3 4 5 6 7 8 9 10; do
  if curl -sf --max-time 1 "http://127.0.0.1:${PORT}/" >/dev/null 2>&1 \
     || curl -sf --max-time 1 "http://127.0.0.1:${PORT}/relay" >/dev/null 2>&1 \
     || curl -sf --max-time 1 "${LAN_URL}/" >/dev/null 2>&1; then
    echo "iroh-relay ready at ${LAN_URL} (bound 0.0.0.0:${PORT})"
    exit 0
  fi
  sleep 1
done

echo "iroh-relay process up (pid=$(cat "$PID_FILE")) but HTTP probe inconclusive; continuing" >&2
echo "advertise url: ${LAN_URL}"
