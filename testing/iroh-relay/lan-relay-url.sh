#!/usr/bin/env bash
# Print http://<LAN-IP>:3340 for shared host+emulator relay advertisement.
# Prefer a non-loopback IPv4 with a default route; override with SAGA_IROH_RELAY_LAN_IP.
set -euo pipefail

PORT="${SAGA_IROH_RELAY_PORT:-3340}"
if [[ -n "${SAGA_IROH_RELAY_LAN_IP:-}" ]]; then
  echo "http://${SAGA_IROH_RELAY_LAN_IP}:${PORT}"
  exit 0
fi

ip="$(ip -4 route get 1.1.1.1 2>/dev/null | awk '{for (i = 1; i <= NF; i++) if ($i == "src") { print $(i + 1); exit }}')"
if [[ -z "$ip" || "$ip" == "127.0.0.1" ]]; then
  ip="$(hostname -I 2>/dev/null | awk '{print $1}')"
fi
if [[ -z "$ip" || "$ip" == "127.0.0.1" ]]; then
  echo "ERROR: could not determine LAN IP; set SAGA_IROH_RELAY_LAN_IP" >&2
  exit 1
fi
echo "http://${ip}:${PORT}"
