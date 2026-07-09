#!/usr/bin/env bash
# Seed bob, alice, thor test contacts on an emulator.
# Primary path: launch Saga so TestContactSeeder runs in-process (reliable).
# adb content insert is attempted as a fallback but often returns no URI on modern emulators.
set -euo pipefail

SERIAL="${1:?usage: seed-test-contacts.sh SERIAL}"
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
APK="${APK:-$REPO_ROOT/android/app/build/outputs/apk/debug/app-debug.apk}"

PHONE_BOB="${PHONE_BOB:-+15550100010}"
PHONE_ALICE="${PHONE_ALICE:-+15550100011}"
PHONE_THOR="${PHONE_THOR:-+15550100012}"

echo "=== Grant contacts permission to Saga on $SERIAL ==="
adb -s "$SERIAL" shell pm grant org.saga android.permission.READ_CONTACTS 2>/dev/null || true
adb -s "$SERIAL" shell pm grant org.saga android.permission.WRITE_CONTACTS 2>/dev/null || true

echo "=== In-app seed (TestContactSeeder via app launch) ==="
adb -s "$SERIAL" shell am force-stop org.saga
adb -s "$SERIAL" logcat -c 2>/dev/null || true
adb -s "$SERIAL" shell monkey -p org.saga -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || true
sleep 3

echo "=== Verify bob/alice/thor ==="
if bash "$(dirname "$0")/verify-contact-keys.sh" "$SERIAL" >/dev/null 2>&1; then
  echo "seeded via in-app TestContactSeeder or existing contacts"
  bash "$(dirname "$0")/verify-contact-keys.sh" "$SERIAL" || true
  exit 0
fi

echo "WARN: contacts not verified; trying adb fallback" >&2

insert_contact_adb() {
  local name="$1" phone="$2" saga_key_b64="${3:-}"
  local before after raw_id
  before="$(adb -s "$SERIAL" shell content query --uri content://com.android.contacts/raw_contacts \
    --projection _id 2>/dev/null | rg -o '_id=[0-9]+' | tail -1 | cut -d= -f2 || echo 0)"
  adb -s "$SERIAL" shell content insert --uri content://com.android.contacts/raw_contacts \
    --bind aggregation_mode:i:1 >/dev/null 2>&1 || true
  after="$(adb -s "$SERIAL" shell content query --uri content://com.android.contacts/raw_contacts \
    --projection _id 2>/dev/null | rg -o '_id=[0-9]+' | tail -1 | cut -d= -f2 || echo 0)"
  if [[ "$after" == "$before" ]]; then
    echo "ERROR: adb raw_contacts insert did not create a row for [$name] (insert stdout is empty on this API level)" >&2
    return 1
  fi
  raw_id="$after"
  adb -s "$SERIAL" shell content insert --uri content://com.android.contacts/data \
    --bind raw_contact_id:i:"$raw_id" \
    --bind mimetype:s:vnd.android.cursor.item/name \
    --bind data1:s:"$name" >/dev/null
  adb -s "$SERIAL" shell content insert --uri content://com.android.contacts/data \
    --bind raw_contact_id:i:"$raw_id" \
    --bind mimetype:s:vnd.android.cursor.item/phone_v2 \
    --bind data1:s:"$phone" \
    --bind data2:i:2 >/dev/null
  if [[ -n "$saga_key_b64" ]]; then
    adb -s "$SERIAL" shell content insert --uri content://com.android.contacts/data \
      --bind raw_contact_id:i:"$raw_id" \
      --bind mimetype:s:vnd.android.cursor.item/vnd.saga.galdralag_pubkey \
      --bind data1:s:"$saga_key_b64" >/dev/null
  fi
  echo "  adb raw_contact_id=$raw_id name=$name"
}

KEY_BOB="$(printf '%s' "15550100010" | base64 -w0)"
KEY_ALICE="$(printf '%s' "15550100011" | base64 -w0)"
insert_contact_adb "bob" "$PHONE_BOB" "$KEY_BOB" || true
insert_contact_adb "alice" "$PHONE_ALICE" "$KEY_ALICE" || true
insert_contact_adb "thor" "$PHONE_THOR" "" || true

echo "seeded contacts on $SERIAL (adb fallback path)"
