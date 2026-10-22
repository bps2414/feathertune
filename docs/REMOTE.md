# Phone remote: plan

pt-BR: [REMOTE.pt-BR.md](REMOTE.pt-BR.md)

A phone app that controls feathertune on a paired PC over the local network. It does not play music: it shows what the PC plays (track, artwork, position, queue) and sends play/pause, next, previous, seek, volume and "play this queue item". Not started yet; this is the plan contributors should build from. Questions and progress go in the tracking issue.

## Goals and limits

- **Only paired phones control a PC, and only the PC they paired with.** A phone on the same Wi-Fi that was never paired, a stolen pairing code after it was used, a replayed message or a phone the user revoked gets nothing: no state, no commands.
- Local network only. No cloud relay, no account, no port forwarding, no UPnP. Remote use from outside the house works only through the user's own VPN (e.g. a mesh VPN), which looks like the LAN to both sides.
- Off by default (Experimental tab), like every feature that opens something to the outside.
- The rules of [AGENTS.md](../AGENTS.md) apply unchanged: nothing during an ad is skipped, shortened or muted (seek, next, previous and queue jumps are refused while one plays, and volume does not change); no audio leaves the PC; no brand names in the app name, store listing or UI.

## Stack

**Phone: Expo (React Native), SDK 58, TypeScript.** Chosen over the alternatives because:

| Option | Why not (for this app) |
|---|---|
| Tauri 2 mobile | Would share Rust with the desktop, but the UI is a webview, mobile is the less mature side of Tauri (plugins not all ported), and an iOS build needs Xcode on a Mac. The maintainer works on Windows only. |
| Flutter | Good UI, but a third language (Dart) with nothing shared, and iOS builds also need a Mac or a paid CI. |
| Native (Kotlin + Swift) | Two codebases for one small app. |
| Rust UI (Dioxus, Slint) | Mobile support is young; touch, gestures, camera and secure storage would be hand-written. |

Expo gives a native UI with smooth gestures and animations, the camera (QR scan), the Keychain/Keystore and haptics as maintained modules, and EAS Build builds iOS in the cloud without a Mac. Libraries:

- `expo-router` (screens), `react-native-reanimated` + `react-native-gesture-handler` (seek bar, volume, sheet), `expo-image` (artwork with a blurred backdrop), `expo-linear-gradient` with the album colors the PC already computes (`album_color.rs`).
- `expo-camera` (QR scanner), `expo-secure-store` (keys), `expo-haptics`, `expo-local-authentication` (optional app lock).
- Crypto: `@noble/curves`, `@noble/ciphers`, `@noble/hashes` (pure JS, audited, run on Hermes), randomness from `expo-crypto`.
- State: one small store (`zustand`). No other state or UI kit.
- A development build (`expo-dev-client`), not Expo Go: the local network permissions and, later, mDNS need native config.

**PC: a new module `remote.rs` in `app/`**, a plugin (`remote`, Experimental). New crates, each needed and small: `tokio` features `net`/`rt` + `tokio-tungstenite` (WebSocket server), `snow` (Noise protocol), `qrcodegen` (pairing QR, drawn by the settings page as SVG). Later, `mdns-sd` for discovery. The PC's private key goes in the Windows credential store through `keyring` (already used for Last.fm).

**Folder:** `mobile/` at the repo root, next to `app/`. It is a separate app with its own `package.json`, but the protocol must change with the desktop in the same pull request, so one repo. Node via `.nvmrc`, pnpm (`node-linker=hoisted` in `mobile/.npmrc`, which Expo needs).

## Security model

Noise Protocol Framework (`25519`, `ChaChaPoly`, `SHA256`) over a WebSocket, binary frames. Noise is a published, analysed handshake (WireGuard and others use it); we do not design our own crypto. The desktop uses `snow`; the phone implements the two patterns below on top of `@noble`, and must pass the official Noise test vectors (cacophony) for them in CI before any other phone code is merged.

**Keys.** The PC has one static X25519 key pair, made on first use. Each phone makes its own key pair at install time. Private keys never leave the device's secure store.

**Pairing (`Noise_IKpsk2`).**
1. The user turns on the plugin and presses "Pair a phone" in the settings window. The PC shows a QR code for 2 minutes holding: protocol version, the PC's LAN addresses and port, the PC's public key and a fresh 32-byte one-time secret. No pairing is possible while that screen is closed.
2. The phone scans it and runs `IKpsk2`: it already knows the PC's key (so a fake PC on the network cannot answer) and proves it saw the QR (the secret is the PSK). Its first message carries its public key and a name the user typed ("Pixel 9").
3. The PC asks the user in the settings window: "Allow <name> to control feathertune?" (shows a 6-digit code also shown on the phone, derived from the handshake hash, so the two screens must match). Only on Allow does the PC store the phone's public key and name.
4. The one-time secret is discarded after one use, a failure or the 2 minutes, whichever comes first.

**Every connection after that (`Noise_KK`).** Both sides know each other's static key. The PC accepts only keys in its paired list; anything else is closed after the first message, before any state is sent. Each connection has new ephemeral keys (forward secrecy) and Noise's nonces reject replays and reordering.

