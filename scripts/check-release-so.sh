#!/usr/bin/env bash
# Fail if a release-profile arm64 libsaga_iroh_core.so contains mock/dev symbols.
# Also require the production Nitrokey backend marker (Part 4).
#
# Uses grep (not ripgrep): CI runners may not have `rg`. Avoid `grep -q` under
# `pipefail` — early exit SIGPIPEs `strings` and can flip match results.
set -euo pipefail

SO="${1:?usage: check-release-so.sh path/to/libsaga_iroh_core.so}"
test -f "$SO"

fail=0

so_contains() {
  local needle="$1"
  strings "$SO" | grep -F -- "$needle" >/dev/null
}

nm_contains() {
  local needle="$1"
  command -v nm >/dev/null 2>&1 || return 1
  nm -D "$SO" 2>/dev/null | grep -F -- "$needle" >/dev/null
}

check_absent() {
  local needle="$1"
  if so_contains "$needle"; then
    echo "FAIL: release .so contains forbidden string: $needle"
    fail=1
  else
    echo "OK: absent [$needle]"
  fi
  if nm_contains "$needle"; then
    echo "FAIL: release .so nm -D contains: $needle"
    fail=1
  fi
}

check_absent "MockSessionToken"
check_absent "saga-dev-v1"
check_absent "insecure_skip_verify"
check_absent "saga-synthetic-audio-v1"
# Fake HAL / test-hal must never ship in release (production uses OsTrng).
check_absent "FakeTrng"
check_absent "FakeVaultStorage"
check_absent "FakeMonotonicCounter"
check_absent "FakeZeroiseController"
check_absent "FakeRebootController"
check_absent "test-hal"
check_absent "fake_hal"
# Debug-only media helpers (Phase 2 close-out).
check_absent "saga-echo-mode"
check_absent "saga-rx-fault"
# Debug PCM / E2E markers must not appear in release .so (Part 5).
check_absent "LATENCY_MARKER"
check_absent "TEST_PCM"
check_absent "TEST_ECHO"
check_absent "TEST_RX_FAULT"

# Production Nitrokey backend must be present in release (Part 4).
if so_contains "saga-nitrokey-session-token-v1"; then
  echo "OK: present [saga-nitrokey-session-token-v1]"
else
  echo "FAIL: release .so missing production Nitrokey backend marker"
  fail=1
fi

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi
echo "OK: release .so mock/dev hygiene check passed"
