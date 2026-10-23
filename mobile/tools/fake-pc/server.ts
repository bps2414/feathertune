// The PC side of the phone remote, made up: the same wire protocol as app/src/remote.rs (docs/REMOTE.md,
// Protocol), with the Noise code the phone uses (src/noise) and a player that only moves numbers. It plays no
// audio, serves only the local network and keeps what it needs (its key, the paired phones) in one JSON file.

import { randomBytes } from 'node:crypto';
import { createSocket } from 'node:dgram';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { createServer, type Server } from 'node:http';
import type { Socket } from 'node:net';

import { bytesToHex, hexToBytes } from '@noble/hashes/utils.js';
import { WebSocketServer, type RawData, type WebSocket } from 'ws';

import { createHandshake, generateKeyPair, type KeyPair, type Transport } from '../../src/noise';
import { Playback } from './playback';

export const DEFAULT_PORT = 47652;
const VERSION = 1;
const MIN_VERSION = 1;
const PAIR_PROLOGUE = new TextEncoder().encode('feathertune-remote/1 pair');
const KK_PROLOGUE = new TextEncoder().encode('feathertune-remote/1');
const PAIRING_WINDOW_MS = 120_000;
const HANDSHAKE_TIMEOUT_MS = 10_000;
/** The phone pings every 15 s; three missed and the session is closed. */
const SILENCE_MS = 45_000;
const MINUTE_MS = 60_000;
const MAX_SESSIONS = 4;
const MAX_DEVICES = 16;
const MAX_NAME = 32;
const NOISE_MAX = 65_535;
const TAG = 16;
/** The largest plaintext from a controller, and the frame that carries it. */
const MAX_IN = 1024;
const MAX_FRAME = MAX_IN + TAG;
const MAX_OUT = NOISE_MAX - TAG;
const MAX_QUEUE = 200;
const MAX_SECONDS = 86_400;

/** Why the PC closes a connection: the WebSocket close code and reason (docs/REMOTE.md). */
const CODES = {
  denied: 4000,
  expired: 4001,
  unknown: 4002,
  removed: 4003,
  busy: 4004,
  timeout: 4005,
  invalid: 4006,
  too_big: 4007,
  off: 4008,
  full: 4009,
} as const;

type Why = keyof typeof CODES | 'gone';

/** Ends a connection for `why`. 'gone': the phone left, nothing to send. */
class Refuse extends Error {
  constructor(readonly why: Why) {
    super(why);
  }
}

export interface Pending {
  name: string;
  /** The 6 digits the phone shows too. */
  code: string;
}

export interface DeviceInfo {
  /** The phone's static public key, lowercase hex. */
  key: string;
  name: string;
  /** Unix seconds. */
  paired: number;
  connected: boolean;
}

export interface FakePcOptions {
  /** The PC name `hello` carries. */
  name?: string;
  /** The JSON file with the PC key and the paired phones; none keeps them in memory. */
  storePath?: string;
  /** The addresses the pairing QR holds; default: the interface on the default route. */
  addresses?: string[];
  /** The artwork URL of every track (https); default none. */
  artwork?: string;
  /** Called when a phone proved it saw the QR and waits for `answer`. */
  onAsk?: (pending: Pending) => void;
  log?: (line: string) => void;
  /** Handshakes one address may start a minute (the real PC: 5). */
  perMinute?: number;
  clock?: () => number;
  /** Milliseconds between player ticks (track ends, colors, a state every few seconds); 0: only `tick()`. */
  tickMs?: number;
}

type Command =
  | { c: 'play' | 'pause' | 'next' | 'previous' }
  | { c: 'seek'; s: number }
  | { c: 'volume'; v: number }
  | { c: 'play_item'; id: string; i: number };

type Incoming = { t: 'ping'; n: number } | { t: 'cmd'; cmd: Command };

interface Session {
  key: string;
  ws: WebSocket;
  send: (message: object) => void;
}

