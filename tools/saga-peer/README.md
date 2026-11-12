# saga-peer

Linux CLI reference peer for Saga voice (dev/test; spec §8). Not a product.

Uses the same mock SessionToken handshake as Android debug builds, and the
shared `saga-media` Opus + SRTP (or plain RTP) pipeline over the pinned
`iroh-roq` fork. No duplicated media or crypto.

## Build

```bash
cd tools/saga-peer
cargo build --release
# Optional host mic/speakers (rubato MIT/Apache-2.0 for rate mismatch):
cargo build --release --features live-audio
```

CI builds without `live-audio` and runs the host WAV round-trip tests only.

## Identities

| Contact | Phone / label | Fingerprint |
|---------|---------------|-------------|
| bob | `15550100010` | `80342e72…` |
| alice | `15550100011` | `ee52f46c…` |
| laptop | `15550100012` | `f14a880d…` |
| thor | `15550100013` | (none — cellular only) |

Thor previously used `15550100012`; that label is now the laptop peer.

## Two-way listening test (emulator ↔ laptop)

Use headphones on the laptop (no AEC on this peer).

1. Start the local relay on the host:

   ```bash
   bash testing/iroh-relay/start-local-relay.sh
   ```

2. Start an emulator with host audio (mic + speaker):

   ```bash
   emulator -avd <AVD> -qemu -enable-kvm \
     -microphone host \
     -prop "ro.audio.monitorof=@" # or your host-audio flags
   ```

   Practical flags that work with current SDK tools:

   ```bash
   emulator -avd <AVD> -no-snapshot-load \
     -writable-system
   ```

   Then ensure host audio is enabled in the Extended Controls → Microphone,
   and that the AVD has mic permission for Saga.

3. Install the debug APK and seed contacts (includes **laptop**):

   ```bash
   ./gradlew -p android assembleDebug
   adb install -r android/app/build/outputs/apk/debug/app-debug.apk
   bash testing/e2e/seed-test-contacts.sh emulator-5554
   ```

4. On the host, listen as the laptop identity (default):

   ```bash
   cargo run --release --manifest-path tools/saga-peer/Cargo.toml -- \
     listen --identity 15550100012 \
     --relay http://127.0.0.1:3340
   ```

   Emulator reaches the same relay via `http://10.0.2.2:3340` (app default).

5. In Saga on the emulator, call contact **laptop**. Peer auto-answers.
   Reverse: peer `call --identity 15550100012 --peer 15550100011` while the
   alice emulator is listening (dev identity set to alice).

6. What to report: echo, choppiness/dropouts, one-way delay (rough seconds),
   robotic/metallic sound, volume imbalance, mute (`m`) / hang-up (`q`).

### Scripted WAV path (no sound card)

```bash
cargo run --manifest-path tools/saga-peer/Cargo.toml -- \
  listen --identity 15550100012 \
  --wav-in saga-media/fixtures/speech_clip_48k.wav \
  --wav-out /tmp/peer-out.wav --loop-wav
```

## Troubleshooting

| Symptom | Check |
|---------|--------|
| Peer never sees the call | Local relay up? Emulator relay URL `http://10.0.2.2:3340`? |
| No mic / silence on emulator | Host audio / mic permission; Extended Controls → Microphone |
| Wrong device on laptop | `saga-peer devices` (needs `--features live-audio`); `--input-device` / `--output-device` |
| Handshake "Not encrypted" | Peer and phone both mock-token debug; contact fingerprint seeded |

## Note on Android media

Android Step 3 still loopbacks PCM locally until RoQ is wired end-to-end.
Handshake and security UI work against saga-peer today; full Opus/SRTP audio
with the emulator needs the Android RoQ media path (follow-up). Host A↔B
tests in this crate exercise the real media pipeline.
