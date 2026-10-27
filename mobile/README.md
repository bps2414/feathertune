# feathertune remote (phone app)

pt-BR: [README.pt-BR.md](README.pt-BR.md)

The phone app that controls feathertune on a paired PC over the local network. Plan, security model and wire protocol: [docs/REMOTE.md](../docs/REMOTE.md).

```
pnpm install
pnpm typecheck && pnpm lint && pnpm test
```

## `pnpm fake-pc`: a PC without the PC

`pnpm fake-pc` plays the PC side of the protocol from your terminal, so you can build and test the app with no Windows machine and no account. It uses the same `src/noise` code as the app and follows the Protocol section of the plan exactly, including its refusals.

- It listens on your local network (port 47652, `--port` to change) and prints the pairing QR and its `feathertune://pair?…` text. Scan or paste it within 2 minutes.
- When a phone pairs, the terminal shows its name and the 6-digit code (the phone shows the same) and asks `y` or `n`.
- After that the phone gets a made-up track and queue that move on their own, and its commands (play, pause, next, previous, seek, volume, play a queue row, move, remove or play next a row, play a playlist of a made-up library) work. `--stuck-drag` ignores moves, like a page that did not take the drag.
- Keys: `a` turns the ad mode on or off (everything but play and pause is then refused with `ad`), `p` opens a new pairing QR, `d` lists the paired phones, `q` quits.
- Other options: `--name <text>` (the PC name), `--artwork <https url>` (artwork for every track; none by default), `--no-rate-limit` (the real PC allows 5 connections per address a minute).
- It plays no audio and drops anything outside the local network. The first run may trigger the Windows Firewall prompt for Node: allow private networks.
- The fake PC's key and the paired phones are in `tools/fake-pc/fake-pc.json` (git-ignored); delete it to start over.

## The connection layer and what needs a phone

`src/remote/` is the phone's side: `protocol.ts` (QR, code, messages, close codes), `client.ts` (one socket and its handshake) and `remote.ts` (the paired PCs, reconnecting, pings, the background), with storage, sockets and the network injected. `tools/fake-pc/remote.test.ts` runs it against the fake PC in process (pairing, reconnecting, a removed phone, a PC of another version), so `pnpm test` covers it on any OS.

Only a development build on a real phone checks the rest: the camera and the QR scan, the local network permission (Android 17's prompt, iOS's own on the first connection), the secure store, and a `ws://` connection to the PC over Wi-Fi.