interface Pairing {
  until: number;
  secret: Uint8Array | null;
  asking: { name: string; code: string; settle: (result: 'allow' | 'deny' | 'expired') => void } | null;
  timer: NodeJS.Timeout;
}

interface Stored {
  key: { private: string; public: string };
  devices: { name: string; key: string; paired: number }[];
}

const random = (length: number) => new Uint8Array(randomBytes(length));

export class FakePc {
  readonly playback: Playback;
  private readonly name: string;
  private readonly addresses?: string[];
  private readonly storePath?: string;
  private readonly onAsk?: (pending: Pending) => void;
  private readonly log: (line: string) => void;
  private readonly perMinute: number;
  private readonly tickMs: number;
  private keys: KeyPair;
  private devices: Stored['devices'] = [];
  private pairing: Pairing | null = null;
  private sessions = new Set<Session>();
  private lastState = 0;
  private attempts = new Map<string, number[]>();
  private closing = new WeakSet<WebSocket>();
  private upgradeTimers = new WeakMap<object, NodeJS.Timeout>();
  private server: Server | null = null;
  private wss: WebSocketServer | null = null;
  private port = 0;
  private timer: NodeJS.Timeout | null = null;
  private clock: () => number;

  constructor(options: FakePcOptions = {}) {
    this.name = options.name ?? 'feathertune';
    this.addresses = options.addresses;
    this.storePath = options.storePath;
    this.onAsk = options.onAsk;
    this.log = options.log ?? (() => {});
    this.perMinute = options.perMinute ?? 5;
    this.tickMs = options.tickMs ?? 250;
    this.clock = options.clock ?? Date.now;
    this.playback = new Playback(this.clock, options.artwork ?? null);
    const stored = this.load();
    this.keys = stored ? { privateKey: hexToBytes(stored.key.private), publicKey: hexToBytes(stored.key.public) } : generateKeyPair(random);
    this.devices = stored?.devices ?? [];
    if (!stored) this.save();
  }

  /** The device id the PC has: the first 8 bytes of SHA-256 of its public key, hex. */
  get publicKey(): string {
    return bytesToHex(this.keys.publicKey);
  }

  get listening(): number {
    return this.port;
  }

  // ---- the API of the terminal (the real PC's settings window)

  /** Opens a pairing window of 2 minutes and returns the QR's text. A new call replaces the window. */
  async startPairing(): Promise<string> {
    const addresses = this.addresses ?? (await lanAddresses());
    if (!this.server || addresses.length === 0) throw new Error('not listening on a local network address');
    this.endPairing();
    const secret = random(32);
    const timer = setTimeout(() => {
      this.log('pairing window closed');
      this.endPairing();
    }, PAIRING_WINDOW_MS);
    timer.unref();
    this.pairing = { until: Date.now() + PAIRING_WINDOW_MS, secret, asking: null, timer };
    return pairingUri(addresses, this.port, this.keys.publicKey, secret);
  }

  /** Ends the pairing window; a phone waiting for an answer gets `expired`. */
  endPairing(): void {
    const pairing = this.pairing;
    this.pairing = null;
    if (!pairing) return;
    clearTimeout(pairing.timer);
    pairing.asking?.settle('expired');
  }

  pending(): Pending | null {
    const asking = this.pairing?.asking;
    return asking ? { name: asking.name, code: asking.code } : null;
  }

  /** Allow or deny the pending phone. False: none was pending. */
  answer(allow: boolean): boolean {
    const asking = this.pairing?.asking;
    if (!asking) return false;
    this.pairing!.asking = null;
    asking.settle(allow ? 'allow' : 'deny');
    return true;
  }

  getDevices(): DeviceInfo[] {
    return this.devices.map((d) => ({ ...d, connected: [...this.sessions].some((s) => s.key === d.key) }));
  }

  /** Forgets one phone and closes its session at once. False: no such phone. */
  remove(key: string): boolean {
    const found = this.devices.some((d) => d.key === key);
    this.devices = this.devices.filter((d) => d.key !== key);
    this.save();
    this.kick((s) => s.key === key, 'removed');
    return found;
  }

