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

## Stage 2 on hardware

Run `target\release\obsid-ambilight.exe`. After each row, check that Apple Home
still controls the strip. GPU tests: `cargo test --test capture_tests -- --ignored`.

| # | Scenario | Expected | Result |
|---|---|---|---|
| A1 | `obsid-poc --ip <ip> --leds 60 direct --pattern ends`, then `--pattern chase` | Real LED count and where index 0 is | ✅ 2026-10-10 (user): 60 LEDs; index 0 (red) at the power input, 59 (green) at the free end of the inner loop; every 10th white in between; 20.0 FPS, 0 late |
| A2 | Settings → Ambilight → «Поиск диода» along the whole strip | One white LED, red behind, green ahead | ✅ 2026-10-10 (user): dot and neighbour colours follow the slider. Corners (back view): 0 bottom right, 9 top right, 25 top left, 32 bottom left, 42 bottom before the stand, 47 inner top right, 59 inner top left (inner loop re-laid as a rectangle) |
| A3 | Build the two-loop spiral layout (outer and inner loops as segments) | Both loops follow the screen edges | ✅ 2026-10-10 (user): 6 segments (0–9, 10–25, 26–32, 33–42 outer; 43–47, 48–59 inner) highlight the right runs. Found: the schematic must show where the strip lies → segments now sample the edge they face; ends join on drag, connectors and labels inside the frame (commits 11ba18d, 8fb43d5) |
| A4 | Tray → «Ambilight» on the desktop, then «Статика» | Stream starts within ~1 s; static colour returns | ✅ 2026-10-10 (user): starts in ≤ 1 s, colours match the edges, static returns |
| A5 | Full-screen game (exclusive and borderless) | Colours follow the game | ✅ 2026-10-10 (user): both modes follow the game, almost no pause on switching |
| A6 | 21:9 film in a 16:9 player (or `docs/letterbox-test.html`, F11) | Bars ignored after ~1 s, colours from the picture | ✅ 2026-10-10 (user): film — top/bottom show the picture, no jumping in dark scenes, Diagnostics ~1 % (thin or non-black bars in that video); test page 2.39:1 — 12 % / 0 %, colours match |
| A7 | Toggle Windows HDR while Ambilight runs | Short pause, then colours not washed out; Diagnostics shows HDR and SDR white | ✅ 2026-10-10 (user): ~1 s pause each way; colours saturated in HDR; Diagnostics «да, белый SDR 240 нит», after switching off «нет» |
| A8 | UAC prompt / Win+L | Yellow dot, last colours held, capture resumes afterwards | ✅ 2026-10-10 (user): yellow dot shown, last colours held, Ambilight resumes after UAC and after unlocking. The lock screen picture itself is captured (it is drawn on the user desktop), so the strip shows its colours — behaviour on lock is a Stage 3 decision |
| A9 | 20, 25, 30 FPS for 10 min each | Smooth; Diagnostics send FPS ≤ setting; Apple Home OK | ✅ 2026-10-10 (user): smooth at 20, 25 and 30 FPS, send rate within the setting, Apple Home OK |
| A10 | Ambilight brightness 30 % vs 100 % | Visibly dimmer; note whether `set_bright` affects direct mode | ✅ 2026-10-10 (user): 30 % visibly dimmer, colours not distorted; tray «Яркость ▸» works in Ambilight. Hardware brightness stays pinned at 100, so the effect of `set_bright` on direct mode was not needed |
| A11 | Unplug the strip during Ambilight, plug back | Offline, retries; Ambilight resumes by itself | ✅ 2026-10-10 (user): goes offline quickly, Ambilight resumes by itself after power returns, Apple Home OK |
| A12 | Exit with «Оставить цвет» / «Выключить ленту» from Ambilight | Static colour / off | ✅ 2026-10-10 (user): keep → static colour, off → strip off; Apple Home OK after both |
| A13 | Kill the process in Task Manager during Ambilight; wait 30 s | Record what the strip shows (open question from Stage 0) | ✅ 2026-10-10 (user): the strip freezes on the last Ambilight frame and stays there after the session expires; Apple Home still controls it |
| A14 | Restart the app in Ambilight mode | Ambilight resumes automatically | ✅ 2026-10-10 (user): after the A13 crash, a plain restart resumed live Ambilight within a second or two, no clicks |
| A15 | CPU and RAM of the background process in Ambilight, 25 FPS, 1440p, 5 min | ≤ 3 % CPU average, ≤ 60 MB RAM | ✅ 2026-10-10, moving picture: CPU 0.16 % of 16 cores (2.5 % of one core); working set 37.9 → 38.9 MB, flat after 2 min; private 77.8 MB (D3D11/NVIDIA driver commit); 18 threads |
| A16 | CPU and RAM in Static after leaving Ambilight | Stage 1 targets (≤ 35 MB, ≤ 1 % CPU) | ✅ 2026-10-10, same process after A15: CPU 0.000 s over 120 s; working set 26.3 MB (GPU driver DLLs stay mapped; Stage 1 cold start was 13.1 MB); private 77.8 → 6.7 MB, threads 18 → 7 — capture and D3D resources released |

## Stage 3 on hardware

Run `target\release\obsid-ambilight.exe`. After each row, check that Apple Home
still controls the strip.

| # | Scenario | Expected | Result |
|---|---|---|---|
| L1 | First start of the Stage 3 build | Task Manager → Startup apps lists Obsid Ambilight (enabled) | ✅ listed as «obsid-ambilight», Включено |
| L2 | Win+L, wait 10 s, unlock — in Static and in Ambilight | Off while locked; mode returns after unlock | ✅ Static and Ambilight: off at once on lock, mode back after unlock |
| L3 | Display turns off by the power plan timer (set it to 1 min for the test), then move the mouse | Off with the display; mode returns with it | ✅ off with the display, Ambilight back on mouse move |
| L4 | Sleep (Start → Power → Sleep), wake up | Off before sleeping; mode returns after waking (maybe after a network retry) | ✅ off before sleeping, Ambilight back after waking |
| L5 | Restart Windows in Ambilight | Off during shutdown (no frozen frame); after logon the app starts by itself and Ambilight resumes | ✅ off during restart; after logon the app started itself and Ambilight resumed |
| L6 | Sign out and back in | Off at sign-out; app starts and resumes at sign-in | ✅ off at sign-out; app started itself and Ambilight resumed at sign-in |
| L7 | Uncheck «Запускать вместе с Windows», then check it again | Entry disappears from / returns to Startup apps | ✅ entry disappears and returns; hint shown |
| L8 | Disable the app in Task Manager → Startup apps, restart Windows | App does not start; the checkbox still shows on (hint explains) | ✅ app did not start; checkbox still on (expected, the hint explains) |
| L9 | Lock while the settings window highlights a LED | Highlight ends, strip off; after unlock the mode returns | ✅ off on lock incl. highlight; mode back without highlight after unlock |
