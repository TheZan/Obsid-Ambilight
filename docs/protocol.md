# Yeelight Chameleon2 UDP protocol — research notes

Status of every protocol detail used by the app. **Confirmed (HW)** means observed
on a real Obsid strip: model `Chameleon2`, firmware `fw_ver: 36`, tested 2026-10-09.

## Sources

| Source | Use | License |
|---|---|---|
| [SignalRGB Yeelight plugin](https://gitlab.com/signalrgb/yeelight) (`Yeelight.js`, commit `948d1ed`) | Chameleon2 model id, LED limits, method names | **No license file** — used only as a reference, no code copied |
| [Yeelight Chroma Connector](https://github.com/Yeelight/Yeelight-Chroma-Connector) incl. `Yeelight_Inter_Operation_Spec_UDP_R2_en.pdf` | Official UDP session handshake, keep-alive, session loss detection | BSD-3-Clause |
| [Yeelight Inter-Operation Spec](https://www.yeelight.com/download/Yeelight_Inter-Operation_Spec.pdf) | Discovery message format, classic method params | Vendor doc |
| [Yeelight forum: CubeMatrix `update_leds`](https://forum.yeelight.com/t/topic/33649) | `update_leds` encoding = Base64 | — |

## Discovery (UDP 1982)

- Request (Confirmed HW): `M-SEARCH * HTTP/1.1` / `HOST: 239.255.255.250:1982` /
  `MAN: "ssdp:discover"` / `ST: wifi_bulb`, sent to the multicast group and the
  subnet broadcast address. The device answers with a unicast `HTTP/1.1 200 OK`.
- Response fields used: `id` (stable, e.g. `dcda0cd778bc` — 12 hex digits on
  Chameleon2, not the `0x…` form of older bulbs), `model`, `Location`
  (`yeelight://<ip>:55443` — the classic TCP port, **not** the UDP control port),
  `name`, `fw_ver`, `support`, `power`.
- Chameleon2 fw 36 `support` (Confirmed HW): `get_prop set_default set_power toggle
  set_bright set_scene cron_add cron_get cron_del start_cf stop_cf set_name
  set_adjust adjust_bright set_ct_abx adjust_ct adjust_color set_rgb set_hsv
  set_music set_music_data udp_sess_new udp_sess_keep_alive udp_chroma_sess_new
  get_supported_fx_mode activate_fx_mode deactivate_fx_mode update_music_data update_leds`.
  No `bg_*` methods.
- **Windows pitfall (Confirmed HW):** a VPN/TUN adapter holding the default route
  swallows a multicast sent from an unbound socket. Discovery therefore sends
  through every operational adapter (`GetAdaptersAddresses` + `IP_MULTICAST_IF`)
  and to each subnet's directed broadcast.
- Windows firewall admits unicast replies to a multicast/broadcast request for
  only a few seconds; the request is resent every second while collecting.
- **Unicast M-SEARCH is ignored (Confirmed HW, fw 36):** a search sent directly to
  `<ip>:1982` gets no reply, so `discovery::probe` cannot verify a saved IP and
  `reconnect::resolve` always falls back to multicast discovery. The saved-IP fast
  path needs another identity check (Stage 1).
- The response also advertises the current state (`bright`, `color_mode`, `ct`,
  `rgb`, `hue`, `sat`); `obsid-poc watch` uses it to observe the strip without
  opening a control session.

## Session (UDP 55444)

All messages are one-line JSON terminated by `\r\n`.

| Step | Message | Status |
|---|---|---|
| Open | `{"id":1,"method":"udp_sess_new","params":[]}` | Confirmed HW |
| Token | `{"id":1, "method":"udp_sess_token", "params":{"token":"<32 hex>"}}` | Confirmed HW. Same id as the request on fw 36; the official PDF example shows a different id, so the handshake is matched by method, not id |
| Keep-alive | `{"id":N,"method":"udp_sess_keep_alive","params":["keeplive_interval","10"],"token":"…"}` → `{"id":N, "result":["ok"], "token":"…"}` | Confirmed HW. `keeplive` spelling is the device's |
| Command | `{"id":N,"method":…,"params":[…],"token":"…"}` → `{"id":N, "result":["ok"], …}` | Confirmed HW |
| Close | none documented; the session simply expires | Inferred |

- The official connector sends a keep-alive every 3 s and treats 3 missed replies
  as a lost session; this app does the same (`SessionConfig`).
- Replies contain spaces after commas — parse as JSON, never by string matching.
- The token is kept in memory only, redacted in logs (`packets::redact`) and in
  `Debug`/`Display` of `Token`.
- **Unknown:** the exact error returned for an expired/invalid token. The session
  treats any device error whose message mentions "token" or "session" as a lost
  session and re-handshakes on the next command.

## Commands

| Method | Params used | Status |
|---|---|---|
| `set_power` | `["on"\|"off","sudden",0]` | Confirmed HW (`on`) |
| `set_bright` | `[1..100,"sudden",0]` | Confirmed HW |
| `set_rgb` | `[r*65536+g*256+b,"sudden",0]` (black rejected client-side) | Confirmed HW |
| `set_hsv` | `[hue,sat,"sudden",0]` | Confirmed HW (used to restore Apple Home state) |
| `get_prop` | `["power","bright","rgb","color_mode",…]` → strings; unknown names → `""` | Confirmed HW over UDP |
| `get_supported_fx_mode` | `[]` → `["music_spectrum","music_breathing","music_gather","music_blossom","direct"]` | Confirmed HW |
| `activate_fx_mode` | `[{"mode":"direct"}]` → `ok` | Confirmed HW. An unknown mode gets **no reply at all** |
| `update_leds` | `["<base64 of R,G,B bytes per LED>"]` | Confirmed HW at 20 FPS for 60 LEDs. **The device sends no reply**, so frames are fire-and-forget |
| `deactivate_fx_mode` | `[{"mode":"direct"}]` → `ok` | Confirmed HW. Outside an fx mode it answers `ok` to any params |

### LED packing

Each LED is 3 bytes `R,G,B`; the concatenation is standard Base64 (4 chars/LED,
never padded). 120 LEDs → 480 chars → a ~560-byte datagram, below the 1400-byte
limit enforced by `encode_request`.

## Leaving direct mode (spec §4.3)

Tested 2026-10-09, fw 36, 60 LEDs, 5 s rainbow stream at 20 FPS, then:

1. `deactivate_fx_mode [{"mode":"direct"}]` → `ok` (22 ms), then `set_rgb`, `set_bright` → `ok`.
2. `get_prop` right after: `power=on bright=100 rgb=<target> color_mode=1`.
3. 15 s of complete silence (no frames, no keep-alive → session expired), new
   session, `get_prop`: unchanged.

Protocol-level result: `deactivate_fx_mode` is accepted and the static state is
retained after the session ends. **Still to confirm visually and with Apple Home**
(see `docs/testing.md`): that the LEDs actually show the static colour rather
than the last streamed frame, and that Apple Home controls the strip afterwards.
The alternative strategies (`set-rgb`, `power-cycle`, `expire`) remain in the PoC
for comparison.

### Apple Home hand-over (under investigation)

2026-10-09, user run of `exit-test deactivate` (fw 36): LEDs behaved as expected
(blue → rainbow → static colour held for 25 s), but Apple Home then showed the
strip as not responding; it recovered by itself after ~2 minutes, and the strip
afterwards reported the colour Apple Home had before the test (`color_mode=3`,
hue 30, sat 67, bright 100) instead of the test colour. Root cause not yet known.
Candidate causes being separated with `obsid-poc watch`:

1. the strip reboots/drops off Wi-Fi (watch shows `NO REPLY`);
2. the strip stays on the LAN but its Matter side is blocked while/after a LAN
   UDP session or fx mode is active;
3. the Home hub loses its Matter subscription during the 20 FPS stream and only
   re-subscribes later (strip itself fine).

Second occurrence, 2026-10-09 ~19:24 UTC: only `obsid-poc session` was run
(`udp_sess_new` + one keep-alive, no commands, no direct mode) while
`obsid-poc watch` sent a multicast search every second. Apple Home went to
"no response" and had not recovered after 5+ minutes, even after `watch` was
stopped. During the whole time the strip stayed on the LAN (`watch` never lost
it), its advertised state did not change, and it kept answering mDNS for
`_matter._tcp` with three operational Matter instances (fabrics). So:

- cause 1 (reboot / Wi-Fi drop) and cause 3 (stream load) are ruled out for this
  occurrence;
- the strip's network stack and Matter advertisement are alive; the failure is at
  the Matter session level between the hub and the strip;
- still confounded: the UDP session vs. the 1 Hz multicast search.

Apple Home recovered on its own later, with no reboot of the strip or the hub
(outage > 5 min, exact length not recorded). Both outages were transient.

Controlled run (single trial): `discover` followed by `session`, without `watch`
→ Apple Home kept working. So a bare UDP session (handshake + keep-alive) and a
one-off search are not sufficient to cause the outage. Remaining suspects: the
20 FPS stream / fx mode (outage 1) and the 1 Hz search loop of `watch` (outage 2),
or an intermittent fault unrelated to this app.

Repeat of `exit-test deactivate` without `watch` (single trial): Apple Home kept
working. Outage 1 is therefore **not reproducible** so far.

**Current conclusion:** the outages are intermittent and not tied to any single
protocol step tested in isolation; root cause unknown. Mitigations for the app,
independent of the cause: no periodic discovery while connected, no polling of
the strip, keep-alive only while a session is actually needed, and re-check
Apple Home after mode switches during Stage 4 soak tests.

## Open questions

- Physical LED count and the hardware behaviour when `update_leds` carries more or
  fewer colours than physical LEDs (needs `obsid-poc direct --pattern ends`).
- What the strip does when a direct-mode session expires *without*
  `deactivate_fx_mode` (relevant for crash/abnormal exit).
- Error format for an expired token.
- Whether `udp_chroma_sess_new` differs from `udp_sess_new` (not needed so far).
- Whether `set_bright` scales `update_leds` colours in direct mode (the app pins
  hardware brightness to 100 and scales frames itself; checklist A10).
- Stability of 25–30 FPS streams and of the Apple Home hand-over after long
  Ambilight sessions (checklist A9).
