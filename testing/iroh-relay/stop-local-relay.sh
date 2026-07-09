#!/usr/bin/env bash
set -euo pipefail
PID_FILE="${SAGA_IROH_RELAY_PID_FILE:-/tmp/saga-iroh-relay.pid}"
if [[ -f "$PID_FILE" ]]; then
  pid="$(cat "$PID_FILE" 2>/dev/null || true)"
  if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
    kill "$pid" 2>/dev/null || true
    echo "stopped iroh-relay pid=$pid"
  fi
  rm -f "$PID_FILE"
fi