  /** Forgets every phone and makes a new PC key, so no old pairing (or a copy of it) works again. */
  forgetAll(): void {
    this.keys = generateKeyPair(random);
    this.devices = [];
    this.endPairing();
    this.save();
    this.kick(() => true, 'removed');
  }

  /** The ad mode: what the real PC does while one plays. */
  setAd(on: boolean): void {
    this.playback.setAd(on);
    this.broadcastState();
  }

  /** Moves the player on (track ends, colors arriving) and sends a state when something changed. */
  tick(): void {
    const before = this.playback.index;
    const changed = this.playback.tick();
    const idle = !this.playback.paused && !this.playback.ad && this.clock() - this.lastState >= 3000;
    if (changed || idle) this.broadcastState();
    if (this.playback.index !== before) this.broadcastQueue();
  }

  // ---- the server

  /** Listens on every interface (IPv6 and IPv4 when it can) or on `host`; returns the port. */
  async listen(port = DEFAULT_PORT, host?: string): Promise<number> {
    await this.close();
    const hosts = host ? [host] : ['::', '0.0.0.0'];
    let failure: unknown;
    for (const h of hosts) {
      try {
        this.port = await this.bind(port, h);
        if (this.tickMs > 0) {
          this.timer = setInterval(() => this.tick(), this.tickMs);
          this.timer.unref();
        }
        return this.port;
      } catch (error) {
        failure = error;
      }
    }
    throw failure;
  }

  private bind(port: number, host: string): Promise<number> {
    const server = createServer((_, response) => response.writeHead(404).end());
    const wss = new WebSocketServer({ noServer: true, maxPayload: NOISE_MAX });
    server.on('connection', (socket) => this.admit(socket));
    server.on('upgrade', (request, socket, head) => {
      clearTimeout(this.upgradeTimers.get(socket));
      const path = new URL(request.url ?? '/', 'http://pc').pathname;
      if (path !== '/v1' && path !== '/v1/pair') {
        socket.end('HTTP/1.1 404 Not Found\r\nConnection: close\r\nContent-Length: 0\r\n\r\n');
        return;
      }
      wss.handleUpgrade(request, socket, head, (ws) => void this.connection(ws, path === '/v1/pair'));
    });
    return new Promise((resolve, reject) => {
      server.once('error', reject);
      server.listen(port, host, () => {
        server.off('error', reject);
        this.server = server;
        this.wss = wss;
        resolve((server.address() as { port: number }).port);
      });
    });
  }

  /** Stops listening and closes every connection. */
  async close(): Promise<void> {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
    this.endPairing();
    const { server, wss } = this;
    this.server = this.wss = null;
    this.port = 0;
    this.sessions.clear();
    if (!server || !wss) return;
    for (const ws of wss.clients) this.closeSocket(ws, 'off');
    wss.close();
    await new Promise<void>((resolve) => server.close(() => resolve()));
  }

  /** TCP level, before the WebSocket handshake: a source outside the local network, a sixth try within a minute
   * or a step that takes over 10 s just closes. */
  private admit(socket: Socket): void {
    const ip = socket.remoteAddress;
    if (!ip || !allowedSource(ip) || !this.rateOk(ip)) {
      socket.destroy();
      return;
    }
    socket.setNoDelay(true);
    const timer = setTimeout(() => socket.destroy(), HANDSHAKE_TIMEOUT_MS);
    timer.unref();
    this.upgradeTimers.set(socket, timer);
    socket.once('close', () => clearTimeout(timer));
  }

  private rateOk(ip: string): boolean {
    const now = Date.now();
    for (const [address, times] of this.attempts) {
      const recent = times.filter((t) => now - t < MINUTE_MS);
      if (recent.length > 0) this.attempts.set(address, recent);
      else this.attempts.delete(address);
    }
    const times = this.attempts.get(ip) ?? [];
    if (times.length >= this.perMinute) return false;
    this.attempts.set(ip, [...times, now]);
    return true;
  }

