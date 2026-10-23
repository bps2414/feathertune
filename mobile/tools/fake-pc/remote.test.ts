// The phone's connection layer (src/remote) against the fake PC in process, over real sockets: pairing,
// reconnecting, the background, a PC that removed the phone and a PC of another version.

import { WebSocket } from 'ws';

import type { SocketLike } from '../../src/remote/client';
import { parsePairingUri } from '../../src/remote/protocol';
import { Remote, type RemoteState } from '../../src/remote/remote';
import { memoryStore, type KeyStore } from '../../src/remote/storage';
import { FakePc, type FakePcOptions } from './server';

const random = (length: number) => crypto.getRandomValues(new Uint8Array(length));

const servers: FakePc[] = [];
const remotes: Remote[] = [];

afterEach(async () => {
  for (const remote of remotes.splice(0)) remote.setForeground(false);
  for (const pc of servers.splice(0)) await pc.close();
  await sleep(150);
});

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

async function start(options: FakePcOptions = {}): Promise<{ pc: FakePc; port: number }> {
  const pc = new FakePc({ addresses: ['127.0.0.1'], tickMs: 0, perMinute: Infinity, ...options });
  servers.push(pc);
  return { pc, port: await pc.listen(0, '127.0.0.1') };
}

function phone(storage: KeyStore = memoryStore()): Remote {
  const remote = new Remote({
    storage,
    openSocket: (url) => new WebSocket(url) as unknown as SocketLike,
    random,
    network: async () => ({ lan: true, vpn: false, ip: '127.0.0.1' }),
    timing: { backoffMs: [50, 100], pingMs: 200, backgroundMs: 100, silenceMs: 5000 },
  });
  remotes.push(remote);
  return remote;
}

/** Waits until `check` holds for the remote's state; fails with the state it ended in. */
async function until(remote: Remote, check: (state: RemoteState) => boolean, ms = 8000): Promise<RemoteState> {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const state = remote.store.getState();
    if (check(state)) return state;
    await sleep(10);
  }
  const { status, problem, pairing } = remote.store.getState();
  throw new Error(`timed out: ${JSON.stringify({ status, problem, pairing: { status: pairing.status, problem: pairing.problem } })}`);
}

/** Scans the PC's QR and pairs under `name`; `allow`: the user's answer on the PC (none: no answer yet). */
async function pairWith(remote: Remote, pc: FakePc, allow?: boolean): Promise<void> {
  if (!remote.store.getState().ready) await remote.load();
  remote.setPairingQr(parsePairingUri(await pc.startPairing())!);
  void remote.pair('Pixel 9');
  const { pairing } = await until(remote, (s) => s.pairing.status === 'code');
  // The PC asks once the phone's proof arrives, just after the phone shows the code.
  for (let i = 0; i < 200 && !pc.pending(); i++) await sleep(10);
  expect(pc.pending()).toEqual({ name: 'Pixel 9', code: pairing.code });
  if (allow !== undefined) pc.answer(allow);
}

describe('pairing', () => {
  it('pairs, keeps the PC and connects again after either side restarts', async () => {
    const skew = 60_000;
    // The first address of the QR answers nothing; the second is the PC.
    const { pc, port } = await start({ addresses: ['127.0.0.2', '127.0.0.1'], clock: () => Date.now() + skew });
    const storage = memoryStore();
    const first = phone(storage);
    await pairWith(first, pc, true);
    const state = await until(first, (s) => s.status === 'connected' && s.queue !== null);
    expect(state.pairing.status).toBe('done');
    expect(state.hello).toEqual({ v: 1, min: 1, name: 'feathertune' });
    expect(state.pcs).toEqual([
      expect.objectContaining({ name: 'feathertune', key: pc.publicKey, addresses: ['127.0.0.2', '127.0.0.1'], port, lastAddress: '127.0.0.1' }),
    ]);
    expect(state.activeId).toBe(state.pcs[0].id);
    expect(state.player?.track.id).toBe('fakeTrk0001');
    expect(state.queue?.items).toHaveLength(8);
    expect(pc.getDevices()).toEqual([expect.objectContaining({ name: 'Pixel 9', connected: true })]);
    // The pings every 200 ms here get pongs, which give the PC's clock.
    const { clockOffset } = await until(first, (s) => s.clockOffset > skew - 1000);
    expect(clockOffset).toBeLessThan(skew + 1000);
    first.command({ c: 'pause' });
    await until(first, (s) => s.player?.paused === true);

    // The phone again, from what it stored: KK on the last good address.
    first.setForeground(false);
    await until(first, (s) => s.status === 'idle');
    const second = phone(storage);
    await second.load();
    expect(second.store.getState().pcs).toEqual(state.pcs);
    await until(second, (s) => s.status === 'connected');

    // The PC again: the phone waits, then reconnects on its own.
    await pc.close();
    await until(second, (s) => s.status === 'waiting' && s.problem === 'off');
    await pc.listen(port, '127.0.0.1');
    await until(second, (s) => s.status === 'connected' && s.problem === null);
  });

  it('says why a pairing ended: denied, expired, nothing answering', async () => {
    const { pc } = await start();
    const denied = phone();
    await pairWith(denied, pc, false);
    await until(denied, (s) => s.pairing.status === 'failed' && s.pairing.problem === 'denied');

    const expired = phone();
    await pairWith(expired, pc);
    pc.endPairing();
    await until(expired, (s) => s.pairing.status === 'failed' && s.pairing.problem === 'expired');
    expect(expired.store.getState().pcs).toEqual([]);

    const nobody = phone();
    await nobody.load();
    const qr = parsePairingUri(await pc.startPairing())!;
    nobody.setPairingQr({ ...qr, addresses: ['127.0.0.2'] });
    await nobody.pair('Pixel 9');
    expect(nobody.store.getState().pairing).toMatchObject({ status: 'failed', problem: 'unreachable' });
    // Nothing reached the PC, so the same QR still pairs.
    nobody.setPairingQr(qr);
    void nobody.pair('Pixel 9');
    await until(nobody, (s) => s.pairing.status === 'code');
    pc.answer(true);
    await until(nobody, (s) => s.status === 'connected');
  });
});

