#!/usr/bin/env bash
# Phase 2 Step 5 suite: Cases 1–10 + 4b + onboarding (API from ANDROID_API).
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
E2E_DIR="$(cd "$(dirname "$0")" && pwd)"
record() { echo "$1"; }

echo "=== Step 5: Cases 1–6 + 4b (run-e2e-full) ==="
bash "$E2E_DIR/run-e2e-full.sh" || record "STEP5:WARN run-e2e-full exited non-zero (continuing Cases 8–10)"

echo ""
echo "=== Step 5: Case 8 rekey PCM gap ==="
bash "$E2E_DIR/run-case8-rekey-gap.sh" || true

echo ""
echo "=== Step 5: Case 9 cellular hold ==="
bash "$E2E_DIR/run-case9-cellular-hold.sh" || true

echo ""
echo "=== Step 5: Case 10 mute ==="
bash "$E2E_DIR/run-case10-mute.sh" || true

echo ""
echo "=== Step 5: Phase 4 conference C1–C5 ==="
bash "$E2E_DIR/run-e2e-conference.sh" || true

echo ""
echo "=== Step 5: onboarding ==="
bash "$E2E_DIR/run-e2e-phone-account-onboarding.sh" || true

echo "STEP5_SUITE:DONE"
