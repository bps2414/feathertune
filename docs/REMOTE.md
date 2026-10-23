# Phone remote: plan

pt-BR: [REMOTE.pt-BR.md](REMOTE.pt-BR.md)

A phone app that controls feathertune on a paired PC over the local network. It does not play music: it shows what the PC plays (track, artwork, position, queue) and sends play/pause, next, previous, seek, volume and "play this queue item". Later the same protocol links several feathertune PCs and phones into one small network of devices (a device picker, one PC controlling another, moving playback between PCs). Not started yet; this is the plan contributors should build from. Questions and progress go in the tracking issue.

## Does this exist already?

Checked in October 2026, so we do not rebuild something the service or the PC already gives:

- **The service's own apps cannot do it.** The phone app casts to TVs, speakers and streamers and then acts as their remote, but a computer's browser is only ever a cast *sender*, never a target, and nothing on the phone lists or controls a browser tab. Since mid-2025 the service syncs the queue and the position between the phone and the web ("resume where you left off", from early 2026 between phones too): that is a hand-off you tap on the device you move to, not live control and not a device picker. feathertune hosts the official site, so this resume already works in it, in both directions; we do not rebuild it.
- **One stream per account.** A paid account streams music on one device at a time: starting a second one pauses the first ("account in use in another location"). Playing the same account in two rooms at once is therefore out, and nothing here tries to work around it.
- **Basic buttons already work from a generic remote.** feathertune has its own entry in the Windows media session (`smtc.rs`), so a phone app that remote-controls the PC's media session already gets play/pause, next, previous, seek and the system volume. Such apps exist (some commercial, with ads or a paid tier). They cannot show or play the queue, tell an ad from a track, set the player's own volume, or show the album colors and lyrics.
- **Other unofficial desktop clients have remotes.** The two largest serve plain HTTP or WebSocket on every interface (`0.0.0.0`), approve a client once with a popup or a code, and then trust a bearer token sent in clear over the LAN; anyone on the Wi-Fi who sees it can control the player.

So the remote is worth building for the queue, ad awareness, the look, several PCs and a stronger security model; the basic buttons are a side effect, not the reason.

## Goals and limits

- **Only paired devices control a PC, and only the PC they paired with.** A phone on the same Wi-Fi that was never paired, a stolen pairing code after it was used, a replayed message or a device the user revoked gets nothing: no state, no commands.
- Local network only. No cloud relay, no account, no port forwarding, no UPnP. Remote use from outside the house works only through the user's own VPN (e.g. a mesh VPN), which looks like the LAN to both sides.
- Off by default (Experimental tab), like every feature that opens something to the outside.
- The rules of [AGENTS.md](../AGENTS.md) apply unchanged: nothing during an ad is skipped, shortened or muted (seek, next, previous, queue jumps and moving playback are refused while one plays, and volume does not change); no audio leaves the PC; no brand names in the app name, store listing or UI.
- Never cast to TVs or speakers and never control the service's apps on other devices: both need the service's private pairing and casting protocols (no private APIs), and the WebView2 has cast discovery turned off anyway (docs/DESIGN.md, −12 MB). For the TV, the user casts from the service's phone app, and its queue sync carries on from the PC.
- Never play on two devices at once (see "One stream per account" above).

## Device network

The picture other services call "connect": every device the user owns sees the others and any of them can steer the one that plays. What fits the limits above:

