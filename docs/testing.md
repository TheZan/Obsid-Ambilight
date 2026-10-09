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

## Stage 1 on hardware

Run `target\release\obsid-ambilight.exe`. After each row, check that Apple Home
still controls the strip.

| # | Scenario | Expected | Result |
|---|---|---|---|
| S1 | First start without `config.toml` | Strip found and lit with the static colour; disc icon | ✅ auto-selected `dcda0cd778bc`, strip reported `#F5D4A1` @ 40 %; icon and tooltip confirmed by user 2026-10-10, Apple Home OK. Note: changes made in Apple Home are not reflected in the tray (by design, no polling) |
| S2 | Restart the app | Connects without searching UI, applies static colour | ✅ 2026-10-10 (user), Apple Home OK |
| S3 | Tray: preset, brightness, power off/on | Strip follows; icon/tooltip update | ✅ 2026-10-10 (user), Apple Home OK |
| S4 | Settings: colour picker drag, brightness slider | Smooth, ≤ 5 updates/s, final value applied | ✅ 2026-10-10 (user), Apple Home OK; HEX focus/unchanged-commit fixes confirmed |
| S5 | Unplug the strip, change colour, wait, plug back | Red icon «Устройство недоступно», retries grow; after power returns the last colour is applied | ✅ 2026-10-10 (user): offline icon, growing retries, reconnected ~10 s after power-on and applied the offline change; Apple Home OK |
| S6 | Launch the exe a second time | Settings open in the running app, no second icon | ✅ forwarded `OpenSettings`, settings window opened/focused, no second icon (user 2026-10-10) |
| S7 | Restart Explorer | Icon returns | ✅ 2026-10-10 (user), Apple Home OK |
| S8 | Exit with «Оставить цвет» / «Выключить ленту» | Strip keeps colour / turns off; process ends | ✅ 2026-10-10 (user), Apple Home OK (both keep and off) |
| S9 | Switch language RU ↔ EN | Tray menu and window switch at once | ✅ 2026-10-10 (user), Apple Home OK; choice persists across restart |
| S10 | Corrupt `config.toml`, start | App starts with defaults, `config.bad-*.toml` kept, warning in Diagnostics | ✅ 2026-10-10 (user), Apple Home OK |
| S11 | Memory/CPU in Static (background process) | ≤ 35 MB, ≤ 1 % CPU on average | ✅ 13.1 MB working set, 2.4 MB private, 0.000 s CPU over 60 s idle (16 cores) |
| S12 | Memory of the settings process while open | recorded | ✅ ~125 MB working set, ~139 MB private (OpenGL); freed on close |
