// The phone's side of the remote: the paired PCs, the one connection to the chosen PC and its state, in a
// zustand store the screens read (docs/REMOTE.md, Lifecycle). Connected only in the foreground; it pings every
// 15 s, reconnects with backoff (last good address first, then the QR's) and closes a few seconds after the app
// goes to the background. Platform pieces (storage, sockets, randomness, the network) are injected.

import { bytesToHex, hexToBytes } from '@noble/hashes/utils.js';
import { createStore, type StoreApi } from 'zustand/vanilla';

import { generateKeyPair, type KeyPair, type RandomBytes } from '../noise';
import { openSession, pair, RemoteError, type OpenSocket, type Session, type SocketLike } from './client';
import {
  deviceId,
  isFinal,
  validName,
  versionProblem,
  type Command,
  type Hello,
  type PairingQr,
  type PcMessage,
  type PlayerState,
  type Problem,
  type QueueState,
} from './protocol';
import type { KeyStore } from './storage';

export interface PcRecord {
  /** The device id: the first 8 bytes of SHA-256 of `key`, hex. */
  id: string;
  /** The name the PC sent in `hello`. */
  name: string;
  /** The PC's static public key, hex. */
  key: string;
  /** From the QR. */
  addresses: string[];
  port: number;
  /** The last address a session opened on. */
  lastAddress: string | null;
  /** Unix seconds. */
  paired: number;
}

/**
 * `connecting`: a try is under way; `connected`: `hello` came and the versions match; `waiting`: the last try
 * failed and another is scheduled; `failed`: the user has to act (`problem` says how); `idle`: not trying (no PC
 * chosen, or the app is in the background).
 */
export type Status = 'idle' | 'connecting' | 'connected' | 'waiting' | 'failed';

export interface PairingState {
  status: 'idle' | 'scanned' | 'connecting' | 'code' | 'done' | 'failed';
  qr: PairingQr | null;
  /** The 6 digits to compare with the PC's screen. */
  code: string | null;
  problem: Problem | null;
  /** The PC paired, once done. */
  pcId: string | null;
}

export interface RemoteState {
  /** False until the storage has been read. */
  ready: boolean;
  phoneName: string;
  /** The local network permission was explained (and asked, on Android). */
  networkExplained: boolean;
  pcs: PcRecord[];
  activeId: string | null;
  status: Status;
  problem: Problem | null;
  hello: Hello | null;
  player: PlayerState | null;
  queue: QueueState | null;
  /** The PC's answer to the last refused command. */
  refusal: 'ad' | 'unavailable' | 'invalid' | null;
  /** PC clock minus phone clock, in milliseconds, from the last `pong`. */
  clockOffset: number;
  pairing: PairingState;
}

export interface NetworkInfo {
  /** On Wi-Fi, Ethernet or a VPN (where a PC can be). */
  lan: boolean;
  vpn: boolean;
  /** The phone's IPv4 address, when known. */
  ip: string | null;
}

export interface Timing {
  pingMs: number;
  /** Nothing from the PC this long (three pings unanswered): the session is gone. */
  silenceMs: number;
  /** How long after going to the background the socket is closed. */
  backgroundMs: number;
  /** The waits between tries, the last one repeating. */
  backoffMs: number[];
}

export const TIMING: Timing = {
  pingMs: 15_000,
  silenceMs: 45_000,
  backgroundMs: 5_000,
  backoffMs: [1_000, 2_000, 4_000, 8_000, 16_000, 30_000],
};

export interface RemoteDeps {
  storage: KeyStore;
  openSocket: OpenSocket;
  random: RandomBytes;
  /** What the phone is connected to, to tell "not on the same network" apart; none: not checked. */
  network?: () => Promise<NetworkInfo>;
  now?: () => number;
  timing?: Partial<Timing>;
}

const IDLE_PAIRING: PairingState = { status: 'idle', qr: null, code: null, problem: null, pcId: null };