  private async connection(ws: WebSocket, pair: boolean): Promise<void> {
    const inbox = new Inbox(ws);
    let why: Why = 'gone';
    try {
      const done = pair ? await this.pair(ws, inbox) : await this.knock(ws, inbox);
      await this.session(ws, inbox, done);
    } catch (error) {
      why = error instanceof Refuse ? error.why : 'invalid';
      // Logged without data: no address, key or name.
      if (why !== 'gone' && why !== 'timeout') this.log(`${pair ? 'pairing' : 'session'} closed (${why})`);
    }
    this.closeSocket(ws, why);
  }

  /** `/v1/pair`: `IKpsk2`, the phone's proof, then the user's answer. */
  private async pair(ws: WebSocket, inbox: Inbox): Promise<Done> {
    const first = await inbox.read(HANDSHAKE_TIMEOUT_MS);
    const pairing = this.pairing;
    if (!pairing) throw new Refuse('expired');
    // One try per secret: it is gone from here on, whatever happens next.
    const secret = pairing.secret;
    if (!secret) throw new Refuse('expired');
    pairing.secret = null;
    if (this.devices.length >= MAX_DEVICES) throw new Refuse('full');
    const hs = createHandshake({
      pattern: 'IKpsk2',
      initiator: false,
      staticKey: this.keys,
      prologue: PAIR_PROLOGUE,
      psk: secret,
      random,
    });
    const payload = attempt(() => hs.readMessage(first));
    const hello = parseJson(payload);
    const name = hello && Object.keys(hello).length === 1 ? validName(hello.name) : null;
    if (!name) throw new Refuse('invalid');
    const key = bytesToHex(hs.remoteStaticKey!);
    ws.send(hs.writeMessage());
    const code = pairingCode(hs.handshakeHash());
    const transport = hs.split();
    // The PSK is only mixed in by the PC's reply: the phone's first transport message is what proves it had
    // the secret, so only then is the user asked. Like the real PC, it waits for the proof until the window ends.
    const frame = await inbox.read(Math.max(0, pairing.until - Date.now()));
    const proof = parseJson(attempt(() => transport.receive.decrypt(frame)));
    if (!proof || Object.keys(proof).length !== 1 || proof.t !== 'pair') throw new Refuse('invalid');
    if (this.pairing !== pairing) throw new Refuse('expired');
    const answer = await new Promise<'allow' | 'deny' | 'expired'>((settle) => {
      pairing.asking = { name, code, settle };
      this.onAsk?.({ name, code });
    });
    // The window ends with its one pairing, whatever the answer.
    if (this.pairing === pairing) this.endPairing();
    if (answer === 'deny') throw new Refuse('denied');
    if (answer === 'expired') throw new Refuse('expired');
    this.devices = [...this.devices.filter((d) => d.key !== key), { name, key, paired: Math.floor(this.clock() / 1000) }];
    this.save();
    return { transport, key, ping: null };
  }

  /** `/v1`: `KK` against each paired key (the PC cannot tell which phone knocks; only its key reads the first
   * message). */
  private async knock(ws: WebSocket, inbox: Inbox): Promise<Done> {
    const first = await inbox.read(HANDSHAKE_TIMEOUT_MS);
    for (const device of this.devices) {
      const hs = createHandshake({
        pattern: 'KK',
        initiator: false,
        staticKey: this.keys,
        remoteStatic: hexToBytes(device.key),
        prologue: KK_PROLOGUE,
        random,
      });
      try {
        hs.readMessage(first);
      } catch {
        continue;
      }
      ws.send(hs.writeMessage());
      const transport = hs.split();
      // A replayed first message gets this far; only the phone itself can send a ping now, and nothing is sent
      // or counted before it.
      const frame = await inbox.read(HANDSHAKE_TIMEOUT_MS);
      const incoming = parseIncoming(attempt(() => transport.receive.decrypt(frame)));
      if (incoming?.t !== 'ping') throw new Refuse('invalid');
      return { transport, key: device.key, ping: incoming.n };
    }
    throw new Refuse('unknown');
  }