describe('the connection', () => {
  it('closes in the background and reconnects in the foreground', async () => {
    const { pc } = await start();
    const remote = phone();
    await pairWith(remote, pc, true);
    await until(remote, (s) => s.status === 'connected');
    remote.setForeground(false);
    // Still connected for a moment (a quick look at another app), then closed.
    expect(remote.store.getState().status).toBe('connected');
    await until(remote, (s) => s.status === 'idle');
    await sleep(50);
    expect(pc.getDevices()[0].connected).toBe(false);
    remote.setForeground(true);
    await until(remote, (s) => s.status === 'connected');
    expect(pc.getDevices()[0].connected).toBe(true);
  });

  it('stops at "removed by the PC", connected or not, and pairs again', async () => {
    const { pc } = await start();
    const storage = memoryStore();
    const remote = phone(storage);
    await pairWith(remote, pc, true);
    await until(remote, (s) => s.status === 'connected');
    expect(pc.remove(JSON.parse(storage.data.get('phone-key')!).public)).toBe(true);
    await until(remote, (s) => s.status === 'failed' && s.problem === 'removed');
    // No more tries.
    await sleep(300);
    expect(remote.store.getState()).toMatchObject({ status: 'failed', problem: 'removed' });

    // Connecting from scratch gets `unknown` from the PC: the same answer.
    remote.setForeground(false);
    const again = phone(storage);
    await again.load();
    await until(again, (s) => s.status === 'failed' && s.problem === 'removed');
    // Pairing again replaces the old entry.
    await pairWith(again, pc, true);
    const state = await until(again, (s) => s.status === 'connected');
    expect(state.pcs).toHaveLength(1);
  });

  it('refuses a PC outside its version range and says which side to update', async () => {
    const newer = await start({ version: { v: 3, min: 2 } });
    const app = phone();
    await pairWith(app, newer.pc, true);
    await until(app, (s) => s.status === 'failed' && s.problem === 'update_app');
    expect(app.store.getState().pcs).toHaveLength(1);

    const older = await start({ version: { v: 0, min: 0 } });
    const pcSide = phone();
    await pairWith(pcSide, older.pc, true);
    await until(pcSide, (s) => s.status === 'failed' && s.problem === 'update_pc');
    // Over KK too, and no retries.
    await pcSide.connect(pcSide.store.getState().activeId!);
    await until(pcSide, (s) => s.status === 'failed' && s.problem === 'update_pc');
  });

  it('forgets a PC here and connects to another by hand', async () => {
    const one = await start({ name: 'Desk' });
    const two = await start({ name: 'Laptop' });
    const remote = phone();
    await pairWith(remote, one.pc, true);
    await until(remote, (s) => s.status === 'connected');
    await pairWith(remote, two.pc, true);
    const state = await until(remote, (s) => s.status === 'connected' && s.hello?.name === 'Laptop');
    expect(state.pcs.map((pc) => pc.name)).toEqual(['Desk', 'Laptop']);
    const desk = state.pcs[0].id;
    await remote.connect(desk);
    await until(remote, (s) => s.status === 'connected' && s.hello?.name === 'Desk');
    await sleep(50);
    expect(two.pc.getDevices()[0].connected).toBe(false);
    await remote.remove(desk);
    expect(remote.store.getState()).toMatchObject({ activeId: null, status: 'idle' });
    expect(remote.store.getState().pcs.map((pc) => pc.name)).toEqual(['Laptop']);
  });

  it('controls playback, seeks, sets volume, jumps queue and handles ad mode', async () => {
    const { pc } = await start();
    const client = phone();
    await pairWith(client, pc, true);
    await until(client, (s) => s.status === 'connected' && s.player !== null && s.queue !== null);

    // Play and pause
    client.command({ c: 'pause' });
    await until(client, (s) => s.player?.paused === true);
    client.command({ c: 'play' });
    await until(client, (s) => s.player?.paused === false);

    // Seek
    client.command({ c: 'seek', s: 42 });
    await until(client, (s) => s.player !== null && Math.floor(s.player.position) === 42);

    // Volume
    client.command({ c: 'volume', v: 85 });
    await until(client, (s) => s.player?.volume === 85);

    // Play queue item
    const queue = client.store.getState().queue!;
    expect(queue.items.length).toBeGreaterThan(2);
    const target = queue.items[2];
    client.command({ c: 'play_item', id: target.id, i: 2 });
    await until(client, (s) => s.player?.track.id === target.id);

    // Ad mode
    pc.setAd(true);
    await until(client, (s) => s.player?.ad === true);

    // Seek during ad is refused
    client.command({ c: 'seek', s: 10 });
    await until(client, (s) => s.refusal === 'ad');

    // Pause during ad is allowed
    client.command({ c: 'pause' });
    await until(client, (s) => s.player?.paused === true);

    // Leave ad
    pc.setAd(false);
    await until(client, (s) => s.player?.ad === false);
  });
});
