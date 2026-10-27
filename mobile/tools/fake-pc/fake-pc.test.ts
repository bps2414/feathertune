// The fake PC against a phone's side of the protocol (docs/REMOTE.md, Protocol), over a real socket: pairing, KK
// sessions, the commands, the ad mode and the refusals. The phone side is built here on src/noise, as the app's is.

import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { bytesToHex, hexToBytes } from '@noble/hashes/utils.js';
import { WebSocket } from 'ws';

import { createHandshake, generateKeyPair, type KeyPair, type Transport } from '../../src/noise';
import { FakePc, allowedSource, type FakePcOptions } from './server';

const random = (length: number) => crypto.getRandomValues(new Uint8Array(length));
const utf8 = (text: string) => new TextEncoder().encode(text);
const PAIR_PROLOGUE = utf8('feathertune-remote/1 pair');
const KK_PROLOGUE = utf8('feathertune-remote/1');

type Json = Record<string, any>;
type Closed = { closed: number };

/** A client socket: its binary frames and its close, one at a time. */
class Link {
  private items: (Buffer | Closed)[] = [];
  private waiter: ((item: Buffer | Closed) => void) | null = null;

  constructor(readonly ws: WebSocket) {
    ws.on('message', (data, isBinary) => isBinary && this.push(Buffer.from(data as Buffer)));
    ws.on('close', (code) => this.push({ closed: code }));
    ws.on('error', () => {});
  }

  private push(item: Buffer | Closed): void {
    if (this.waiter) {
      const waiter = this.waiter;
      this.waiter = null;
      waiter(item);
    } else {
      this.items.push(item);
    }
  }

  /** The next frame or close; null when nothing comes within `ms`. */
  next(ms = 5000): Promise<Buffer | Closed | null> {
    const item = this.items.shift();
    if (item) return Promise.resolve(item);
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        this.waiter = null;
        resolve(null);
      }, ms);
      this.waiter = (found) => (clearTimeout(timer), resolve(found));
    });
  }

  send(frame: Uint8Array | string): void {
    this.ws.send(frame);
  }
}

/** A phone's side of a session. */
class Phone {
  constructor(
    readonly link: Link,
    private readonly transport: Transport,
    readonly code = ''
  ) {}

  encrypt(message: object | string): Uint8Array {
    return this.transport.send.encrypt(utf8(typeof message === 'string' ? message : JSON.stringify(message)));
  }

  send(message: object | string): void {
    this.link.send(this.encrypt(message));
  }

  /** The next message from the PC; throws when the PC closed (with the code) or went quiet. */
  async recv(ms = 5000): Promise<Json> {
    const item = await this.link.next(ms);
    if (item === null) throw new Error('nothing came');
    if ('closed' in item) throw new Error(`closed ${item.closed}`);
    return JSON.parse(new TextDecoder().decode(this.transport.receive.decrypt(item)));
  }

  /** The close code (1006: dropped without a close frame), skipping what the PC still sends. */
  async closed(): Promise<number> {
    for (;;) {
      const item = await this.link.next();
      if (item === null) throw new Error('not closed');
      if ('closed' in item) return item.closed;
    }
  }
}

const servers: FakePc[] = [];
const links: Link[] = [];
const clock = { now: 1_700_000_000_000 };
let dir: string;

beforeEach(() => {
  clock.now = 1_700_000_000_000;
  dir = mkdtempSync(join(tmpdir(), 'fake-pc-test-'));
});

afterEach(async () => {
  for (const link of links.splice(0)) link.ws.terminate();
  for (const pc of servers.splice(0)) await pc.close();
  rmSync(dir, { recursive: true, force: true });
});

async function start(options: FakePcOptions = {}): Promise<{ pc: FakePc; port: number }> {
  const pc = new FakePc({
    addresses: ['127.0.0.1'],
    storePath: join(dir, 'fake-pc.json'),
    tickMs: 0,
    perMinute: Infinity,
    clock: () => clock.now,
    ...options,
  });
  servers.push(pc);
  return { pc, port: await pc.listen(0, '127.0.0.1') };
}