  /** Joins the sessions and sends `hello`, `state` and `queue`, in that order, ahead of any broadcast. */
  private async session(ws: WebSocket, inbox: Inbox, done: Done): Promise<void> {
    const { transport, key, ping } = done;
    if (!this.server) throw new Refuse('off');
    if (!this.devices.some((d) => d.key === key)) throw new Refuse('removed');
    // The same phone again (it reconnected before its old socket timed out): the new session wins.
    for (const old of this.sessions) {
      if (old.key !== key) continue;
      this.sessions.delete(old);
      old.ws.terminate();
    }
    if (this.sessions.size >= MAX_SESSIONS) throw new Refuse('busy');
    const send = (message: object) => {
      const text = JSON.stringify(message);
      if (text.length > MAX_OUT) return;
      ws.send(transport.send.encrypt(new TextEncoder().encode(text)));
    };
    const session: Session = { key, ws, send };
    send({ t: 'hello', v: VERSION, min: MIN_VERSION, name: this.name });
    send(this.playback.stateMessage());
    send(this.playback.queueMessage());
    this.sessions.add(session);
    this.lastState = this.clock();
    if (ping !== null) send({ t: 'pong', n: ping, at: this.clock() });
    this.log('session open');
    try {
      for (;;) {
        const frame = await inbox.read(SILENCE_MS);
        // A replayed, reordered or forged message fails here (Noise nonces and tags).
        const reply = this.handle(attempt(() => transport.receive.decrypt(frame)));
        if (reply) send(reply);
      }
    } finally {
      this.sessions.delete(session);
    }
  }

  /** The reply to one controller message, if it has one; the commands are applied here. */
  private handle(plain: Uint8Array): object | null {
    const incoming = parseIncoming(plain);
    if (!incoming) return error('invalid');
    if (incoming.t === 'ping') return { t: 'pong', n: incoming.n, at: this.clock() };
    const { cmd } = incoming;
    // Pausing or resuming an ad shortens nothing; everything else waits for it to end.
    if (this.playback.ad && cmd.c !== 'play' && cmd.c !== 'pause') return error('ad');
    const p = this.playback;
    switch (cmd.c) {
      case 'play':
        p.play();
        break;
      case 'pause':
        p.pause();
        break;
      case 'next':
        p.next();
        break;
      case 'previous':
        p.previous();
        break;
      case 'seek':
        p.seek(cmd.s);
        break;
      case 'volume':
        p.setVolume(cmd.v);
        break;
      case 'play_item':
        p.playItem(cmd.id, cmd.i);
        break;
    }
    // A command that worked gets no reply; the next state shows it.
    this.broadcastState();
    if (cmd.c === 'next' || cmd.c === 'previous' || cmd.c === 'play_item') this.broadcastQueue();
    return null;
  }

  private broadcast(message: object): void {
    for (const session of this.sessions) session.send(message);
  }

  private broadcastState(): void {
    this.lastState = this.clock();
    this.broadcast(this.playback.stateMessage());
  }

  private broadcastQueue(): void {
    this.broadcast(this.playback.queueMessage());
  }

  /** Closes the matching sessions; they leave the list at once. */
  private kick(which: (session: Session) => boolean, why: Why): void {
    for (const session of [...this.sessions]) {
      if (!which(session)) continue;
      this.sessions.delete(session);
      this.closeSocket(session.ws, why);
    }
  }

  /** Sends the close frame (when there is one to send) and drops the socket if the phone does not answer. */
  private closeSocket(ws: WebSocket, why: Why): void {
    if (this.closing.has(ws)) return;
    this.closing.add(ws);
    if (why === 'gone') {
      ws.terminate();
      return;
    }
    ws.close(CODES[why], why);
    setTimeout(() => ws.terminate(), 2000).unref();
  }