**Other rules.**
- Accept connections only from private, link-local and loopback source addresses (RFC 1918, `fc00::/7`, `fe80::/10`, CGNAT `100.64/10` for mesh VPNs); others are dropped before the handshake.
- At most 4 paired phones connected at once; handshakes rate-limited per source address (e.g. 5 a minute), bad ones logged without data.
- Message size cap (64 KB from the PC, 1 KB from the phone); every command is parsed into a fixed shape (`deny_unknown_fields`), numbers finite and in range, ids checked like `player.rs` does.
- The settings window lists the paired phones (name, paired on, last seen) with Remove; removing one closes its connection at once. Turning the plugin off closes the port.
- The phone keeps the list of paired PCs and an optional app lock (biometrics). It never logs message contents.
- What the phone sees is what the user sees in the window: track, artist, album, artwork URL, position, volume, queue (ids, titles, artists; at most 200). No history, no account data, no stats.
- `capabilities/` does not change: the page gets no new command. Commands from the phone go through Rust to `window.__feathertune.controls`, like the tray and the media flyout already do.

The Security section of docs/DESIGN.md gets a paragraph for this when it lands.

## Protocol

After the handshake, each Noise transport message is one JSON object. Version 1:

PC → phone:
- `{"t":"hello","v":1,"name":"<PC name>"}` once.
- `{"t":"state","track":{id,title,artist,album,artwork,duration},"position":s,"paused":b,"ad":b,"volume":0-100,"muted":b,"colors":[...],"at":ms}`: on every `player::Event` and on connect. The phone moves the position itself from `at` (PC clock, sent with a `ping` to estimate the offset).
- `{"t":"queue","items":[{id,title,artist}],"current":i}`: on connect and when the queue changes.
- `{"t":"error","code":"ad"|"unavailable"|"invalid"}` after a refused command.

Phone → PC:
- `{"t":"cmd","c":"play"|"pause"|"next"|"previous"}`
- `{"t":"cmd","c":"seek","s":seconds}`
- `{"t":"cmd","c":"volume","v":0-100}`
- `{"t":"cmd","c":"play_item","id":"<video id>","i":index}`: plays that queue row (both must match, so a stale index never plays the wrong song).
- `{"t":"ping","n":n}` → `{"t":"pong","n":n,"at":ms}`

## Desktop work

1. `player.rs`: add `volume`, `muted` and `ad` to the state the `core` injection reports (validated like the rest).
2. `injections/controls/page.js`: add `seek(s)`, `setVolume(v)` (through `precise_volume` when loaded) and `playItem(id, i)` (clicks the queue row), each refusing during an ad, like `volume` does today.
3. `remote.rs`: listener (only while the plugin is on), Noise handshakes, the paired list in the profile (`remote-devices.json`: names and public keys only), subscribe to `Player`, translate commands to `crate::player`.
4. Settings: a Remote section in the Experimental tab (switch, Pair a phone with the QR, the confirmation, the paired list). Texts through `t()` + `pt-BR.json`.
5. `plugins.rs` and docs/DESIGN.md: plugin entry, cost "not measured yet", Security paragraph. The listener costs nothing measurable when off (no socket).
6. Windows Firewall: the first listen shows Windows' prompt; the app does not add rules itself. Document it.

## Phone work

1. Scaffold `mobile/` (Expo SDK 58, TypeScript, Expo Router, dev client), lint and typecheck.
2. `mobile/src/noise/`: `IKpsk2` and `KK` on `@noble`, with the official test vectors as tests. CI job (Ubuntu): `pnpm install --frozen-lockfile`, typecheck, lint, tests.
3. Pairing screens: welcome, camera permission, scan, name, matching code, done. Android 17+ (`ACCESS_LOCAL_NETWORK`, target API 37) and iOS (`NSLocalNetworkUsageDescription`) permission asked with an explanation first.
4. Now playing: artwork large with a backdrop from the album colors, title/artist, seek bar (drag with haptics, sends one `seek` on release), play/pause, previous/next, volume slider. Controls greyed out with a short note during an ad.
5. Queue: a list with the current row marked; tap plays it (`play_item`). Pull down from now playing, or a tab.
6. PCs: list of paired PCs, connect/reconnect (last address, then each address from the QR), remove. Clear states for "PC offline", "not on the same network", "removed by the PC" (asks to pair again).
7. Look: dark first, follows the system theme; one accent from the album; motion respects the OS reduce-motion setting here (the phone is not the desktop page, so the AGENTS rule about it does not apply), and animations stop when the app is in the background.

## Later (not in the first version)

- Discovery with mDNS (`_feathertune._tcp`; PC with `mdns-sd`, phone with a zeroconf module) so a new DHCP address is found without scanning again. The QR addresses are enough at first.
- Like/dislike, shuffle/repeat, reorder or remove queue items: each needs page selectors in `controls` and a reason in DESIGN.md.
- Media controls on the phone's lock screen or a widget.
- Lyrics on the phone (the PC already has the lines).

## Releases

Android: an APK attached to the GitHub release (built with EAS or locally), and maybe a store later. iOS: EAS Build plus TestFlight needs the maintainer's Apple developer account; until then, Android only. Version numbers of `mobile/` are separate from the desktop's; the `hello` message's `v` is what must match.

## How to help

Pick one numbered item above, comment on the tracking issue that you are on it, and open one pull request per item (see [CONTRIBUTING.md](../CONTRIBUTING.md)). Desktop items 1–2 and phone items 1–2 have no dependencies and can start now. Say what you could test: the desktop builds only on Windows, and anything that needs the running app with a signed-in account is checked by the maintainer.
