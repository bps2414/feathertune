// SymmetricState and HandshakeState (Noise spec rev 34, sections 5.2, 5.3 and 9) for the two patterns the
// remote uses: IKpsk2 to pair, KK for every connection after that (docs/REMOTE.md, Security model).

import {
  CipherState,
  concat,
  dh,
  DHLEN,
  generateKeyPair,
  hash,
  HASHLEN,
  hkdf,
  KEYLEN,
  MAX_MESSAGE,
  TAGLEN,
  type KeyPair,
  type RandomBytes,
} from './cipher';

export type Pattern = 'IKpsk2' | 'KK';

type Token = 'e' | 's' | 'ee' | 'es' | 'se' | 'ss' | 'psk';

interface PatternSpec {
  /** Pre-message static keys: `initiator` for `-> s`, `responder` for `<- s`. */
  preInitiator: boolean;
  preResponder: boolean;
  /** Message patterns, alternating from the initiator. */
  messages: Token[][];
}

const PATTERNS: Record<Pattern, PatternSpec> = {
  IKpsk2: {
    preInitiator: false,
    preResponder: true,
    messages: [
      ['e', 'es', 's', 'ss'],
      ['e', 'ee', 'se', 'psk'],
    ],
  },
  KK: {
    preInitiator: true,
    preResponder: true,
    messages: [
      ['e', 'es', 'ss'],
      ['e', 'ee', 'se'],
    ],
  },
};

const EMPTY = new Uint8Array(0);

class SymmetricState {
  readonly cipher = new CipherState();
  private ck: Uint8Array;
  h: Uint8Array;

  constructor(protocolName: string) {
    const name = new TextEncoder().encode(protocolName);
    if (name.length <= HASHLEN) {
      this.h = new Uint8Array(HASHLEN);
      this.h.set(name);
    } else {
      this.h = hash(name);
    }
    this.ck = this.h;
  }

  mixKey(ikm: Uint8Array): void {
    const [ck, tempK] = hkdf(this.ck, ikm, 2);
    this.ck = ck;
    this.cipher.initializeKey(tempK.subarray(0, KEYLEN));
  }

  mixHash(data: Uint8Array): void {
    this.h = hash(this.h, data);
  }

  mixKeyAndHash(ikm: Uint8Array): void {
    const [ck, tempH, tempK] = hkdf(this.ck, ikm, 3);
    this.ck = ck;
    this.mixHash(tempH);
    this.cipher.initializeKey(tempK.subarray(0, KEYLEN));
  }

  encryptAndHash(plaintext: Uint8Array): Uint8Array {
    const ciphertext = this.cipher.encryptWithAd(this.h, plaintext);
    this.mixHash(ciphertext);
    return ciphertext;
  }

  decryptAndHash(ciphertext: Uint8Array): Uint8Array {
    const plaintext = this.cipher.decryptWithAd(this.h, ciphertext);
    this.mixHash(ciphertext);
    return plaintext;
  }

  split(): [CipherState, CipherState] {
    const [k1, k2] = hkdf(this.ck, EMPTY, 2);
    const c1 = new CipherState();
    const c2 = new CipherState();
    c1.initializeKey(k1.subarray(0, KEYLEN));
    c2.initializeKey(k2.subarray(0, KEYLEN));
    return [c1, c2];
  }
}

export interface HandshakeOptions {
  pattern: Pattern;
  initiator: boolean;
  /** This side's static key pair. */
  staticKey: KeyPair;
  /** The peer's static public key: required by both sides of KK and by the IK initiator; the IK responder learns it. */
  remoteStatic?: Uint8Array;
  prologue?: Uint8Array;
  /** 32 bytes; required by IKpsk2, refused by KK. */
  psk?: Uint8Array;
  /** Source of the ephemeral key. */
  random: RandomBytes;
}

export interface Transport {
  /** This side's outgoing messages. */
  send: CipherState;
  /** The peer's messages. */
  receive: CipherState;
}

/**
 * One handshake. Call writeMessage/readMessage in turn (the initiator writes first); once `finished`, take the
 * transport with split(). Any error leaves it failed: discard it and close the connection.
 */
export class Handshake {
  private readonly spec: PatternSpec;
  private readonly initiator: boolean;
  private readonly ss: SymmetricState;
  private readonly s: KeyPair;
  private readonly psk: Uint8Array | null;
  private readonly random: RandomBytes;
  private e: KeyPair | null = null;
  private rs: Uint8Array | null;
  private re: Uint8Array | null = null;
  private index = 0;
  private failed = false;
  private result: [CipherState, CipherState] | null = null;

