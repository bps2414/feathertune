// One connection to a PC (docs/REMOTE.md, Protocol): the WebSocket, the Noise handshake (`IKpsk2` to pair,
// `KK` after that) and then the session's encrypted JSON messages. No retries here; ./remote decides those.

import { createHandshake, type KeyPair, type RandomBytes, type Transport } from '../noise';
import {
  closeProblem,
  KK_PROLOGUE,
  MAX_OUT,
  PAIR_PROLOGUE,
  pairingCode,
  parsePcMessage,
  type Hello,
  type PairingQr,
  type PcMessage,
  type Problem,
} from './protocol';

/** What this code needs of a WebSocket: the browser shape, which React Native's and `ws` both have. */
export interface SocketLike {
  binaryType: string;
  onopen: ((event: unknown) => void) | null;
  onmessage: ((event: { data: unknown }) => void) | null;
  onclose: ((event: { code: number }) => void) | null;
  onerror: ((event: unknown) => void) | null;
  send(data: Uint8Array): void;
  close(code?: number, reason?: string): void;
}

export type OpenSocket = (url: string) => SocketLike;

/** Each step of a handshake waits at most this long (the PC's own limit). */
export const STEP_MS = 10_000;
/** The PC asks the user for up to the 2 minutes of its pairing window, then closes with `expired`. */
const ANSWER_MS = 130_000;

/**
 * A connection that ended before a session. `answered`: the PC itself refused (a close code, or a reply), so
 * another of its addresses would not help.
 */
export class RemoteError extends Error {
  constructor(
    readonly problem: Problem,
    readonly answered: boolean
  ) {
    super(problem);
  }
}

type Item = Uint8Array | { closed: number } | 'text';

/** The frames and the close of one socket, read one at a time during a handshake, then handed to a listener. */
class Inbox {
  private items: Item[] = [];
  private waiter: ((item: Item) => void) | null = null;
  private sink: ((item: Item) => void) | null = null;
  /** Set while waiting for the socket to open. */
  private opening: ((failure: RemoteError | null) => void) | null = null;

  constructor(readonly socket: SocketLike) {
    socket.binaryType = 'arraybuffer';
    socket.onopen = () => this.opened(null);
    socket.onmessage = ({ data }) => {
      if (data instanceof ArrayBuffer) this.push(new Uint8Array(data));
      else if (ArrayBuffer.isView(data)) this.push(new Uint8Array(data.buffer, data.byteOffset, data.byteLength));
      else this.push('text');
    };
    socket.onclose = ({ code }) => {
      this.opened(new RemoteError('unreachable', false));
      this.push({ closed: code });
    };
    socket.onerror = () => this.opened(new RemoteError('unreachable', false));
  }

  /** Resolves once the socket is open; `blocked` when nothing answers in time, `unreachable` on an early error
   * (refused, no route, a 404). A PC that is awake answers or refuses; a firewall that drops says nothing. */
  open(): Promise<void> {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => this.opened(new RemoteError('blocked', false)), STEP_MS);
      this.opening = (failure) => {
        clearTimeout(timer);
        if (!failure) return resolve();
        this.drop();
        reject(failure);
      };
    });
  }

  private opened(failure: RemoteError | null): void {
    const opening = this.opening;
    this.opening = null;
    opening?.(failure);
  }

  private push(item: Item): void {
    if (this.sink) {
      this.sink(item);
    } else if (this.waiter) {
      const waiter = this.waiter;
      this.waiter = null;
      waiter(item);
    } else {
      this.items.push(item);
    }
  }

  /** The next frame; a close, a text frame or the timeout is a RemoteError. */
  async read(timeoutMs: number): Promise<Uint8Array> {
    const item =
      this.items.shift() ??
      (await new Promise<Item | null>((resolve) => {
        const timer = setTimeout(() => {
          this.waiter = null;
          resolve(null);
        }, timeoutMs);
        this.waiter = (found) => (clearTimeout(timer), resolve(found));
      }));
    if (item instanceof Uint8Array) return item;
    if (item === null) {
      this.drop();
      throw new RemoteError('timeout', true);
    }
    if (item === 'text') {
      this.drop();
      throw new RemoteError('invalid', true);
    }
    throw new RemoteError(closeProblem(item.closed) ?? 'lost', closeProblem(item.closed) !== null);
  }

  /** From here on every item goes to `on`, the queued ones first. */
  listen(on: (item: Item) => void): void {
    this.waiter = null;
    for (const item of this.items.splice(0)) on(item);
    this.sink = on;
  }

  drop(): void {
    try {
      this.socket.close(1000);
    } catch {
      // already closed
    }
  }
}