const INITIAL: RemoteState = {
  ready: false,
  phoneName: '',
  networkExplained: false,
  pcs: [],
  activeId: null,
  status: 'idle',
  problem: null,
  hello: null,
  player: null,
  queue: null,
  refusal: null,
  clockOffset: 0,
  pairing: IDLE_PAIRING,
};

/** Storage keys (letters, digits, `.`, `-` and `_` only, as SecureStore needs). */
const KEY = {
  phone: 'phone-key',
  phoneName: 'phone-name',
  networkExplained: 'network-explained',
  active: 'active-pc',
  pcs: 'pcs',
  pc: (id: string) => `pc.${id}`,
};

export class Remote {
  readonly store: StoreApi<RemoteState>;
  private readonly deps: RemoteDeps;
  private readonly timing: Timing;
  private readonly now: () => number;
  private keys: KeyPair | null = null;
  private session: Session | null = null;
  /** The socket of the try or session under way, so stop() can close it at any step. */
  private socket: SocketLike | null = null;
  /** Bumped by stop(): a try or session from an older generation drops whatever it finishes. */
  private generation = 0;
  private failures = 0;
  private foreground = true;
  private nextPing = 0;
  private pings = new Map<number, number>();
  private retryTimer: ReturnType<typeof setTimeout> | null = null;
  private backgroundTimer: ReturnType<typeof setTimeout> | null = null;
  private pingTimer: ReturnType<typeof setInterval> | null = null;
  private silenceTimer: ReturnType<typeof setTimeout> | null = null;

  constructor(deps: RemoteDeps) {
    this.deps = deps;
    this.timing = { ...TIMING, ...deps.timing };
    this.now = deps.now ?? Date.now;
    this.store = createStore<RemoteState>(() => INITIAL);
  }

  private get state(): RemoteState {
    return this.store.getState();
  }

  private set(partial: Partial<RemoteState>): void {
    this.store.setState(partial);
  }

  /** Reads the phone key (made on first run) and the paired PCs, then connects to the chosen one. */
  async load(): Promise<void> {
    const { storage } = this.deps;
    const stored = parseKeys(await storage.get(KEY.phone));
    this.keys = stored ?? generateKeyPair(this.deps.random);
    if (!stored) await storage.set(KEY.phone, serializeKeys(this.keys));
    const ids = parseIds(await storage.get(KEY.pcs));
    const pcs: PcRecord[] = [];
    for (const id of ids) {
      const pc = parsePc(await storage.get(KEY.pc(id)));
      if (pc) pcs.push(pc);
    }
    const active = await storage.get(KEY.active);
    this.set({
      ready: true,
      pcs,
      activeId: pcs.some((pc) => pc.id === active) ? active : (pcs[0]?.id ?? null),
      phoneName: (await storage.get(KEY.phoneName)) ?? '',
      networkExplained: (await storage.get(KEY.networkExplained)) === '1',
    });
    void this.attempt();
  }

  /** Chooses a PC and connects to it (again), from the first try. */
  async connect(id: string): Promise<void> {
    if (!this.state.pcs.some((pc) => pc.id === id)) return;
    this.stop();
    this.failures = 0;
    this.set({ activeId: id, ...disconnected() });
    await this.deps.storage.set(KEY.active, id);
    void this.attempt();
  }

  /** Forgets a PC here (the PC still lists the phone until removed there). */
  async remove(id: string): Promise<void> {
    const pcs = this.state.pcs.filter((pc) => pc.id !== id);
    if (this.state.activeId === id) {
      this.stop();
      this.set({ activeId: null, ...disconnected() });
      await this.deps.storage.remove(KEY.active);
    }
    this.set({ pcs });
    await this.deps.storage.remove(KEY.pc(id));
    await this.deps.storage.set(KEY.pcs, JSON.stringify(pcs.map((pc) => pc.id)));
  }

