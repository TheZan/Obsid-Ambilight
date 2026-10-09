# Obsid Ambilight

Lightweight, fully local Windows controller for the **Yeelight Obsid RGBIC
strip** (protocol model `Chameleon2`): static workspace lighting and screen-synced
Ambilight, without Yeelight Station, SignalRGB, cloud accounts or a web view.

## Status

| Area | State |
|---|---|
| Stage 0 — LAN discovery (per-adapter multicast + subnet broadcast) | ✅ works on hardware |
| Stage 0 — UDP session (`udp_sess_new`, token, keep-alive, id matching, timeouts) | ✅ works on hardware |
| Stage 0 — Power / brightness / RGB / `get_prop` | ✅ protocol confirmed |
| Stage 0 — Direct mode + `update_leds` streaming at 20 FPS | ✅ protocol confirmed |
| Stage 0 — Leaving direct mode (`deactivate_fx_mode`) | ✅ confirmed on hardware; Apple Home outages seen twice, not reproducible |
| Stage 1 — tray, settings window (RU/EN), static control, reconnect, single instance | ✅ implemented, ⏳ hardware checklist |
| Ambilight, Windows lifecycle (autostart, shutdown, sleep) | Stages 2–3 |

Details: [docs/protocol.md](docs/protocol.md), hardware checklist:
[docs/testing.md](docs/testing.md), Stage 1 design:
[docs/superpowers/specs](docs/superpowers/specs).

## Build

Requires Rust stable (1.85+) on Windows x64.

```bash
cargo build --release
```

This produces `target/release/obsid-ambilight.exe` (the app) and
`target/release/obsid-poc.exe` (the Stage 0 protocol tool).

## Running the app

Close SignalRGB and Yeelight Station first: only one controller may drive the strip.

```bash
target/release/obsid-ambilight.exe
```

- The app has no window at start: it lives in the system tray. On the first
  run it finds the strip; if there is exactly one, it is selected and lit
  with the static colour, otherwise the settings window opens.
- Right-click the tray icon for colour presets, brightness, power, search and
  exit; double-click it (or choose «Настройки…») for the settings window.
- Starting the exe again opens the settings of the running instance.
- Settings: `%APPDATA%\ObsidAmbilight\config.toml` (written by the app;
  an unreadable file is kept as `config.bad-<time>.toml`).
  Logs: `%APPDATA%\ObsidAmbilight\logs\`.
- The interface is in Russian or English (follows Windows, can be changed in
  the settings).

## Using the PoC

```bash
obsid-poc discover
obsid-poc --ip 192.168.1.85 static "#F5D4A1" 40
obsid-poc --ip 192.168.1.85 direct --pattern rainbow --seconds 10
obsid-poc --ip 192.168.1.85 exit-test deactivate
obsid-poc --ip 192.168.1.85 watch --seconds 300
obsid-poc help
```

Without `--ip`, commands discover the strip automatically (use `--id` when there
are several). `-vv` logs every packet with the session token redacted.

## Layout

```text
src/
  main.rs        app entry: background process, or --settings window
  app/           controller state machine, device worker thread, UDP link
  config.rs      config.toml: lenient load, atomic save
  i18n.rs        Russian / English strings
  ipc/           named pipe between the settings window and the background process
  platform/      single-instance mutex, paths, Windows helpers
  ui/            tray (Win32), settings window (egui), status icons
  diagnostics.rs log files, panic hook
  protocol/      UDP transport (+ mock), packets, session, Chameleon2 device
  discovery/     adapter enumeration, M-SEARCH, response parsing
  reconnect.rs   resolve a saved device by id, backoff with jitter
  bin/obsid-poc.rs   Stage 0 console tool
tests/           automated tests (no hardware needed)
docs/            protocol notes, testing checklist, design and plans
```
