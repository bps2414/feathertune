// Noise primitives for 25519, ChaChaPoly and SHA256, and the CipherState (spec rev 34, sections 4 and 5.1).

import { chacha20poly1305 } from '@noble/ciphers/chacha.js';
import { x25519 } from '@noble/curves/ed25519.js';
import { hmac } from '@noble/hashes/hmac.js';
import { sha256 } from '@noble/hashes/sha2.js';

export const DHLEN = 32;
export const HASHLEN = 32;
export const KEYLEN = 32;
export const TAGLEN = 16;
/** Largest Noise message, handshake or transport (spec section 3). */
export const MAX_MESSAGE = 65535;

/** Returns `length` cryptographically secure random bytes. */
export type RandomBytes = (length: number) => Uint8Array;

export interface KeyPair {
  publicKey: Uint8Array;
  privateKey: Uint8Array;
}

export function generateKeyPair(random: RandomBytes): KeyPair {
  const privateKey = random(DHLEN);
  if (privateKey.length !== DHLEN) throw new Error('noise: random source returned the wrong length');
  return { privateKey, publicKey: x25519.getPublicKey(privateKey) };
}

/** X25519. Throws on a low-order public key (all-zero output), which the spec allows. */
export function dh(privateKey: Uint8Array, publicKey: Uint8Array): Uint8Array {
  return x25519.getSharedSecret(privateKey, publicKey);
}

export function hash(...parts: Uint8Array[]): Uint8Array {
  return sha256(concat(...parts));
}

/** HKDF as Noise defines it (section 4.3), returning `outputs` (2 or 3) HASHLEN values. */
export function hkdf(chainingKey: Uint8Array, ikm: Uint8Array, outputs: 2 | 3): Uint8Array[] {
  const tempKey = hmac(sha256, chainingKey, ikm);
  const out: Uint8Array[] = [];
  let previous = new Uint8Array(0);
  for (let i = 1; i <= outputs; i++) {
    previous = hmac(sha256, tempKey, concat(previous, Uint8Array.of(i)));
    out.push(previous);
  }
  return out;
}

export function concat(...parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}

/** 2^64 - 1 is reserved (section 5.1): reaching it ends the CipherState. */
const MAX_NONCE = 0xffff_ffff_ffff_ffffn;

/** 32 zero bits, then the 64-bit nonce little-endian (section 12.3). */
function nonceBytes(n: bigint): Uint8Array {
  const out = new Uint8Array(12);
  new DataView(out.buffer).setBigUint64(4, n, true);
  return out;
}

/** A key and a nonce. No rekey: a connection is far from 2^64 messages. */
export class CipherState {
  private key: Uint8Array | null = null;
  private n = 0n;

  initializeKey(key: Uint8Array | null): void {
    this.key = key;
    this.n = 0n;
  }

  hasKey(): boolean {
    return this.key !== null;
  }

  /** For the vector tests and the spec's SetNonce(); a transport never needs it. */
  setNonce(n: bigint): void {
    this.n = n;
  }

  encryptWithAd(ad: Uint8Array, plaintext: Uint8Array): Uint8Array {
    if (this.key === null) return plaintext;
    if (this.n >= MAX_NONCE) throw new Error('noise: nonce exhausted');
    const out = chacha20poly1305(this.key, nonceBytes(this.n), ad).encrypt(plaintext);
    this.n++;
    return out;
  }

  /** Throws on a bad tag and keeps the nonce, so a forged or replayed message changes nothing. */
  decryptWithAd(ad: Uint8Array, ciphertext: Uint8Array): Uint8Array {
    if (this.key === null) return ciphertext;
    if (this.n >= MAX_NONCE) throw new Error('noise: nonce exhausted');
    let out: Uint8Array;
    try {
      out = chacha20poly1305(this.key, nonceBytes(this.n), ad).decrypt(ciphertext);
    } catch {
      throw new Error('noise: decryption failed');
    }
    this.n++;
    return out;
  }

  /** A transport message: the plaintext must fit in MAX_MESSAGE once encrypted. */
  encrypt(plaintext: Uint8Array, ad: Uint8Array = new Uint8Array(0)): Uint8Array {
    if (plaintext.length + TAGLEN > MAX_MESSAGE) throw new Error('noise: message too long');
    return this.encryptWithAd(ad, plaintext);
  }

  decrypt(ciphertext: Uint8Array, ad: Uint8Array = new Uint8Array(0)): Uint8Array {
    if (ciphertext.length > MAX_MESSAGE) throw new Error('noise: message too long');
    return this.decryptWithAd(ad, ciphertext);
  }
}