  /** The app came to the foreground (reconnects at once) or left it (closes after a few seconds). */
  setForeground(on: boolean): void {
    if (on === this.foreground) return;
    this.foreground = on;
    if (!on) {
      this.clearRetry();
      this.backgroundTimer = setTimeout(() => {
        this.backgroundTimer = null;
        this.stop();
        if (this.state.status !== 'failed') this.set({ status: 'idle' });
      }, this.timing.backgroundMs);
      return;
    }
    if (this.backgroundTimer) clearTimeout(this.backgroundTimer);
    this.backgroundTimer = null;
    if (!this.session && this.state.status !== 'failed') {
      this.failures = 0;
      void this.attempt();
    }
  }

  /** Sends a command to the connected PC; nothing happens when there is no session. */
  command(command: Command): void {
    if (!this.session || this.state.status !== 'connected') return;
    this.set({ refusal: null });
    this.session.send({ t: 'cmd', ...command });
  }

  async setNetworkExplained(): Promise<void> {
    this.set({ networkExplained: true });
    await this.deps.storage.set(KEY.networkExplained, '1');
  }

  // ---- pairing

  /** A QR was scanned and read: the pairing screens go on from here. */
  setPairingQr(qr: PairingQr): void {
    this.set({ pairing: { ...IDLE_PAIRING, status: 'scanned', qr } });
  }

  /** Pairs with the scanned PC under `name`; on Allow it becomes the chosen PC and stays connected. */
  async pair(name: string): Promise<void> {
    const qr = this.state.pairing.qr;
    const phoneName = validName(name);
    if (!qr || !phoneName || !this.keys) return;
    this.stop();
    const generation = this.generation;
    this.set({ phoneName, pairing: { ...IDLE_PAIRING, status: 'connecting', qr } });
    await this.deps.storage.set(KEY.phoneName, phoneName);
    let blocked = false;
    for (const address of qr.addresses) {
      try {
        const { session, hello } = await pair({
          openSocket: this.open,
          random: this.deps.random,
          phone: this.keys,
          qr,
          address,
          name: phoneName,
          onCode: (code) => generation === this.generation && this.set({ pairing: { ...this.state.pairing, status: 'code', code } }),
        });
        if (generation !== this.generation) return session.close();
        const pc: PcRecord = {
          id: deviceId(qr.key),
          name: hello.name,
          key: bytesToHex(qr.key),
          addresses: qr.addresses,
          port: qr.port,
          lastAddress: address,
          paired: Math.floor(this.now() / 1000),
        };
        await this.savePc(pc);
        await this.deps.storage.set(KEY.active, pc.id);
        this.failures = 0;
        this.set({ activeId: pc.id, ...disconnected(), pairing: { ...this.state.pairing, status: 'done', pcId: pc.id } });
        this.adopt(session, pc, address, { t: 'hello', ...hello });
        return;
      } catch (error) {
        if (generation !== this.generation) return;
        const e = error instanceof RemoteError ? error : new RemoteError('unreachable', false);
        // Once the PC answered (a close code, or its reply), the secret is spent or the PC said why.
        if (e.answered || this.state.pairing.status === 'code') return this.pairingFailed(e.problem);
        blocked ||= e.problem === 'blocked';
      }
    }
    this.pairingFailed(await this.networkProblem(qr.addresses, blocked ? 'blocked' : 'unreachable'));
  }

  /** Leaves the pairing screens, done or not; a pairing under way is dropped. */
  leavePairing(): void {
    const { status } = this.state.pairing;
    if (status === 'connecting' || status === 'code') this.stop();
    this.set({ pairing: IDLE_PAIRING });
    if (!this.session) void this.attempt();
  }

  private pairingFailed(problem: Problem): void {
    this.set({ pairing: { ...this.state.pairing, status: 'failed', problem } });
  }

  // ---- the connection

  private readonly open: OpenSocket = (url) => (this.socket = this.deps.openSocket(url));

  private active(): PcRecord | null {
    return this.state.pcs.find((pc) => pc.id === this.state.activeId) ?? null;
  }