  constructor(options: HandshakeOptions) {
    const { pattern, initiator, staticKey, remoteStatic, prologue = EMPTY, psk, random } = options;
    const spec = PATTERNS[pattern];
    checkLength(staticKey.privateKey, DHLEN, 'static private key');
    checkLength(staticKey.publicKey, DHLEN, 'static public key');
    const needsRemote = initiator ? spec.preResponder : spec.preInitiator;
    if (needsRemote !== (remoteStatic !== undefined)) {
      throw new Error(`noise: ${pattern} ${needsRemote ? 'needs' : 'refuses'} a remote static key on this side`);
    }
    if (remoteStatic) checkLength(remoteStatic, DHLEN, 'remote static key');
    const usesPsk = spec.messages.some((m) => m.includes('psk'));
    if (usesPsk !== (psk !== undefined)) throw new Error(`noise: ${pattern} ${usesPsk ? 'needs' : 'refuses'} a psk`);
    if (psk) checkLength(psk, KEYLEN, 'psk');

    this.spec = spec;
    this.initiator = initiator;
    this.s = staticKey;
    this.rs = remoteStatic ?? null;
    this.psk = psk ?? null;
    this.random = random;
    this.ss = new SymmetricState(`Noise_${pattern}_25519_ChaChaPoly_SHA256`);
    this.ss.mixHash(prologue);
    // Pre-message keys, the initiator's first (section 7.1).
    if (spec.preInitiator) this.ss.mixHash(initiator ? staticKey.publicKey : remoteStatic!);
    if (spec.preResponder) this.ss.mixHash(initiator ? remoteStatic! : staticKey.publicKey);
  }

  get finished(): boolean {
    return this.result !== null;
  }

  /** Whose turn it is: true when this side writes the next message. */
  get isMyTurn(): boolean {
    return !this.finished && this.index % 2 === (this.initiator ? 0 : 1);
  }

  /** The peer's static public key: known from the start, or (IK responder) after the first message. */
  get remoteStaticKey(): Uint8Array | null {
    return this.rs;
  }

  /** The handshake hash (section 11.2), for channel binding such as the pairing code. Only once finished. */
  handshakeHash(): Uint8Array {
    if (!this.finished) throw new Error('noise: handshake not finished');
    return this.ss.h.slice();
  }

  /** The transport cipher states. Only once finished. */
  split(): Transport {
    if (!this.result) throw new Error('noise: handshake not finished');
    const [c1, c2] = this.result;
    return this.initiator ? { send: c1, receive: c2 } : { send: c2, receive: c1 };
  }

  writeMessage(payload: Uint8Array = EMPTY): Uint8Array {
    return this.step(true, () => {
      const parts: Uint8Array[] = [];
      for (const token of this.spec.messages[this.index]) {
        if (token === 'e') {
          this.e = generateKeyPair(this.random);
          parts.push(this.e.publicKey);
          this.mixEphemeral(this.e.publicKey);
        } else if (token === 's') {
          parts.push(this.ss.encryptAndHash(this.s.publicKey));
        } else {
          this.mixToken(token);
        }
      }
      parts.push(this.ss.encryptAndHash(payload));
      const message = concat(...parts);
      if (message.length > MAX_MESSAGE) throw new Error('noise: message too long');
      return message;
    });
  }

  readMessage(message: Uint8Array): Uint8Array {
    return this.step(false, () => {
      if (message.length > MAX_MESSAGE) throw new Error('noise: message too long');
      let at = 0;
      const take = (n: number) => {
        if (at + n > message.length) throw new Error('noise: message too short');
        const out = message.subarray(at, at + n);
        at += n;
        return out;
      };
      for (const token of this.spec.messages[this.index]) {
        if (token === 'e') {
          this.re = take(DHLEN).slice();
          this.mixEphemeral(this.re);
        } else if (token === 's') {
          this.rs = this.ss.decryptAndHash(take(this.ss.cipher.hasKey() ? DHLEN + TAGLEN : DHLEN)).slice();
        } else {
          this.mixToken(token);
        }
      }
      if (this.ss.cipher.hasKey() && message.length - at < TAGLEN) throw new Error('noise: message too short');
      return this.ss.decryptAndHash(message.subarray(at)).slice();
    });
  }

  private step(writing: boolean, run: () => Uint8Array): Uint8Array {
    if (this.failed) throw new Error('noise: handshake failed earlier');
    if (this.finished) throw new Error('noise: handshake already finished');
    if (this.isMyTurn !== writing) throw new Error(`noise: not this side's turn to ${writing ? 'write' : 'read'}`);
    try {
      const out = run();
      this.index++;
      if (this.index === this.spec.messages.length) this.result = this.ss.split();
      return out;
    } catch (error) {
      this.failed = true;
      throw error;
    }
  }

  private mixEphemeral(publicKey: Uint8Array): void {
    this.ss.mixHash(publicKey);
    if (this.psk) this.ss.mixKey(publicKey);
  }

  private mixToken(token: Exclude<Token, 'e' | 's'>): void {
    switch (token) {
      case 'ee':
        this.ss.mixKey(dh(this.e!.privateKey, this.re!));
        break;
      case 'es':
        this.ss.mixKey(this.initiator ? dh(this.e!.privateKey, this.rs!) : dh(this.s.privateKey, this.re!));
        break;
      case 'se':
        this.ss.mixKey(this.initiator ? dh(this.s.privateKey, this.re!) : dh(this.e!.privateKey, this.rs!));
        break;
      case 'ss':
        this.ss.mixKey(dh(this.s.privateKey, this.rs!));
        break;
      case 'psk':
        this.ss.mixKeyAndHash(this.psk!);
        break;
    }
  }
}

export function createHandshake(options: HandshakeOptions): Handshake {
  return new Handshake(options);
}

function checkLength(bytes: Uint8Array, length: number, what: string): void {
  if (bytes.length !== length) throw new Error(`noise: ${what} must be ${length} bytes`);
}