function open(port: number, path: string): Promise<Link> {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(`ws://127.0.0.1:${port}${path}`);
    const link = new Link(ws);
    links.push(link);
    ws.once('open', () => resolve(link));
    ws.once('error', reject);
  });
}

function parseUri(uri: string) {
  const url = new URL(uri);
  const get = (name: string) => url.searchParams.get(name)!;
  return { scheme: url.protocol, host: url.host, v: get('v'), k: hexToBytes(get('k')), s: hexToBytes(get('s')), a: get('a'), p: get('p') };
}

/** `IKpsk2` as the phone runs it, up to its proof; the PC then asks the user. A refusal is a close code. */
async function pair(
  port: number,
  uri: string,
  keys: KeyPair,
  name: string,
  secret?: Uint8Array
): Promise<Phone | number> {
  const q = parseUri(uri);
  const link = await open(port, '/v1/pair');
  const hs = createHandshake({
    pattern: 'IKpsk2',
    initiator: true,
    staticKey: keys,
    remoteStatic: q.k,
    prologue: PAIR_PROLOGUE,
    psk: secret ?? q.s,
    random,
  });
  link.send(hs.writeMessage(utf8(JSON.stringify({ name }))));
  const reply = await link.next();
  if (!reply || 'closed' in reply) return reply ? reply.closed : -1;
  hs.readMessage(new Uint8Array(reply));
  const hash = hs.handshakeHash();
  const code = String(new DataView(hash.buffer, hash.byteOffset, 4).getUint32(0) % 1_000_000).padStart(6, '0');
  const phone = new Phone(link, hs.split(), code);
  phone.send({ t: 'pair' });
  return phone;
}

/** `KK` as the phone runs it, then its first ping (unless `ping` is false). */
async function knock(port: number, pcKey: string, keys: KeyPair, ping = true): Promise<Phone | number> {
  const link = await open(port, '/v1');
  const hs = createHandshake({
    pattern: 'KK',
    initiator: true,
    staticKey: keys,
    remoteStatic: hexToBytes(pcKey),
    prologue: KK_PROLOGUE,
    random,
  });
  link.send(hs.writeMessage());
  const reply = await link.next();
  if (!reply || 'closed' in reply) return reply ? reply.closed : -1;
  hs.readMessage(new Uint8Array(reply));
  const phone = new Phone(link, hs.split());
  if (ping) phone.send({ t: 'ping', n: 0 });
  return phone;
}

