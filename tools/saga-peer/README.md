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

CI builds without `live-audio` and runs the host WAV round-trip tests only
(`[profile.test] opt-level = 2`).

## Identities

| Contact | Phone / label | Fingerprint |
|---------|---------------|-------------|
| bob | `15550100010` | `80342e72…` |
| alice | `15550100011` | `ee52f46c…` |
| thor | `15550100012` | (none — cellular only; never Iroh) |
| laptop | `15550100020` | `43676d15…` |

Thor keeps the original cellular fixture number. Laptop uses a separate label
so cellular contacts never resolve to an Iroh identity.

## Two-way listening test (emulator ↔ laptop)

Use headphones on the laptop (no AEC on this peer).

1. Start the local relay on the host (binds `0.0.0.0:3340`, advertises LAN IP):

   ```bash
   bash testing/iroh-relay/start-local-relay.sh
   # prints / writes http://<LAN-IP>:3340 to /tmp/saga-iroh-relay-url.txt
   ```

2. Resolve the shared relay URL (same value for peer + emulators):

   ```bash
   # helper:
   cargo run --manifest-path tools/saga-peer/Cargo.toml -- relay-host
   # or one-liner:
   echo "http://$(ip -4 route get 1.1.1.1 | awk '{for(i=1;i<=NF;i++) if($i=="src"){print $(i+1); exit}}'):3340"
   # or:
   bash testing/iroh-relay/lan-relay-url.sh
   ```

   Do **not** advertise `127.0.0.1` or `10.0.2.2` — the peer publishes its
   home relay into EndpointAddr, and the emulator must reach that URL.

3. Start an emulator with host audio enabled (Extended Controls → Microphone).
   Grant Saga mic permission after install.

4. Install the debug APK and seed contacts (includes **laptop**). E2E pushes
   `saga_iroh_relay.xml` with the LAN URL automatically:

   ```bash
   ./gradlew -p android assembleDebug
   adb install -r android/app/build/outputs/apk/debug/app-debug.apk
   bash testing/e2e/seed-test-contacts.sh emulator-5554
   ```

5. On the host, listen as the laptop identity (default relay = LAN URL file):

   ```bash
   RELAY=$(bash testing/iroh-relay/lan-relay-url.sh)
   cargo run --release --manifest-path tools/saga-peer/Cargo.toml -- \
     listen --identity 15550100020 --relay "$RELAY"
   ```

6. In Saga on the emulator, call contact **laptop**. Peer auto-answers.
   Reverse: peer `call --identity 15550100020 --peer 15550100011` while the
   alice emulator is listening (dev identity set to alice).

7. What to report: echo, choppiness/dropouts, one-way delay (rough seconds),
   robotic/metallic sound, volume imbalance, mute (`m`) / hang-up (`q`).

### Scripted WAV path (no sound card)

```bash
RELAY=$(bash testing/iroh-relay/lan-relay-url.sh)
cargo run --manifest-path tools/saga-peer/Cargo.toml -- \
  listen --identity 15550100020 --relay "$RELAY" \
  --wav-in saga-media/fixtures/speech_clip_48k.wav \
  --wav-out /tmp/peer-out.wav --loop-wav
```

Automated harness: `testing/e2e/run-peer-wav-roundtrip.sh` (emulator inject/dump +
peer `--wav-in/--wav-out`). Uses the LAN relay URL from `start-local-relay.sh`.

## Troubleshooting

| Symptom | Check |
|---------|--------|
| Peer never sees the call | Local relay up? Same LAN `http://<LAN-IP>:3340` on peer **and** emulator prefs? |
| Emulator connection refused | Peer still advertising `127.0.0.1`? Re-run `start-local-relay.sh` + `--relay $(lan-relay-url.sh)` |
| No mic / silence on emulator | Host audio / mic permission; Extended Controls → Microphone |
| Wrong device on laptop | `saga-peer devices` (needs `--features live-audio`); `--input-device` / `--output-device` |
| Handshake "Not encrypted" | Peer and phone both mock-token debug; contact fingerprint seeded |

## Note on Android media

With Step 3b, Android capture/playback uses shared `saga-media` over RoQ
(Opus + SRTP or plain RTP). Host A↔B tests and emulator↔peer WAV paths
exercise real network media — not local loopback.
