#!/usr/bin/env bash
# Verify bob/alice have Saga pubkey rows; thor must not.
set -euo pipefail

SERIAL="${1:?usage: verify-contact-keys.sh SERIAL}"

echo "=== Contact keys on $SERIAL ==="
FAIL=0

DATA="$(adb -s "$SERIAL" shell "content query --uri content://com.android.contacts/data --projection data1:mimetype" 2>/dev/null || true)"

decode_key() {
  printf '%s' "$1" | base64 -d 2>/dev/null || echo "(decode failed)"
}

saga_key_after_name() {
  local name="$1"
  local found=0 key_b64=""
  while IFS= read -r line; do
    if [[ "$line" == *"data1=$name,"* && "$line" == *"item/name"* ]]; then
      found=1
      continue
    fi
    if [[ "$found" -eq 1 && "$line" == *"saga.galdralag_pubkey"* ]]; then
      key_b64="$(echo "$line" | sed -n 's/.*data1=\([^,]*\).*/\1/p')"
      echo "$key_b64"
      return 0
    fi
    if [[ "$found" -eq 1 && "$line" == *"item/name"* ]]; then
      return 1
    fi
  done <<< "$DATA"
  return 1
}

has_name() {
  echo "$DATA" | rg -q "data1=$1, mimetype=vnd.android.cursor.item/name"
}

for name in bob alice; do
  if ! has_name "$name"; then
    echo "FAIL: contact [$name] not found"
    FAIL=1
    continue
  fi
  key_b64="$(saga_key_after_name "$name" || true)"
  if [[ -z "$key_b64" ]]; then
    echo "FAIL: [$name] missing Saga pubkey row"
    FAIL=1
    continue
  fi
  decoded="$(decode_key "$key_b64")"
  if [[ "$decoded" == *bobpeer12* || "$decoded" == *alicepeer1* ]]; then
    echo "FAIL: [$name] still has legacy key [$decoded]"
    FAIL=1
  else
    echo "PASS: [$name] pubkey endpoint=[$decoded]"
  fi
done

if ! has_name thor; then
  echo "FAIL: contact [thor] not found"
  FAIL=1
else
  thor_key="$(saga_key_after_name thor || true)"
  if [[ -n "$thor_key" ]]; then
    decoded="$(decode_key "$thor_key")"
    echo "FAIL: [thor] has Saga pubkey [$decoded] but should not"
    FAIL=1
  else
    echo "PASS: [thor] has no Saga pubkey (expected)"
  fi
fi

exit "$FAIL"