  private load(): Stored | null {
    if (!this.storePath || !existsSync(this.storePath)) return null;
    try {
      const stored = JSON.parse(readFileSync(this.storePath, 'utf8')) as Stored;
      const keyOk = isKey(stored.key?.private) && isKey(stored.key?.public);
      const devices = (stored.devices ?? []).filter((d) => isKey(d.key) && validName(d.name) !== null).slice(0, MAX_DEVICES);
      return keyOk ? { key: stored.key, devices } : null;
    } catch {
      this.log(`${this.storePath} is invalid; starting again`);
      return null;
    }
  }

  private save(): void {
    if (!this.storePath) return;
    const stored: Stored = {
      key: { private: bytesToHex(this.keys.privateKey), public: bytesToHex(this.keys.publicKey) },
      devices: this.devices,
    };
    writeFileSync(this.storePath, JSON.stringify(stored, null, 2));
  }
}

interface Done {
  transport: Transport;
  key: string;
  /** The `n` of the ping that opened a `KK` session (answered with the first pong); none after pairing. */
  ping: number | null;
}

/** The binary frames of one socket, one at a time; a text frame or an oversized one is a refusal to read. */
class Inbox {
  private items: (Buffer | Refuse)[] = [];
  private waiter: { resolve: (frame: Buffer) => void; reject: (why: Refuse) => void } | null = null;
  private ended = false;

  constructor(ws: WebSocket) {
    ws.on('message', (data: RawData, isBinary: boolean) => {
      const frame = Array.isArray(data) ? Buffer.concat(data) : Buffer.from(data as Buffer);
      this.push(!isBinary ? new Refuse('invalid') : frame.length > MAX_FRAME ? new Refuse('too_big') : frame);
    });
    ws.on('close', () => this.end());
    ws.on('error', () => this.end());
  }

  private push(item: Buffer | Refuse): void {
    const waiter = this.waiter;
    if (!waiter) {
      this.items.push(item);
      return;
    }
    this.waiter = null;
    if (item instanceof Refuse) waiter.reject(item);
    else waiter.resolve(item);
  }

  private end(): void {
    this.ended = true;
    this.waiter?.reject(new Refuse('gone'));
    this.waiter = null;
  }

  read(timeoutMs: number): Promise<Buffer> {
    const item = this.items.shift();
    if (item) return item instanceof Refuse ? Promise.reject(item) : Promise.resolve(item);
    if (this.ended) return Promise.reject(new Refuse('gone'));
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.waiter = null;
        reject(new Refuse('timeout'));
      }, timeoutMs);
      this.waiter = {
        resolve: (frame) => (clearTimeout(timer), resolve(frame)),
        reject: (why) => (clearTimeout(timer), reject(why)),
      };
    });
  }
}

/** Noise or parsing failures are the phone's doing: `invalid`. */
function attempt<T>(run: () => T): T {
  try {
    return run();
  } catch {
    throw new Refuse('invalid');
  }
}

function error(code: 'ad' | 'unavailable' | 'invalid') {
  return { t: 'error', code };
}