async function asked(pc: FakePc) {
  for (let i = 0; i < 250; i++) {
    const pending = pc.pending();
    if (pending) return pending;
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  throw new Error('no pending request');
}

/** A phone paired and allowed (its pairing session dropped); returns its keys. */
async function paired(pc: FakePc, port: number, name = 'Phone'): Promise<KeyPair> {
  const keys = generateKeyPair(random);
  const phone = await pair(port, await pc.startPairing(), keys, name);
  if (typeof phone === 'number') throw new Error(`pairing closed ${phone}`);
  await asked(pc);
  expect(pc.answer(true)).toBe(true);
  expect((await phone.recv()).t).toBe('hello');
  phone.link.ws.terminate();
  // Its pairing session must be gone before the next connection counts the open ones.
  const key = bytesToHex(keys.publicKey);
  for (let i = 0; i < 100 && pc.getDevices().some((d) => d.key === key && d.connected); i++) {
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  return keys;
}

async function session(port: number, pc: FakePc, keys: KeyPair): Promise<Phone> {
  const phone = await knock(port, pc.publicKey, keys);
  if (typeof phone === 'number') throw new Error(`session closed ${phone}`);
  for (const t of ['hello', 'state', 'queue', 'library', 'pong']) expect((await phone.recv()).t).toBe(t);
  return phone;
}

describe('pairing and a session', () => {
  it('pairs after the user allows, then controls the player over KK', async () => {
    const { pc, port } = await start();
    const uri = await pc.startPairing();
    const q = parseUri(uri);
    expect([q.scheme, q.host, q.v, q.a, q.p]).toEqual(['feathertune:', 'pair', '1', '127.0.0.1', String(port)]);
    expect([q.k.length, q.s.length, bytesToHex(q.k)]).toEqual([32, 32, pc.publicKey]);

    const keys = generateKeyPair(random);
    const phone = (await pair(port, uri, keys, '  Pixel 9  ')) as Phone;
    expect(await asked(pc)).toEqual({ name: 'Pixel 9', code: phone.code });
    expect(phone.code).toMatch(/^\d{6}$/);
    expect(pc.answer(true)).toBe(true);

    expect(await phone.recv()).toEqual({ t: 'hello', v: 2, min: 1, name: 'feathertune' });
    const state = await phone.recv();
    expect(state).toMatchObject({ t: 'state', paused: false, ad: false, volume: 70, track: { id: 'fakeTrk0001', artwork: null } });
    expect(state.colors).toHaveLength(3);
    const queue = await phone.recv();
    expect([queue.t, queue.items.length, queue.current]).toEqual(['queue', 8, 0]);
    expect(await phone.recv()).toEqual({ t: 'library', items: [{ list: 'LM', title: 'Liked music' }, { list: 'PLfakeRoadTrip01', title: 'Road trip' }, { list: 'PLfakeQuietHours', title: 'Quiet hours' }] });
    expect(Object.keys(queue.items[0])).toEqual(['id', 'title', 'artist']);
    expect(pc.getDevices()).toEqual([{ key: bytesToHex(keys.publicKey), name: 'Pixel 9', paired: 1_700_000_000, connected: true }]);
    expect(JSON.parse(readFileSync(join(dir, 'fake-pc.json'), 'utf8')).devices[0].name).toBe('Pixel 9');
    expect(pc.pending()).toBeNull();
    phone.link.ws.terminate();

    // Every later connection is KK, and nothing is sent before the phone's first ping.
    const quiet = (await knock(port, pc.publicKey, keys, false)) as Phone;
    expect(await quiet.link.next(300)).toBeNull();
    quiet.send({ t: 'ping', n: 41 });
    expect((await quiet.recv()).t).toBe('hello');
    expect((await quiet.recv()).t).toBe('state');
    expect((await quiet.recv()).t).toBe('queue');
    expect((await quiet.recv()).t).toBe('library');
    expect(await quiet.recv()).toMatchObject({ t: 'pong', n: 41, at: clock.now });
    quiet.link.ws.terminate();

    const p = await session(port, pc, keys);
    p.send({ t: 'cmd', c: 'pause' });
    expect(await p.recv()).toMatchObject({ t: 'state', paused: true });
    p.send({ t: 'cmd', c: 'play' });
    expect(await p.recv()).toMatchObject({ paused: false });
    p.send({ t: 'cmd', c: 'seek', s: 12.5 });
    expect(await p.recv()).toMatchObject({ position: 12.5 });
    p.send({ t: 'cmd', c: 'volume', v: 33 });
    expect(await p.recv()).toMatchObject({ volume: 33 });
    p.send({ t: 'cmd', c: 'next' });
    expect(await p.recv()).toMatchObject({ track: { id: 'fakeTrk0002' }, position: 0, colors: [] });
    expect(await p.recv()).toMatchObject({ t: 'queue', current: 1 });
    p.send({ t: 'cmd', c: 'play_item', id: 'fakeTrk0005', i: 4 });
    expect(await p.recv()).toMatchObject({ track: { id: 'fakeTrk0005' } });
    expect(await p.recv()).toMatchObject({ t: 'queue', current: 4 });
    p.send({ t: 'cmd', c: 'previous' });
    expect(await p.recv()).toMatchObject({ track: { id: 'fakeTrk0004' } });
    expect(await p.recv()).toMatchObject({ t: 'queue', current: 3 });
    p.send({ t: 'ping', n: 7 });
    expect(await p.recv()).toMatchObject({ t: 'pong', n: 7 });

    // The colors of a new track arrive later; a track that ends is followed by the next.
    clock.now += 1000;
    pc.tick();
    expect((await p.recv()).colors).toHaveLength(3);
    clock.now += 300_000;
    pc.tick();
    expect(await p.recv()).toMatchObject({ track: { id: 'fakeTrk0005' }, position: 0 });
    expect(await p.recv()).toMatchObject({ t: 'queue', current: 4 });
  });

  it('keeps the paired phones and the PC key across restarts', async () => {
    const first = await start();
    const keys = await paired(first.pc, first.port);
    const key = first.pc.publicKey;
    await first.pc.close();
    const second = await start();
    expect(second.pc.publicKey).toBe(key);
    expect(second.pc.getDevices().map((d) => d.key)).toEqual([bytesToHex(keys.publicKey)]);
    await session(second.port, second.pc, keys);
  });
});

describe('refusals', () => {
  it('closes a key the PC does not have, without a word', async () => {
    const { pc, port } = await start();
    await paired(pc, port);
    expect(await knock(port, pc.publicKey, generateKeyPair(random))).toBe(4002);
  });

  it('uses a secret once, and only while the window is open', async () => {
    const { pc, port } = await start();
    // No window at all.
    const stray = parseUri(await pc.startPairing());
    pc.endPairing();
    expect(await pair(port, `feathertune://pair?v=1&k=${pc.publicKey}&s=${bytesToHex(stray.s)}`, generateKeyPair(random), 'Late')).toBe(4001);
    // A second phone with the secret already taken, while the first waits for the answer and after it.
    const uri = await pc.startPairing();
    const first = (await pair(port, uri, generateKeyPair(random), 'First')) as Phone;
    await asked(pc);
    expect(await pair(port, uri, generateKeyPair(random), 'Copy')).toBe(4001);
    expect(pc.answer(true)).toBe(true);
    expect((await first.recv()).t).toBe('hello');
    expect(await pair(port, uri, generateKeyPair(random), 'Copy')).toBe(4001);
    expect(pc.getDevices()).toHaveLength(1);
  });

  it('answers a wrong secret with a reply the phone cannot read, and burns the window', async () => {
    const { pc, port } = await start();
    const uri = await pc.startPairing();
    await expect(pair(port, uri, generateKeyPair(random), 'Guess', random(32))).rejects.toThrow();
    expect(await pair(port, uri, generateKeyPair(random), 'Real')).toBe(4001);
    expect(pc.getDevices()).toHaveLength(0);
  });

  it('closes junk with invalid and leaves the window open', async () => {
    const { pc, port } = await start();
    const uri = await pc.startPairing();
    // Random bytes, and a first message made for another PC's key: neither reads, neither spends the secret.
    const junk = await open(port, '/v1/pair');
    junk.send(random(96));
    expect(await junk.next()).toEqual({ closed: 4006 });
    const other = generateKeyPair(random).publicKey;
    expect(await pair(port, uri.replace(pc.publicKey, bytesToHex(other)), generateKeyPair(random), 'Stray')).toBe(4006);
    const phone = (await pair(port, uri, generateKeyPair(random), 'Real')) as Phone;
    await asked(pc);
    expect(pc.answer(true)).toBe(true);
    expect((await phone.recv()).t).toBe('hello');
  });

  it('closes a denied phone, a window that ends while it waits and a bad name', async () => {
    const { pc, port } = await start();
    const denied = (await pair(port, await pc.startPairing(), generateKeyPair(random), 'Pixel')) as Phone;
    await asked(pc);
    expect(pc.answer(false)).toBe(true);
    expect(await denied.closed()).toBe(4000);
    expect(pc.answer(true)).toBe(false);

    const waiting = (await pair(port, await pc.startPairing(), generateKeyPair(random), 'Pixel')) as Phone;
    await asked(pc);
    pc.endPairing();
    expect(await waiting.closed()).toBe(4001);

    for (const name of ['x'.repeat(33), '   ', 'a\u0007b']) {
      expect(await pair(port, await pc.startPairing(), generateKeyPair(random), name).then((p) => (typeof p === 'number' ? p : p.closed()))).toBe(4006);
    }
    expect(pc.getDevices()).toHaveLength(0);
  });

  it('refuses malformed commands and closes on an oversized or text frame', async () => {
    const { pc, port } = await start();
    const keys = await paired(pc, port);
    const phone = await session(port, pc, keys);
    for (const bad of [
      'not json',
      '[]',
      '{"t":"bogus"}',
      '{"t":"cmd","c":"jump"}',
      '{"t":"cmd","c":"next","x":1}',
      '{"t":"cmd","c":"seek","s":"x"}',
      '{"t":"cmd","c":"seek","s":-1}',
      '{"t":"cmd","c":"seek","s":1e9}',
      '{"t":"cmd","c":"volume","v":101}',
      '{"t":"cmd","c":"play_item","id":"bad","i":0}',
      '{"t":"cmd","c":"play_item","id":"fakeTrk0001","i":200}',
      '{"t":"cmd","c":"play_item","id":"fakeTrk0001"}',
      '{"t":"ping"}',
      '{"t":"ping","n":-1}',
      '{"t":"ping","n":1,"c":"next"}',
    ]) {
      phone.send(bad);
      expect(await phone.recv()).toEqual({ t: 'error', code: 'invalid' });
    }
    expect(pc.playback).toMatchObject({ index: 0, volume: 70, paused: false });
    phone.send({ t: 'ping', n: 1, pad: 'x'.repeat(1024) });
    expect(await phone.closed()).toBe(4007);

    const second = await session(port, pc, keys);
    second.link.send('{"t":"ping","n":1}');
    expect(await second.closed()).toBe(4006);
  });

  it('closes a replayed message, runs it once, and sends nothing to a replayed handshake', async () => {
    const { pc, port } = await start();
    const keys = await paired(pc, port);
    const phone = await session(port, pc, keys);
    const frame = phone.encrypt({ t: 'cmd', c: 'next' });
    phone.link.send(frame);
    phone.link.send(frame);
    expect(await phone.closed()).toBe(4006);
    expect(pc.playback.index).toBe(1);

    // A copy of the phone's first handshake message: the PC cannot tell and answers it, but the copy cannot
    // send the ping, so nothing else comes before the close.
    const hs = createHandshake({
      pattern: 'KK',
      initiator: true,
      staticKey: keys,
      remoteStatic: hexToBytes(pc.publicKey),
      prologue: KK_PROLOGUE,
      random,
    });
    const first = hs.writeMessage();
    for (const copy of [0, 1]) {
      const link = await open(port, '/v1');
      link.send(first);
      expect(await link.next()).toBeInstanceOf(Buffer);
      if (copy === 1) link.send(random(40));
      else link.send(first);
      expect(await link.next()).toEqual({ closed: 4006 });
    }
    expect(pc.playback.index).toBe(1);
  });

  it('refuses everything but play and pause during an ad', async () => {
    const { pc, port } = await start();
    const phone = await session(port, pc, await paired(pc, port));
    pc.setAd(true);
    expect(await phone.recv()).toMatchObject({ t: 'state', ad: true });
    for (const c of ['"c":"next"', '"c":"previous"', '"c":"seek","s":5', '"c":"volume","v":10', '"c":"play_item","id":"fakeTrk0003","i":2']) {
      phone.send(`{"t":"cmd",${c}}`);
      expect(await phone.recv()).toEqual({ t: 'error', code: 'ad' });
    }
    // Malformed is still malformed, and a ping still works.
    phone.send('{"t":"cmd","c":"seek","s":-5}');
    expect(await phone.recv()).toEqual({ t: 'error', code: 'invalid' });
    phone.send({ t: 'cmd', c: 'pause' });
    expect(await phone.recv()).toMatchObject({ t: 'state', paused: true, ad: true });
    phone.send({ t: 'ping', n: 2 });
    expect(await phone.recv()).toMatchObject({ t: 'pong', n: 2 });
    expect(pc.playback).toMatchObject({ index: 0, volume: 70 });

    pc.setAd(false);
    expect(await phone.recv()).toMatchObject({ ad: false });
    phone.send({ t: 'cmd', c: 'next' });
    expect(await phone.recv()).toMatchObject({ track: { id: 'fakeTrk0002' } });
  });

  it('closes a removed phone at once, and every key after Forget all', async () => {
    const { pc, port } = await start();
    const keys = await paired(pc, port);
    const phone = await session(port, pc, keys);
    expect(pc.remove(bytesToHex(keys.publicKey))).toBe(true);
    expect(await phone.closed()).toBe(4003);
    expect(pc.getDevices()).toEqual([]);
    expect(await knock(port, pc.publicKey, keys)).toBe(4002);

    const other = await paired(pc, port);
    const old = pc.publicKey;
    const connected = await session(port, pc, other);
    pc.forgetAll();
    expect(await connected.closed()).toBe(4003);
    expect(pc.publicKey).not.toBe(old);
    expect(await knock(port, pc.publicKey, other)).toBe(4002);
  });

  it('lets the newest session of a phone win, and at most 4 at once', async () => {
    const { pc, port } = await start();
    const keys = await paired(pc, port);
    const older = await session(port, pc, keys);
    const newer = await session(port, pc, keys);
    expect(await older.closed()).toBe(1006);
    newer.send({ t: 'ping', n: 3 });
    expect((await newer.recv()).t).toBe('pong');

    const extra: KeyPair[] = [];
    for (let i = 0; i < 4; i++) extra.push(await paired(pc, port, `Phone ${i}`));
    const others = [newer];
    for (const k of extra.slice(0, 3)) others.push(await session(port, pc, k));
    const fifth = (await knock(port, pc.publicKey, extra[3])) as Phone;
    expect(await fifth.closed()).toBe(4004);
    expect(pc.getDevices().filter((d) => d.connected)).toHaveLength(4);
    // Everyone connected gets every state.
    others[0].send({ t: 'cmd', c: 'volume', v: 11 });
    for (const phone of others) expect(await phone.recv()).toMatchObject({ t: 'state', volume: 11 });
  });

  it('answers 404 to any other path, and drops the sixth connection of an address within a minute', async () => {
    const { pc, port } = await start({ perMinute: 5 });
    const status = await new Promise<number>((resolve) => {
      const ws = new WebSocket(`ws://127.0.0.1:${port}/v2`);
      ws.on('unexpected-response', (_, response) => resolve(response.statusCode ?? 0));
      ws.on('error', () => {});
    });
    expect(status).toBe(404);
    for (let i = 0; i < 4; i++) await open(port, '/v1');
    await expect(open(port, '/v1')).rejects.toThrow();
    expect(pc.getDevices()).toEqual([]);
  });
});

describe('the local network rule', () => {
  it('accepts private, link-local, loopback and CGNAT sources only', () => {
    for (const ok of ['192.168.1.20', '10.0.0.5', '172.16.3.4', '172.31.255.1', '169.254.1.1', '127.0.0.1', '100.64.0.1', '100.127.255.254', '::1', 'fd12::1', 'fe80::1', '::ffff:192.168.0.9', 'fe80::1%eth0']) {
      expect(allowedSource(ok)).toBe(true);
    }
    for (const bad of ['8.8.8.8', '172.32.0.1', '100.128.0.1', '100.63.255.255', '11.0.0.1', '2001:db8::1', '2606:4700::1111', '::ffff:8.8.8.8', '0.0.0.0', '::']) {
      expect(allowedSource(bad)).toBe(false);
    }
  });
});
