#!/usr/bin/env bash
# Case 7: alice↔bob speech round-trip via PCM inject/dump.
# Thresholds (LibriSpeech fixture): corr >= 0.85, SI-SNR >= 10 dB, STOI >= 0.85.
set -uo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
E2E_DIR="$(cd "$(dirname "$0")" && pwd)"
OUT_DIR="${OUT_DIR:-/tmp/saga-case7-speech}"
mkdir -p "$OUT_DIR"
record() { echo "$1"; }

# shellcheck source=e2e-common.sh
source "$E2E_DIR/e2e-common.sh"

export SERIAL_A="${SERIAL_A:-emulator-5554}"
export SERIAL_B="${SERIAL_B:-emulator-5556}"
export OUT_DIR

set -e
bash "$E2E_DIR/run-pcm-roq-metrics.sh" | tee "$OUT_DIR/pcm-roq.log"
set +e

pass=1
check_dir() {
  local label="$1" line
  line=$(rg "^${label}: corr=" "$OUT_DIR/pcm-roq.log" | tail -1 || true)
  if [[ -z "$line" ]]; then
    record "CASE7_${label}:FAIL missing metrics"
    pass=0
    return
  fi
  if python3 - "$label" "$line" <<'PY'
import re, sys
label, line = sys.argv[1], sys.argv[2]
corr = float(re.search(r"corr=([0-9.]+)", line).group(1))
snr = float(re.search(r"si_snr_db=(-?[0-9.]+)", line).group(1))
m = re.search(r"stoi=([0-9.]+)", line)
stoi = float(m.group(1)) if m else None
ok = corr >= 0.85 and snr >= 10.0 and (stoi is None or stoi >= 0.85)
stoi_part = f" stoi={stoi:.4f}" if stoi is not None else " stoi=n/a"
print(f"CASE7_{label}:{'PASS' if ok else 'FAIL'} corr={corr:.3f} si_snr_db={snr:.1f}{stoi_part}")
sys.exit(0 if ok else 1)
PY
  then
    :
  else
    pass=0
  fi
}

check_dir "alice_to_bob"
check_dir "bob_to_alice"

if [[ "$pass" -eq 1 ]]; then
  record "CASE7:PASS speech round-trip both directions (corr>=0.85 SI-SNR>=10 STOI>=0.85)"
  exit 0
fi
record "CASE7:FAIL speech metrics below threshold (see $OUT_DIR)"
exit 1