function parseJson(bytes: Uint8Array): Record<string, unknown> | null {
  try {
    const value: unknown = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
    return value !== null && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

const num = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);
const inRange = (v: unknown, max: number): v is number => num(v) && v >= 0 && v <= max;
const hasOnly = (o: Record<string, unknown>, ...keys: string[]) =>
  Object.keys(o).length === keys.length && keys.every((k) => k in o);

/** A controller message in its fixed shape: unknown fields, wrong types and out-of-range values are refused. */
export function parseIncoming(bytes: Uint8Array): Incoming | null {
  const m = parseJson(bytes);
  if (!m) return null;
  if (m.t === 'ping') return hasOnly(m, 't', 'n') && Number.isSafeInteger(m.n) && (m.n as number) >= 0 ? { t: 'ping', n: m.n as number } : null;
  if (m.t !== 'cmd') return null;
  switch (m.c) {
    case 'play':
    case 'pause':
    case 'next':
    case 'previous':
      return hasOnly(m, 't', 'c') ? { t: 'cmd', cmd: { c: m.c } } : null;
    case 'seek':
      return hasOnly(m, 't', 'c', 's') && inRange(m.s, MAX_SECONDS) ? { t: 'cmd', cmd: { c: 'seek', s: m.s } } : null;
    case 'volume':
      return hasOnly(m, 't', 'c', 'v') && inRange(m.v, 100) ? { t: 'cmd', cmd: { c: 'volume', v: m.v } } : null;
    case 'play_item':
      return hasOnly(m, 't', 'c', 'id', 'i') &&
        typeof m.id === 'string' &&
        /^[A-Za-z0-9_-]{11}$/.test(m.id) &&
        Number.isInteger(m.i) &&
        (m.i as number) >= 0 &&
        (m.i as number) < MAX_QUEUE
        ? { t: 'cmd', cmd: { c: 'play_item', id: m.id, i: m.i as number } }
        : null;
    default:
      return null;
  }
}

/** 1 to 32 characters once trimmed, no control characters. */
export function validName(name: unknown): string | null {
  if (typeof name !== 'string') return null;
  const trimmed = name.trim();
  const count = [...trimmed].length;
  return count >= 1 && count <= MAX_NAME && !/[\u0000-\u001f\u007f-\u009f]/.test(trimmed) ? trimmed : null;
}

/** The 6 digits both screens show: the first 4 bytes of the handshake hash, big-endian, modulo 1 000 000. */
export function pairingCode(hash: Uint8Array): string {
  const n = new DataView(hash.buffer, hash.byteOffset, 4).getUint32(0, false);
  return String(n % 1_000_000).padStart(6, '0');
}

export function pairingUri(addresses: string[], port: number, key: Uint8Array, secret: Uint8Array): string {
  return `feathertune://pair?v=${VERSION}&k=${bytesToHex(key)}&s=${bytesToHex(secret)}&a=${addresses.join(',')}&p=${port}`;
}

const isKey = (text: unknown): text is string => typeof text === 'string' && /^[0-9a-f]{64}$/.test(text);

/** Private, link-local, loopback and CGNAT (mesh VPNs) sources; IPv4-mapped IPv6 counts as IPv4. */
export function allowedSource(address: string): boolean {
  const ip = address.split('%')[0].toLowerCase();
  const mapped = /^::ffff:(\d+\.\d+\.\d+\.\d+)$/i.exec(ip);
  if (mapped) return allowedSource(mapped[1]);
  const v4 = /^(\d+)\.(\d+)\.\d+\.\d+$/.exec(ip);
  if (v4) {
    const [a, b] = [Number(v4[1]), Number(v4[2])];
    return (
      a === 10 || (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168) || (a === 169 && b === 254) || a === 127 || (a === 100 && b >= 64 && b <= 127)
    );
  }
  if (ip === '::1') return true;
  const first = /^([0-9a-f]{1,4}):/.exec(ip);
  if (!first) return false;
  const segment = parseInt(first[1], 16);
  return (segment & 0xfe00) === 0xfc00 || (segment & 0xffc0) === 0xfe80;
}

/** The address of the interface the default route uses, found without sending anything (connecting a UDP
 * socket only picks a route). */
export function lanAddresses(): Promise<string[]> {
  return new Promise((resolve) => {
    const socket = createSocket('udp4');
    const done = (found: string[]) => {
      try {
        socket.close();
      } catch {
        // already closed
      }
      resolve(found);
    };
    socket.once('error', () => done([]));
    socket.connect(9, '192.0.2.1', () => {
      const { address } = socket.address();
      done(allowedSource(address) && !address.startsWith('127.') ? [address] : []);
    });
  });
}