/** A session: the PC's messages in order, and the phone's commands and pings. */
export class Session {
  private ended = false;

  constructor(
    private readonly inbox: Inbox,
    private readonly transport: Transport
  ) {}

  /** Encrypts and sends one message; a message too large for the PC is a bug, so it throws. */
  send(message: object): void {
    if (this.ended) return;
    const plain = new TextEncoder().encode(JSON.stringify(message));
    if (plain.length > MAX_OUT) throw new Error('remote: message too large');
    this.inbox.socket.send(this.transport.send.encrypt(plain));
  }

  /**
   * Delivers each message from the PC, then `onEnd` once with why the session ended: the PC's close code, or
   * `lost` for a plain drop. A frame that does not decrypt ends it as `invalid`; a malformed message is skipped.
   */
  listen(onMessage: (message: PcMessage) => void, onEnd: (problem: Problem) => void): void {
    const end = (problem: Problem) => {
      if (this.ended) return;
      this.ended = true;
      this.inbox.drop();
      onEnd(problem);
    };
    this.inbox.listen((item) => {
      if (this.ended) return;
      if (item === 'text') return end('invalid');
      if (!(item instanceof Uint8Array)) return end(closeProblem(item.closed) ?? 'lost');
      let plain: Uint8Array;
      try {
        plain = this.transport.receive.decrypt(item);
      } catch {
        return end('invalid');
      }
      const message = parsePcMessage(plain);
      if (message) onMessage(message);
    });
  }

  /** Closes from this side; `onEnd` is not called. */
  close(): void {
    this.ended = true;
    this.inbox.drop();
  }
}

function url(address: string, port: number, path: string): string {
  return `ws://${address}:${port}${path}`;
}

/** A session with a paired PC over `KK`, its first `ping` sent. The PC then sends `hello`, `state`, `queue`, `pong`. */
export async function openSession(options: {
  openSocket: OpenSocket;
  random: RandomBytes;
  phone: KeyPair;
  pcKey: Uint8Array;
  address: string;
  port: number;
  ping: number;
}): Promise<Session> {
  const inbox = new Inbox(options.openSocket(url(options.address, options.port, '/v1')));
  await inbox.open();
  try {
    const hs = createHandshake({
      pattern: 'KK',
      initiator: true,
      staticKey: options.phone,
      remoteStatic: options.pcKey,
      prologue: KK_PROLOGUE,
      random: options.random,
    });
    inbox.socket.send(hs.writeMessage());
    const reply = await inbox.read(STEP_MS);
    try {
      hs.readMessage(reply);
    } catch {
      throw new RemoteError('invalid', true);
    }
    const session = new Session(inbox, hs.split());
    session.send({ t: 'ping', n: options.ping });
    return session;
  } catch (error) {
    inbox.drop();
    throw error;
  }
}

/**
 * Pairs with the PC of a scanned QR over `IKpsk2`: shows `onCode` once the PC replied, then waits for the user's
 * answer on the PC. Allowed, the connection goes on as a session; its `hello` is returned with it.
 */
export async function pair(options: {
  openSocket: OpenSocket;
  random: RandomBytes;
  phone: KeyPair;
  qr: PairingQr;
  address: string;
  name: string;
  onCode: (code: string) => void;
}): Promise<{ session: Session; hello: Hello }> {
  const { qr } = options;
  const inbox = new Inbox(options.openSocket(url(options.address, qr.port, '/v1/pair')));
  await inbox.open();
  try {
    const hs = createHandshake({
      pattern: 'IKpsk2',
      initiator: true,
      staticKey: options.phone,
      remoteStatic: qr.key,
      prologue: PAIR_PROLOGUE,
      psk: qr.secret,
      random: options.random,
    });
    inbox.socket.send(hs.writeMessage(new TextEncoder().encode(JSON.stringify({ name: options.name }))));
    const reply = await inbox.read(STEP_MS);
    try {
      hs.readMessage(reply);
    } catch {
      // Whatever answered does not have the secret: stop here.
      throw new RemoteError('not_this_pc', true);
    }
    options.onCode(pairingCode(hs.handshakeHash()));
    const transport = hs.split();
    const session = new Session(inbox, transport);
    session.send({ t: 'pair' });
    const first = await inbox.read(ANSWER_MS);
    let message: PcMessage | null = null;
    try {
      message = parsePcMessage(transport.receive.decrypt(first));
    } catch {
      // handled below
    }
    if (message?.t !== 'hello') throw new RemoteError('invalid', true);
    return { session, hello: message };
  } catch (error) {
    inbox.drop();
    throw error;
  }
}