  /** One round of tries: the last good address, then each of the QR's. */
  private async attempt(): Promise<void> {
    const pc = this.active();
    if (!pc || !this.foreground || !this.keys || this.session) return;
    const busy = this.state.pairing.status;
    if (busy === 'connecting' || busy === 'code') return;
    this.stop();
    const generation = this.generation;
    this.set({ status: 'connecting' });
    const addresses = [...new Set([pc.lastAddress, ...pc.addresses].filter((a): a is string => a !== null))];
    let answer: Problem | null = null;
    let removed = false;
    let blocked = false;
    for (const address of addresses) {
      try {
        const ping = this.ping();
        const session = await openSession({
          openSocket: this.open,
          random: this.deps.random,
          phone: this.keys,
          pcKey: hexToBytes(pc.key),
          address,
          port: pc.port,
          ping,
        });
        if (generation !== this.generation) return session.close();
        this.adopt(session, pc, address);
        return;
      } catch (error) {
        if (generation !== this.generation) return;
        const e = error instanceof RemoteError ? error : new RemoteError('unreachable', false);
        // Another feathertune PC may have taken the last address: `unknown` from it is not the last word.
        if (e.problem === 'removed') removed = true;
        else if (e.answered) {
          answer = e.problem;
          break;
        } else blocked ||= e.problem === 'blocked';
      }
    }
    const problem = answer ?? (removed ? 'removed' : await this.networkProblem(pc.addresses, blocked ? 'blocked' : 'unreachable'));
    if (generation === this.generation) this.failed(problem);
  }

  /** `other_network` when the phone is not where the PC can be, else `fallback`. */
  private async networkProblem(addresses: string[], fallback: Problem): Promise<Problem> {
    const network = await this.deps.network?.().catch(() => null);
    return network && otherNetwork(network, addresses) ? 'other_network' : fallback;
  }

  private failed(problem: Problem): void {
    this.set({ ...disconnected(), status: isFinal(problem) ? 'failed' : 'waiting', problem });
    if (isFinal(problem) || !this.foreground) return;
    const { backoffMs } = this.timing;
    const wait = backoffMs[Math.min(this.failures, backoffMs.length - 1)];
    this.failures++;
    this.clearRetry();
    this.retryTimer = setTimeout(() => {
      this.retryTimer = null;
      void this.attempt();
    }, wait);
  }

  /** Takes an open session as the connection: its messages, its pings and its silence. `first`: a message the
   * handshake already read (pairing's `hello`), handled before the rest. */
  private adopt(session: Session, pc: PcRecord, address: string, first?: PcMessage): void {
    const generation = this.generation;
    this.session = session;
    if (pc.lastAddress !== address) void this.savePc({ ...pc, lastAddress: address });
    if (first) this.onMessage(first);
    if (generation !== this.generation) return;
    session.listen(
      (message) => generation === this.generation && this.onMessage(message),
      (problem) => generation === this.generation && this.ended(problem)
    );
    this.pingTimer = setInterval(() => {
      session.send({ t: 'ping', n: this.ping() });
    }, this.timing.pingMs);
    this.heard();
  }

  private ended(problem: Problem): void {
    this.stop();
    this.failed(problem);
  }

  private onMessage(message: PcMessage): void {
    this.heard();
    switch (message.t) {
      case 'hello': {
        const hello: Hello = { v: message.v, min: message.min, name: message.name };
        const mismatch = versionProblem(hello);
        if (mismatch) return this.ended(mismatch);
        this.failures = 0;
        this.set({ status: 'connected', problem: null, hello });
        const pc = this.active();
        if (pc && pc.name !== hello.name) void this.savePc({ ...pc, name: hello.name });
        break;
      }
      case 'state': {
        const { track, position, paused, ad, volume, muted, colors, at } = message;
        this.set({ player: { track, position, paused, ad, volume, muted, colors, at } });
        break;
      }
      case 'queue':
        this.set({ queue: { items: message.items, current: message.current } });
        break;
      case 'error':
        this.set({ refusal: message.code });
        break;
      case 'pong': {
        const sent = this.pings.get(message.n);
        this.pings.delete(message.n);
        if (sent !== undefined) this.set({ clockOffset: Math.round(message.at - (sent + this.now()) / 2) });
        break;
      }
    }
  }

