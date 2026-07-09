#!/usr/bin/env bash
# Provision one emulator with Saga APK and default-dialer role.
set -euo pipefail

SERIAL="${1:?usage: provision-emulator.sh SERIAL}"
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
E2E_DIR="$(dirname "$0")"
APK="${APK:-$REPO_ROOT/android/app/build/outputs/apk/debug/app-debug.apk}"
DEV_IDENTITY="${DEV_IDENTITY:-}"

echo "=== Building APK ==="
(cd "$REPO_ROOT/android" && ./gradlew assembleDebug -q)

echo "=== Installing on $SERIAL ==="
adb -s "$SERIAL" install -r "$APK"

if [[ -n "$DEV_IDENTITY" ]]; then
  echo "=== Seeding dev Iroh identity [$DEV_IDENTITY] on $SERIAL ==="
  adb -s "$SERIAL" push "$E2E_DIR/saga_dev_identity_${DEV_IDENTITY}.xml" /data/local/tmp/saga_dev_id.xml >/dev/null
  adb -s "$SERIAL" shell run-as org.saga mkdir -p shared_prefs
  adb -s "$SERIAL" shell run-as org.saga cp /data/local/tmp/saga_dev_id.xml shared_prefs/saga_dev_identity.xml
fi

echo "=== Granting ROLE_DIALER ==="
adb -s "$SERIAL" shell cmd role add-role-holder android.app.role.DIALER org.saga \
  || adb -s "$SERIAL" shell pm grant org.saga android.permission.CALL_PHONE

adb -s "$SERIAL" shell monkey -p org.saga -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || true
echo "provisioned $SERIAL"
