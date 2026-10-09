# Obsid Ambilight

Lightweight, fully local Windows controller for the **Yeelight Obsid RGBIC
strip** (protocol model `Chameleon2`): static workspace lighting and screen-synced
Ambilight, without Yeelight Station, SignalRGB, cloud accounts or a web view.

## Status: Stage 0 — protocol proof of concept

Only the device layer exists so far. Per the specification, no UI, tray, screen
capture or autostart is built until the protocol is confirmed on the real strip.

| Area | State |
|---|---|
| LAN discovery (per-adapter multicast + subnet broadcast) | ✅ works on hardware |
| UDP session (`udp_sess_new`, token, keep-alive, id matching, timeouts) | ✅ works on hardware |
| Power / brightness / RGB / `get_prop` | ✅ protocol confirmed |
| Direct mode + `update_leds` streaming at 20 FPS | ✅ protocol confirmed |
| Leaving direct mode (`deactivate_fx_mode`) | ✅ protocol confirmed, ⏳ visual + Apple Home check |
| Tray, config, Ambilight, Windows lifecycle | Stages 1–3 |

Details: [docs/protocol.md](docs/protocol.md), hardware checklist:
[docs/testing.md](docs/testing.md).

## Build

Requires Rust stable (1.85+) on Windows x64.

```bash
cargo build --release
```

The PoC binary is `target/release/obsid-poc.exe`.

## Using the PoC

Close SignalRGB and Yeelight Station before running it.

```bash
obsid-poc discover
obsid-poc --ip 192.168.1.85 static "#F5D4A1" 40
obsid-poc --ip 192.168.1.85 direct --pattern rainbow --seconds 10
obsid-poc --ip 192.168.1.85 exit-test deactivate
obsid-poc help
```

Without `--ip`, commands discover the strip automatically (use `--id` when there
are several). `-vv` logs every packet with the session token redacted.

## Layout

```text
src/
  protocol/    UDP transport (+ mock), packets, session, Chameleon2 device
  discovery/   adapter enumeration, M-SEARCH, response parsing
  reconnect.rs resolve a saved device by id, backoff with jitter
  bin/obsid-poc.rs   Stage 0 console tool
tests/         protocol and discovery tests (no hardware needed)
docs/          protocol research notes, testing checklist
```
