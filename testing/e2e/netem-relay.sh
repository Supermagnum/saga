#!/usr/bin/env bash
# Apply / clear tc netem on the host iface that carries the local iroh-relay.
# Usage: netem-relay.sh apply|clear [IFACE]
# Default: 5% loss + 30 ms delay (jitter approx via delay distribution).
set -euo pipefail
ACTION="${1:-apply}"
IFACE="${2:-}"
if [[ -z "$IFACE" ]]; then
  IFACE="$(ip route get 1.1.1.1 2>/dev/null | awk '{for(i=1;i<=NF;i++) if($i=="dev"){print $(i+1); exit}}' || true)"
fi
IFACE="${IFACE:-lo}"

case "$ACTION" in
  apply)
    sudo tc qdisc replace dev "$IFACE" root netem delay 30ms 10ms distribution normal loss 5%
    echo "NETEM:APPLY iface=$IFACE delay=30ms±10ms loss=5%"
    ;;
  clear)
    sudo tc qdisc del dev "$IFACE" root 2>/dev/null || true
    echo "NETEM:CLEAR iface=$IFACE"
    ;;
  *)
    echo "usage: $0 apply|clear [IFACE]" >&2
    exit 2
    ;;
esac