  /** A new ping number, remembered with its time for the clock offset. */
  private ping(): number {
    const n = this.nextPing++;
    this.pings.set(n, this.now());
    if (this.pings.size > 8) this.pings.delete(this.pings.keys().next().value!);
    return n;
  }

  /** Something came from the PC: it is still there. */
  private heard(): void {
    if (this.silenceTimer) clearTimeout(this.silenceTimer);
    const generation = this.generation;
    this.silenceTimer = setTimeout(() => generation === this.generation && this.ended('lost'), this.timing.silenceMs);
  }

  /** Ends the try or session under way, and every timer but the background one. */
  private stop(): void {
    this.generation++;
    this.clearRetry();
    if (this.pingTimer) clearInterval(this.pingTimer);
    if (this.silenceTimer) clearTimeout(this.silenceTimer);
    this.pingTimer = this.silenceTimer = null;
    this.session?.close();
    this.session = null;
    try {
      this.socket?.close(1000);
    } catch {
      // already closed
    }
    this.socket = null;
  }

  private clearRetry(): void {
    if (this.retryTimer) clearTimeout(this.retryTimer);
    this.retryTimer = null;
  }

  private async savePc(pc: PcRecord): Promise<void> {
    const pcs = [...this.state.pcs.filter((p) => p.id !== pc.id), pc];
    this.set({ pcs });
    await this.deps.storage.set(KEY.pc(pc.id), JSON.stringify(pc));
    await this.deps.storage.set(KEY.pcs, JSON.stringify(pcs.map((p) => p.id)));
  }
}

function disconnected(): Partial<RemoteState> {
  return { status: 'idle', problem: null, hello: null, player: null, queue: null, refusal: null };
}

/**
 * True when the phone cannot be on the PC's network: no Wi-Fi, Ethernet or VPN, or a home network of another
 * /24 (a phone on 192.168.1.x does not reach a PC on 192.168.0.x). Other ranges may be larger, so no verdict.
 */
export function otherNetwork(network: NetworkInfo, addresses: string[]): boolean {
  if (!network.lan) return true;
  if (network.vpn || !network.ip || !network.ip.startsWith('192.168.')) return false;
  const subnet = network.ip.split('.')[2];
  return addresses.every((a) => a.startsWith('192.168.') && a.split('.')[2] !== subnet);
}

const HEX64 = /^[0-9a-f]{64}$/;

function serializeKeys(keys: KeyPair): string {
  return JSON.stringify({ private: bytesToHex(keys.privateKey), public: bytesToHex(keys.publicKey) });
}

function parseKeys(text: string | null): KeyPair | null {
  try {
    const k = JSON.parse(text ?? 'null');
    return HEX64.test(k?.private) && HEX64.test(k?.public) ? { privateKey: hexToBytes(k.private), publicKey: hexToBytes(k.public) } : null;
  } catch {
    return null;
  }
}

function parseIds(text: string | null): string[] {
  try {
    const ids: unknown = JSON.parse(text ?? '[]');
    return Array.isArray(ids) ? ids.filter((id): id is string => typeof id === 'string' && /^[0-9a-f]{16}$/.test(id)) : [];
  } catch {
    return [];
  }
}

function parsePc(text: string | null): PcRecord | null {
  try {
    const pc = JSON.parse(text ?? 'null');
    const ok =
      typeof pc?.id === 'string' &&
      typeof pc.name === 'string' &&
      HEX64.test(pc.key) &&
      Array.isArray(pc.addresses) &&
      pc.addresses.every((a: unknown) => typeof a === 'string') &&
      Number.isInteger(pc.port) &&
      (pc.lastAddress === null || typeof pc.lastAddress === 'string') &&
      Number.isInteger(pc.paired);
    return ok && pc.id === deviceId(hexToBytes(pc.key)) ? (pc as PcRecord) : null;
  } catch {
    return null;
  }
}