- **Players** are feathertune PCs. **Controllers** are phones and, later, other feathertune PCs. A PC is both.
- Each player has a **device id**: the first 8 bytes of SHA-256 of its static public key, hex. Stable while its key is, never the PC's hostname.
- A controller keeps one entry per paired player (id, user-given name, public key, last addresses). Pairing is per pair of devices; there is no shared group key and no device trusts another because a third one does.
- **Discovery** (stage 2) is mDNS (`_feathertune._tcp`). The announcement must not identify the PC to strangers: no name and no fixed id, only the port, the protocol version and a tag that only paired devices recognize (an HMAC of the current hour under a key exchanged at pairing). Announced only while the plugin is on.
- **Device picker** (stage 2): the phone lists its paired PCs with online/offline and what each plays, from a short connection to each while the picker is open; it stays connected only to the chosen one.
- **Moving playback between PCs** (stage 3): from the picker, "play here" on PC B pauses PC A first, then B opens the same track, at the same position, in the same playlist (`watch?v=<id>&list=<list>` on the official site, then a seek once it plays; an ad that comes first plays in full). It is a navigation of the official site, nothing more. Build it only if the service's own resume proves too slow or manual in practice; check that first.
- **Continue on the phone** (stage 3): pauses the PC and opens the track at its position in the service's own phone app through a normal link. It plays there under the service's rules; our app still plays nothing.
- **Out, and why:** TVs, speakers and other browsers as targets (private protocols); several rooms in sync on one account (one stream per account); a cloud relay like other services use to reach devices on other networks (our server, accounts and an attack surface; the user's VPN does it instead).

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
- `expo-camera` (QR scanner), `expo-secure-store` (keys), `expo-haptics`, `expo-local-authentication` (optional app lock), `expo-localization` (English and pt-BR, like the desktop).
- Crypto: `@noble/curves`, `@noble/ciphers`, `@noble/hashes` (pure JS, audited, run on Hermes), randomness from `expo-crypto`.
- State: one small store (`zustand`). No other state or UI kit.
- A development build (`expo-dev-client`), not Expo Go: the local network permissions and, later, mDNS need native config.

**PC: a new module `remote.rs` in `app/`**, a plugin (`remote`, Experimental). `tokio` is already a dependency (`sync`, `time`); it gains `net`. New crates, each needed and small: `tokio-tungstenite` (WebSocket server), `snow` (Noise protocol; only 25519, ChaChaPoly and SHA256), `futures-util` (the `Stream`/`Sink` traits of the socket; already in the tree through `tokio-tungstenite`), `qrcodegen` (pairing QR, drawn by the settings page as SVG). Stage 2 adds `mdns-sd`. The PC's private key goes in the Windows credential store through `keyring` (already used for Last.fm).

**Folder:** `mobile/` at the repo root, next to `app/`. It is a separate app with its own `package.json`, but the protocol must change with the desktop in the same pull request, so one repo. Node via `.nvmrc`, pnpm (`node-linker=hoisted` in `mobile/.npmrc`, which Expo needs).

## Security model

Noise Protocol Framework (`25519`, `ChaChaPoly`, `SHA256`) over a WebSocket, binary frames. Noise is a published, analysed handshake (WireGuard and others use it); we do not design our own crypto. The desktop uses `snow`; the phone implements the two patterns below on top of `@noble`, and must pass the official Noise test vectors (cacophony) for them in CI before any other phone code is merged.

**Keys.** The PC has one static X25519 key pair, made on first use. Each phone makes its own key pair at install time. Private keys never leave the device's secure store. "Forget all devices" in the settings makes a new PC key pair, so every old pairing (and any copy of it) stops working at once.

**Pairing (`Noise_IKpsk2`).**
1. The user turns on the plugin and presses "Pair a phone" in the settings window. The PC shows a QR code for 2 minutes holding: protocol version, the PC's LAN addresses and port, the PC's public key and a fresh 32-byte one-time secret. No pairing is possible while that screen is closed.
2. The phone scans it and runs `IKpsk2`: it already knows the PC's key (so a fake PC on the network cannot answer) and proves it saw the QR (the secret is the PSK). Its first message carries its public key and a name the user typed ("Pixel 9").
3. The PC asks the user in the settings window: "Allow <name> to control feathertune?" (shows a 6-digit code also shown on the phone, derived from the handshake hash, so the two screens must match). Only on Allow does the PC store the phone's public key and name.
4. The one-time secret is discarded after one use, a failure or the 2 minutes, whichever comes first. Bytes that are not a first message for this PC (anyone on the network can send them) do not count as a try, so they cannot close the window.
5. The PC sends its name (the one the user set in the Remote section; by default "feathertune", never the Windows hostname, which often holds a person's name) and, from stage 2, the discovery key.

A PC cannot scan a QR, so pairing one PC with another (stage 3) needs another first step: the target shows a one-time code of at least 80 bits (16 base32 characters, typed on the other PC), found by mDNS, used as the PSK, with the same 6-digit check and the same 2 minutes. To be reviewed before it is built.

**Every connection after that (`Noise_KK`).** Both sides know each other's static key. The PC accepts only keys in its paired list; anything else is closed after the first message, before any state is sent. Each connection has new ephemeral keys (forward secrecy) and Noise's nonces reject replays and reordering.

**Other rules.**
- Accept connections only from private, link-local and loopback source addresses (RFC 1918, `fc00::/7`, `fe80::/10`, CGNAT `100.64/10` for mesh VPNs); others are dropped before the handshake.
- At most 4 paired devices connected at once; handshakes rate-limited per source address (e.g. 5 a minute), bad ones logged without data.
- Message size cap (64 KB from the PC, 1 KB from a controller); every command is parsed into a fixed shape (`deny_unknown_fields`), numbers finite and in range, ids checked like `player.rs` does.
- The settings window lists the paired devices (name, paired on, last seen) with Remove; removing one closes its connection at once. Turning the plugin off closes the port and stops the mDNS announcement.
- The phone keeps the list of paired PCs and an optional app lock (biometrics). It never logs message contents.
- What a controller sees is what the user sees in the window: track, artist, album, artwork URL, position, volume, queue (ids, titles, artists; at most 200). No history, no account data, no stats.
- `capabilities/` does not change: the page gets no new command. Commands from a controller go through Rust to `window.__feathertune.controls`, like the tray and the media flyout already do.

The Security section of docs/DESIGN.md gets a paragraph for this when it lands.

## Protocol

Everything a controller needs to talk to the PC (`app/src/remote.rs`; its tests run a `snow` initiator through every step below). Version 1.

**Transport.**
- WebSocket over TCP, `ws://<address>:<port>`, default port **47652** (`[remote] port` in `config.toml`). Not `wss`: Noise is the encryption. Paths: `/v1/pair` for pairing, `/v1` for every later session; any other path gets HTTP 404.
- One binary frame is one Noise message (at most 65 535 bytes). A text frame closes the connection (`invalid`). A frame from the controller is at most 1 040 bytes (1 024 of JSON plus the 16-byte tag); a larger one closes it (`too_big`). Messages from the PC hold at most 65 519 bytes of JSON.
- The PC drops, before the WebSocket handshake (the TCP connection just closes), sources outside `10/8`, `172.16/12`, `192.168/16`, `169.254/16`, `127/8`, `100.64/10`, `::1`, `fc00::/7`, `fe80::/10` (IPv4-mapped IPv6 counts as IPv4), and a sixth connection from one address within a minute.
- Each step waits at most 10 s: the WebSocket upgrade, the first Noise message, the first `ping` of a session.
- Close frames travel in clear (they are WebSocket frames, not Noise messages); their codes are listed below.

**Pairing QR.** One URI, lowercase hex:

```
feathertune://pair?v=1&k=<PC static public key, 64 hex>&s=<one-time secret, 64 hex>&a=<IPv4 addresses, comma-separated>&p=<port>
```

The controller ignores parameters it does not know. `a` holds the address of the PC's interface on its default route (one today). The PC's device id is the first 8 bytes of SHA-256 of the 32 bytes of `k`, in hex.

**Pairing, `/v1/pair`.** `Noise_IKpsk2_25519_ChaChaPoly_SHA256`; the phone is the initiator, the responder's static key is `k`, the PSK is `s` (position 2), and the prologue is the 25 ASCII bytes `feathertune-remote/1 pair`.
1. Phone → PC, handshake message 1 (`e, es, s, ss`). Its payload is the UTF-8 JSON `{"name":"<phone name>"}`, that key only; the name is 1 to 32 characters once trimmed, with no control characters.
2. PC → phone, handshake message 2 (`e, ee, se, psk`), empty payload. A phone that cannot decrypt it is talking to something that does not have the secret: it stops.
3. Both compute the **6-digit code**: the first 4 bytes of the handshake hash (`h` once message 2 is done, snow's `get_handshake_hash`) as a big-endian unsigned 32-bit number, modulo 1 000 000, padded with zeros to 6 digits. The phone shows it.
4. Phone → PC, the first transport message: `{"t":"pair"}`. It proves the phone used the secret (the PSK only enters with message 2), so only now does the PC show "Allow <name>?" with the same code.
5. Allow: the PC stores the phone, and the connection goes on as a session (below), starting with `hello`; the phone stores the PC (`k`, `hello`'s name, the address it used) when that `hello` arrives. Deny: closed with `denied`. The 2 minutes ending or the settings screen closing: `expired`.

The secret is used once: the first message 1 that the PC can read (made with `k`) takes it, whatever follows (a second phone, or the same one again, gets `expired`). One it cannot read is closed with `invalid` and leaves the window open.

**Session, `/v1`.** `Noise_KK_25519_ChaChaPoly_SHA256`; the phone is the initiator with its own key pair and the PC's `k`, and the prologue is the 20 ASCII bytes `feathertune-remote/1`. Message 1 (`e, es, ss`) and message 2 (`e, ee, se`) have empty payloads. The PC tries message 1 against each paired key; with none it closes with `unknown`, without answering. After message 2 the phone's first transport message must be a `ping`; only then does the PC send `hello`, `state`, `queue` and the `pong`, in that order (a replayed message 1 gets message 2 and nothing more). At most 4 sessions at once (`busy`); the same phone connecting again replaces its old session.

After the handshake each Noise transport message is one JSON object, parsed into a fixed shape: unknown fields, wrong types and out-of-range values are refused.

PC → controller:
- `{"t":"hello","v":1,"min":1,"name":"<PC name>"}` once. A controller outside `min`..`v` closes and says which side to update.
- `{"t":"state","track":{"id","title","artist","album","artwork","duration"},"position":s,"paused":b,"ad":b,"volume":0-100,"muted":b,"colors":[...],"at":ms}`: on every `player::Event`, on connect (once something has played) and when the album colors arrive. `id` (the 11-character video id) and `artwork` (an https URL) may be `null`; `duration` is in seconds, 0 while unknown. `position` is in seconds as of `at` (Unix milliseconds, PC clock): the controller moves it on by itself while `paused` is false (the clock offset is estimated from `ping`/`pong`). `colors` is `["#accent","#ink","#text on accent"]` from the cover, or `[]`; after a track change they come in a later `state`.
- `{"t":"queue","items":[{"id","title","artist"}],"current":i}`: on connect and when the queue changes. At most 200 items, cut from the end if the message would not fit; `current` is `null` when not known.
- `{"t":"error","code":"ad"|"unavailable"|"invalid"}` after a refused message: `ad` while an ad plays, `unavailable` when the page could not be reached, `invalid` for anything malformed. A command that worked gets no reply; the next `state` shows it.

Controller → PC:
- `{"t":"cmd","c":"play"|"pause"|"next"|"previous"}`
- `{"t":"cmd","c":"seek","s":seconds}`: 0 to 86 400.
- `{"t":"cmd","c":"volume","v":0-100}`
- `{"t":"cmd","c":"play_item","id":"<video id>","i":index}`: plays that queue row (both must match, so a stale index never plays the wrong song); `i` 0 to 199.
- `{"t":"ping","n":n}` → `{"t":"pong","n":n,"at":ms}`: first thing in a session, then every 15 s; the PC closes a connection silent for 45 s. `n` is an unsigned integer.

During an ad every command but `play` and `pause` is refused with `ad` (pausing shortens nothing).

Close codes (the reason is the word in brackets):

| Code | Reason | When | Phone |
|---|---|---|---|
| 4000 | `denied` | the user pressed Deny | says so; pair again to retry |
| 4001 | `expired` | no pairing screen open, the 2 minutes are over, or the secret was already used | scan a new QR |
| 4002 | `unknown` | `KK` with a key the PC does not have (removed, or Forget all devices) | "removed by the PC": pair again |
| 4003 | `removed` | the user removed this phone (or forgot all) while connected | "removed by the PC": pair again |
| 4004 | `busy` | 4 sessions already open | retry later |
| 4005 | `timeout` | 45 s without a message (10 s during a handshake) | reconnect |
| 4006 | `invalid` | a message that does not decrypt (replayed, reordered, forged), a text frame, a bad handshake | reconnect; report if it repeats |
| 4007 | `too_big` | a frame over 1 040 bytes | a bug in the controller |
| 4008 | `off` | the plugin was turned off | "the remote is off on the PC" |
| 4009 | `full` | 16 phones already paired | remove one on the PC |

Several controllers at once all get every `state`; commands apply in arrival order (the last one wins). Stage 3 adds `{"t":"cmd","c":"hand_off"}` (PC A → its paused position and playlist for PC B) and `{"t":"cmd","c":"open","id","list","s"}` (PC B opens it), both refused during an ad, and `v` becomes 2.

## Lifecycle

- **PC:** the listener runs while the plugin is on and the app runs, window hidden or in the tray included (that is when a remote is most useful). When Windows sleeps, connections drop and controllers show the PC offline. Waking it from the phone (Wake-on-LAN, the MAC address sent at pairing) is a later idea, not stage 1.
- **Phone:** connected only while the app is in the foreground; it closes the socket a few seconds after going to the background, so it costs no battery when unused. Lock-screen and notification controls (stage 2) need an Android foreground service while a PC plays; iOS does not keep a socket in the background, so there it stays foreground-only.
- **Reconnect:** last good address, then the mDNS answer (stage 2), then each address from the QR; backoff 1, 2, 4… up to 30 s, only in the foreground.
- **Windows Firewall:** the first listen shows Windows' prompt; the app adds no rules. On a network Windows marks as Public, inbound connections are blocked: the Remote section says so and how to mark the network Private.

## Desktop work

1. `player.rs`: add `volume`, `muted` and `ad` to the state the `core` injection reports (validated like the rest). Keep the last queue and its current index in `Player` and send an event when it changes: today the queue goes only to `mixes::observe`, and the `core` injection sends it only when the URL has a playlist (`list=`); the remote needs it for radio and autoplay too.
2. `injections/controls/page.js`: add `seek(s)`, `setVolume(v)` (through `precise_volume` when loaded) and `playItem(id, i)` (clicks the queue row), each refusing during an ad, like `volume` does today.
3. `remote.rs`: listener (only while the plugin is on), Noise handshakes, the paired list in the profile (`remote-devices.json`: names and public keys only), subscribe to `Player`, translate commands to `crate::player`. Its tests run a `snow` initiator against it in-process: pairing, `KK`, a revoked key, an unknown key, a replay, an oversized and a malformed command. That covers the PC side in CI without a phone.
4. Settings: a Remote section in the Experimental tab (switch, PC name, Pair a phone with the QR, the confirmation, the paired list, Forget all devices, the Public network note). Texts through `t()` + `pt-BR.json`.
5. `plugins.rs` and docs/DESIGN.md: plugin entry, cost "not measured yet", Security paragraph. The listener costs nothing measurable when off (no socket).

## Phone work

1. Scaffold `mobile/` (Expo SDK 58, TypeScript, Expo Router, dev client), lint and typecheck.
2. `mobile/src/noise/`: `IKpsk2` and `KK` on `@noble`, with the official test vectors as tests. CI job (Ubuntu): `pnpm install --frozen-lockfile`, typecheck, lint, tests.
3. `mobile/tools/fake-pc`: a Node script that plays the PC side (pairing QR in the terminal, `KK`, a made-up track and queue that move, refusals during a fake ad) with the same `noise/` code. Phone work then needs no Windows PC and no account, and contributors on any OS can build every screen.
4. Pairing screens: welcome, camera permission, scan, name, matching code, done. Android 17+ (`ACCESS_LOCAL_NETWORK`, target API 37) and iOS (`NSLocalNetworkUsageDescription`) permission asked with an explanation first.
5. Now playing: artwork large with a backdrop from the album colors, title/artist, seek bar (drag with haptics, sends one `seek` on release), play/pause, previous/next, volume slider. Controls greyed out with a short note during an ad.
6. Queue: a list with the current row marked; tap plays it (`play_item`). Pull down from now playing, or a tab.
7. PCs: list of paired PCs, connect/reconnect, remove. Clear states for "PC offline or asleep", "not on the same network", "Windows is blocking the connection (Public network)", "removed by the PC" (asks to pair again), "update the app" / "update feathertune".
8. Look: dark first, follows the system theme; one accent from the album; motion respects the OS reduce-motion setting here (the phone is not the desktop page, so the AGENTS rule about it does not apply), and animations stop when the app is in the background. Every control has an accessibility label and text follows the system font size.

## Stages

1. **Phone → one PC** (everything above): pair, now playing, queue, several paired PCs switched by hand.
2. **Find and pick:** mDNS discovery, the device picker with each PC's status, lock-screen and notification controls, lyrics on the phone (the PC already has the lines), like/dislike and shuffle/repeat (each needs page selectors in `controls` and a reason in DESIGN.md).
3. **PCs together:** one feathertune PC controlling another (same protocol, a `snow` client in `remote.rs`, a picker in the tray menu), moving playback between PCs, continue on the phone, reorder or remove queue items. Wake-on-LAN if asked for.

Each stage ships on its own and keeps the earlier ones working; `hello`'s `min` keeps an older phone app usable for as long as the commands it knows still exist.

## Open questions

For the maintainer, with the running app and a signed-in account:
- Does the service's own resume already move playback between two feathertune PCs well enough? If so, stage 3's hand-off is dropped.
- Does a seek right after opening a track by URL land reliably (and after an ad)?
- Does `playItem` by clicking a queue row work for autoplay and radio queues, not only playlists?

## Releases

Android: an APK attached to the GitHub release (built with EAS or locally), and maybe a store later. iOS: EAS Build plus TestFlight needs the maintainer's Apple developer account; until then, Android only. Version numbers of `mobile/` are separate from the desktop's; what must match is the `hello` message's `min`..`v` range.

## How to help

Pick one numbered item above, comment on the tracking issue that you are on it, and open one pull request per item (see [CONTRIBUTING.md](../CONTRIBUTING.md)). Desktop items 1–2 and phone items 1–3 have no dependencies and can start now. Say what you could test: the desktop builds only on Windows, and anything that needs the running app with a signed-in account is checked by the maintainer.
