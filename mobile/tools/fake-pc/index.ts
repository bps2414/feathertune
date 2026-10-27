// `pnpm fake-pc`: the PC side of the phone remote in the terminal (see server.ts).
//   --port <n>       port to listen on (default 47652)
//   --name <text>    the PC name shown on the phone (default feathertune)
//   --artwork <url>  https artwork for every track (default none)
//   --no-rate-limit  no limit on connections per address (the real PC allows 5 a minute)
//   --stuck-drag     ignore queue_move, like a page that did not take the drag

import { join } from 'node:path';
import { emitKeypressEvents } from 'node:readline';

import { bytesToHex, hexToBytes } from '@noble/hashes/utils.js';
import { sha256 } from '@noble/hashes/sha2.js';
import qrcode from 'qrcode-terminal';

import { DEFAULT_PORT, FakePc, lanAddresses } from './server';

function option(name: string): string | undefined {
  const at = process.argv.indexOf(`--${name}`);
  return at >= 0 ? process.argv[at + 1] : undefined;
}

const port = Number(option('port') ?? DEFAULT_PORT);
const artwork = option('artwork');
if (!Number.isInteger(port) || port < 1024 || port > 65535) fail('--port must be 1024 to 65535');
if (artwork && !artwork.startsWith('https://')) fail('--artwork must be an https URL');

function fail(message: string): never {
  console.error(`fake-pc: ${message}`);
  process.exit(1);
}

const say = (line: string) => console.log(line);

const pc = new FakePc({
  name: option('name'),
  artwork,
  storePath: join(__dirname, 'fake-pc.json'),
  perMinute: process.argv.includes('--no-rate-limit') ? Infinity : undefined,
  stuckDrag: process.argv.includes('--stuck-drag'),
  log: (line) => say(`  ${line}`),
  onAsk: ({ name, code }) => say(`\nAllow "${name}" to control feathertune? The phone shows ${code}.  [y] allow  [n] deny`),
});

async function showPairing(): Promise<void> {
  const uri = await pc.startPairing();
  qrcode.generate(uri, { small: true });
  say(uri);
  say('Scan the QR (or paste the URI) within 2 minutes.');
}

const help = () => say('\n[p] pair a phone   [a] ad on/off   [d] paired phones   [q] quit');

async function main(): Promise<void> {
  const addresses = await lanAddresses();
  if (addresses.length === 0) fail('no local network address found');
  await pc.listen(port).catch((error: Error) => fail(`could not listen on port ${port}: ${error.message}`));
  const id = bytesToHex(sha256(hexToBytes(pc.publicKey)).slice(0, 8));
  say(`fake PC "${option('name') ?? 'feathertune'}" (device ${id}) listening on ${addresses[0]}:${pc.listening}`);
  say('No audio, local network only. Playing: made-up tracks that move on by themselves.');
  if (pc.getDevices().length === 0) await showPairing();
  else say(`${pc.getDevices().length} paired phone(s); press p for a new QR.`);
  help();
  keys();
}

function keys(): void {
  const key = (char: string) => {
    if (pc.pending()) {
      if (char === 'y') pc.answer(true);
      else if (char === 'n') pc.answer(false);
      return;
    }
    switch (char) {
      case 'p':
        void showPairing();
        break;
      case 'a':
        pc.setAd(!pc.playback.ad);
        say(pc.playback.ad ? 'ad on: only play and pause are accepted' : 'ad off');
        break;
      case 'd':
        for (const d of pc.getDevices()) say(`  ${d.name}  ${d.connected ? 'connected' : 'offline'}  ${d.key.slice(0, 16)}…`);
        break;
      case 'q':
        void pc.close().then(() => process.exit(0));
        break;
      default:
        help();
    }
  };
  if (process.stdin.isTTY) {
    emitKeypressEvents(process.stdin);
    process.stdin.setRawMode(true);
    process.stdin.on('keypress', (char: string | undefined, info: { ctrl?: boolean; name?: string }) => {
      if (info.ctrl && info.name === 'c') key('q');
      else if (char) key(char.toLowerCase());
    });
  } else {
    process.stdin.setEncoding('utf8');
    process.stdin.on('data', (text: string) => text.split('\n').forEach((line) => line && key(line[0].toLowerCase())));
  }
}

void main();
