#!/usr/bin/env bash
# Fail if a release-profile arm64 libsaga_iroh_core.so contains mock/dev symbols.
set -euo pipefail

SO="${1:?usage: check-release-so.sh path/to/libsaga_iroh_core.so}"
test -f "$SO"

fail=0
check_absent() {
  local needle="$1"
  if strings "$SO" | rg -q --fixed-strings "$needle"; then
    echo "FAIL: release .so contains forbidden string: $needle"
    fail=1
  else
    echo "OK: absent [$needle]"
  fi
  if command -v nm >/dev/null 2>&1; then
    if nm -D "$SO" 2>/dev/null | rg -q --fixed-strings "$needle"; then
      echo "FAIL: release .so nm -D contains: $needle"
      fail=1
    fi
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

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi
echo "OK: release .so mock/dev hygiene check passed"
