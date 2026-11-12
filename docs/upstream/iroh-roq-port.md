# Upstream draft: port `iroh-roq` to iroh 1.x

**Status:** prepared for `n0-computer/iroh-roq` — **not opened upstream**. Saga uses
the fork at https://github.com/Supermagnum/iroh-roq (rev pinned from Saga
`Cargo.toml`).

## Summary

Port RTP-over-QUIC (`iroh-roq`) from iroh **0.35** to iroh **1.0** (Saga pin),
keeping RoQ ALPN `/iroh/roq/1`, flow-id handling, datagram media path, and the
optional uni-stream path.

## Diff size (vs upstream `main` @ `fe00827`)

Source only (excluding `Cargo.lock`): **7 files, +118 / −48**.

## Semantic / API changes forced by iroh 1.x

| Area | 0.35 | 1.x port |
|------|------|----------|
| Endpoint builder | `Endpoint::builder().bind_addr_v4(..)` | `Endpoint::builder(presets::Minimal).relay_mode(Disabled).bind()` (tests) |
| Peer address | `endpoint.node_addr().await?` | `endpoint.addr()` (sync `EndpointAddr`) |
| ALPN on connection | `connection.alpn().unwrap()` → `Option` | `connection.alpn()` → `&[u8]` after handshake |
| QUIC varint codec | `iroh-quinn-proto::{VarInt, coding::Codec}` | `iroh::endpoint::VarInt` + `noq_proto::coding::{Encodable,Decodable}` (iroh 1.x quinn → noq) |
| Datagram send | `Connection::send_datagram` (unchanged) | unchanged; added `SendFlow::send_rtp_wait` using `send_datagram_wait` for bursty media / tests |

RoQ **flow-id** framing is unchanged: datagrams are `VarInt(flow_id) \|\| RTP`;
streams still send flow-id once then length-prefixed RTP. Media for Saga uses
the **datagram** path.

## Tests

On the fork (`cargo test`):

- `test_datagram_flow` — PASS (10 RTP datagrams, echo)
- `test_session_flow` — PASS (uni-stream send / datagram receive echo)
- `test_datagram_flow_1000_packets` — PASS (1000 RTP datagrams, in-process)
- `tests/opus.rs` — PASS (`test_opus_decode_encode`, `test_stream_opus_packets`)

## Suggested upstream PR body (when opening)

Title: `Port to iroh 1.x (noq VarInt / Endpoint 1.0 APIs)`

Body sketch:

> This ports `iroh-roq` to iroh 1.0 while keeping ALPN `/iroh/roq/1` and RoQ
> flow-id semantics. `iroh-quinn-proto` is replaced with `noq-proto` coding
> traits to match iroh’s quinn→noq migration. Endpoint construction in tests
> uses `presets::Minimal` + `RelayMode::Disabled`. Adds `SendFlow::send_rtp_wait`
> and a 1000-packet in-process datagram exchange test. Licence headers /
> dual MIT OR Apache-2.0 unchanged.

## Saga pin

```toml
iroh-roq = { git = "https://github.com/Supermagnum/iroh-roq", rev = "408d8ccccd3c6ee6a44d7da4a960d34e0c36e971" }
```
