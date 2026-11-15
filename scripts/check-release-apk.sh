#!/usr/bin/env bash
# Fail if a release APK's AndroidManifest advertises debug-only TEST_* hooks,
# or if packaged native libs fail check-release-so.sh.
set -euo pipefail

APK="${1:?usage: check-release-apk.sh path/to/app-release.apk}"
test -f "$APK"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail=0

if ! command -v aapt >/dev/null 2>&1 && ! command -v aapt2 >/dev/null 2>&1; then
  # Fall back to SDK build-tools on typical developer machines.
  if [[ -n "${ANDROID_HOME:-}" ]]; then
    BT="$(ls -d "$ANDROID_HOME"/build-tools/*/ 2>/dev/null | sort -V | tail -1 || true)"
    if [[ -n "$BT" && -x "${BT}aapt" ]]; then
      export PATH="${BT}:$PATH"
    fi
  fi
fi

if command -v aapt >/dev/null 2>&1; then
  aapt dump xmltree "$APK" AndroidManifest.xml >"$TMP/manifest-tree.txt"
  aapt dump badging "$APK" >"$TMP/badging.txt"
else
  echo "FAIL: aapt not found — cannot dump release manifest"
  exit 1
fi

check_manifest_absent() {
  local needle="$1"
  if grep -F -- "$needle" "$TMP/manifest-tree.txt" >/dev/null; then
    echo "FAIL: release manifest contains forbidden string: $needle"
    fail=1
  else
    echo "OK: manifest absent [$needle]"
  fi
}

for needle in \
  "org.saga.TEST_IROH_CALL" \
  "org.saga.TEST_FORCE_IROH_SIM_PATH" \
  "org.saga.TEST_MIDCALL_REHANDSHAKE" \
  "org.saga.TEST_IN_CALL_UI" \
  "org.saga.TEST_PCM_INJECT" \
  "org.saga.TEST_PCM_DUMP" \
  "org.saga.TEST_ECHO_MODE" \
  "org.saga.TEST_RX_FAULT" \
  "org.saga.TEST_RELAY_QUERY" \
  "org.saga.TEST_CONTACT_CALL" \
  "org.saga.TEST_ANSWER_INCOMING" \
  "org.saga.TEST_ADD_PARTICIPANT" \
  "org.saga.TEST_PEER_DISGUISE"
do
  check_manifest_absent "$needle"
done

if grep -F 'package: name='\''org.saga'\''' "$TMP/badging.txt" >/dev/null; then
  echo "OK: package name is org.saga (ID change is a one-line follow-up in build.gradle.kts)"
else
  echo "NOTE: package name in badging:"
  grep -E '^package:' "$TMP/badging.txt" || true
fi

if unzip -l "$APK" | grep -F 'lib/arm64-v8a/libsaga_iroh_core.so' >/dev/null; then
  echo "OK: arm64-v8a libsaga_iroh_core.so packaged"
else
  echo "FAIL: release APK missing lib/arm64-v8a/libsaga_iroh_core.so"
  fail=1
fi

# Only libsaga_iroh_core.so may appear under lib/<abi>/ (no leftover cdylibs).
unexpected_so=0
while IFS= read -r entry; do
  [[ -z "$entry" ]] && continue
  base="$(basename "$entry")"
  if [[ "$base" != "libsaga_iroh_core.so" ]]; then
    echo "FAIL: unexpected native library in release APK: $entry"
    fail=1
    unexpected_so=1
  fi
done < <(unzip -l "$APK" | awk '/[[:space:]]lib\/[^/]+\/[^/]+\.so$/ { print $NF }')
if [[ "$unexpected_so" -eq 0 ]]; then
  echo "OK: no unexpected .so under lib/"
fi

# Iroh group-call UI strings are debug source-set only (not in release resources).
# Carrier conference (Phase 4) may still say "Conference:" — that is fine.
check_apk_string_absent() {
  local needle="$1"
  if unzip -p "$APK" resources.arsc 2>/dev/null | strings | grep -F -- "$needle" >/dev/null; then
    echo "FAIL: release APK resources contain forbidden string: $needle"
    fail=1
  else
    echo "OK: resources absent [$needle]"
  fi
}
check_apk_string_absent "Add participant"
check_apk_string_absent "Add to call"
check_apk_string_absent "Only Saga contacts can join a group call"

SO_OUT="$TMP/libsaga_iroh_core.so"
if unzip -p "$APK" 'lib/arm64-v8a/libsaga_iroh_core.so' >"$SO_OUT" 2>/dev/null; then
  SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
  bash "$SCRIPT_DIR/check-release-so.sh" "$SO_OUT" || fail=1
else
  echo "FAIL: could not extract arm64 libsaga_iroh_core.so"
  fail=1
fi

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi
echo "OK: release APK hygiene check passed"
