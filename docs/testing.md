# Testing

## Automated (no hardware)

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

- `tests/protocol_tests.rs` — packet encoding, LED packing, response parsing,
  token redaction, session handshake/keep-alive/id matching against a scripted
  `MockTransport`, the device layer, and the real `UdpTransport` against a fake
  device on 127.0.0.1.
- `tests/discovery_tests.rs` — discovery response parsing, resolving a saved
  device by id (`reconnect::resolve`), backoff with jitter.

## Stage 0 on hardware

Close SignalRGB and Yeelight Station first: only one controller may drive the
strip. Replace `<ip>` with the address shown by `discover`.

| # | Command | Expected | Result 2026-10-09 (fw 36) |
|---|---|---|---|
| 1 | `obsid-poc discover` | The strip is listed with id, IP, `rgbic udp: true` | ✅ id `dcda0cd778bc`, found via `Ethernet` despite VPN default route |
| 2 | `obsid-poc --ip <ip> -vv session` | Handshake + keep-alive OK, token shown as `***` | ✅ |
| 3 | `obsid-poc --ip <ip> on` / `off` | Strip turns on / off | ⏳ needs visual check |
| 4 | `obsid-poc --ip <ip> static "#F5D4A1" 40` | Warm white at 40 % | ⏳ visual |
| 5 | `obsid-poc --ip <ip> direct --pattern rainbow --seconds 10` | Moving rainbow along the strip | ✅ protocol (20.0 FPS, 0 late), ⏳ visual |
| 6 | `obsid-poc --ip <ip> --leds 60 direct --pattern ends` | 1st LED red, last green, every 10th white → tells the real LED count | ⏳ visual |
| 7 | `obsid-poc --ip <ip> direct --pattern chase` | One white dot moving → direction of index 0 | ⏳ visual |
| 8 | `obsid-poc --ip <ip> exit-test deactivate` | Blue → rainbow → static colour that **stays** for 25 s of silence | ✅ protocol/get_prop, ✅ visual (user, 19:13 UTC) |
| 9 | After step 8: change colour, brightness, power in Apple Home | Strip follows Apple Home | ⚠️ once "No response" for ~2 min (recovered on its own); repeat run OK — intermittent, see docs/protocol.md |
| 10 | `obsid-poc --ip <ip> exit-test set-rgb` (and `power-cycle`, `expire`) | Compare with step 8 | ⏳ |
| 11 | `obsid-poc --ip <ip> direct --seconds 5`, then wait 30 s without exit | Documents what an abandoned direct session looks like (crash case) | ⏳ |

Record the visual outcome of each ⏳ row here and in `docs/protocol.md`.
