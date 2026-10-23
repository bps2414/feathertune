// The wire protocol as the phone sees it (docs/REMOTE.md, Protocol): the pairing QR, the 6-digit code, the
// messages from the PC in their fixed shape and what each close code means here.

import { sha256 } from '@noble/hashes/sha2.js';
import { bytesToHex, hexToBytes } from '@noble/hashes/utils.js';

/** The protocol version this app speaks; a PC's `hello` must have it within `min`..`v`. */
export const PROTOCOL = 1;
export const DEFAULT_PORT = 47652;
export const PAIR_PROLOGUE = new TextEncoder().encode('feathertune-remote/1 pair');
export const KK_PROLOGUE = new TextEncoder().encode('feathertune-remote/1');
/** The largest JSON a controller may send (the frame adds the 16-byte tag). */
export const MAX_OUT = 1024;
export const MAX_NAME = 32;

/**
 * Why there is no session, as the phone tells the user. The close codes of the table in docs/REMOTE.md, plus what
 * the phone finds out by itself: no answer (`blocked`), an error before any answer (`unreachable`), a phone off the
 * PC's network, a version outside `min`..`v`, a pairing reply that does not decrypt (`not_this_pc`) and a session
 * that went quiet or dropped (`lost`).
 */
export type Problem =
  | 'denied'
  | 'expired'
  | 'removed'
  | 'busy'
  | 'timeout'
  | 'invalid'
  | 'too_big'
  | 'off'
  | 'full'
  | 'unreachable'
  | 'blocked'
  | 'other_network'
  | 'update_app'
  | 'update_pc'
  | 'not_this_pc'
  | 'lost';

const CLOSE_CODES: Record<number, Problem> = {
  4000: 'denied',
  4001: 'expired',
  4002: 'removed', // unknown: the PC does not have this key (removed, or Forget all devices)
  4003: 'removed',
  4004: 'busy',
  4005: 'timeout',
  4006: 'invalid',
  4007: 'too_big',
  4008: 'off',
  4009: 'full',
};

/** The problem a close code from the PC means; null for a code outside the table (a plain drop). */
export function closeProblem(code: number): Problem | null {
  return CLOSE_CODES[code] ?? null;
}

/** Problems that more tries will not fix: the user has to act (pair again, update). */
export function isFinal(problem: Problem): boolean {
  return problem === 'removed' || problem === 'update_app' || problem === 'update_pc' || problem === 'too_big';
}

export interface PairingQr {
  v: number;
  /** The PC's static public key, 32 bytes. */
  key: Uint8Array;
  secret: Uint8Array;
  addresses: string[];
  port: number;
}

const HEX64 = /^[0-9a-f]{64}$/;
const IPV4 = /^(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(\.(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3}$/;

/** `feathertune://pair?v=1&k=…&s=…&a=…&p=…`; null for anything else. Unknown parameters are ignored. */
export function parsePairingUri(text: string): PairingQr | null {
  const match = /^feathertune:\/\/pair\?(.*)$/.exec(text.trim());
  if (!match) return null;
  const params = new Map<string, string>();
  for (const part of match[1].split('&')) {
    const at = part.indexOf('=');
    if (at > 0 && !params.has(part.slice(0, at))) params.set(part.slice(0, at), part.slice(at + 1));
  }
  const v = Number(params.get('v'));
  const k = params.get('k') ?? '';
  const s = params.get('s') ?? '';
  const addresses = (params.get('a') ?? '').split(',').filter((a) => IPV4.test(a) && privateIpv4(a));
  const port = Number(params.get('p') ?? DEFAULT_PORT);
  if (!Number.isInteger(v) || v < 1 || !HEX64.test(k) || !HEX64.test(s) || addresses.length === 0) return null;
  if (!Number.isInteger(port) || port < 1 || port > 65535) return null;
  return { v, key: hexToBytes(k), secret: hexToBytes(s), addresses: [...new Set(addresses)], port };
}

/** The addresses a PC accepts connections from, which are also the only ones worth trying. */
export function privateIpv4(address: string): boolean {
  const [a, b] = address.split('.').map(Number);
  return (
    a === 10 ||
    (a === 172 && b >= 16 && b <= 31) ||
    (a === 192 && b === 168) ||
    (a === 169 && b === 254) ||
    a === 127 ||
    (a === 100 && b >= 64 && b <= 127)
  );
}

/** The PC's device id: the first 8 bytes of SHA-256 of its public key, hex. */
export function deviceId(publicKey: Uint8Array): string {
  return bytesToHex(sha256(publicKey).slice(0, 8));
}

/** The 6 digits both screens show: the first 4 bytes of the handshake hash, big-endian, modulo 1 000 000. */
export function pairingCode(hash: Uint8Array): string {
  const n = new DataView(hash.buffer, hash.byteOffset, 4).getUint32(0, false);
  return String(n % 1_000_000).padStart(6, '0');
}

/** 1 to 32 characters once trimmed, no control characters (the PC refuses anything else). */
export function validName(name: string): string | null {
  const trimmed = name.trim();
  const count = [...trimmed].length;
  return count >= 1 && count <= MAX_NAME && !/[\u0000-\u001f\u007f-\u009f]/.test(trimmed) ? trimmed : null;
}

export interface Hello {
  v: number;
  min: number;
  name: string;
}

export interface Track {
  /** The 11-character video id, or null. */
  id: string | null;
  title: string;
  artist: string;
  album: string;
  /** An https URL, or null. */
  artwork: string | null;
  /** Seconds; 0 while unknown. */
  duration: number;
}

export interface PlayerState {
  track: Track;
  /** Seconds, as of `at`. */
  position: number;
  paused: boolean;
  ad: boolean;
  volume: number;
  muted: boolean;
  /** `[accent, ink, text on accent]`, or empty. */
  colors: string[];
  /** Unix milliseconds, PC clock. */
  at: number;
}

export interface QueueItem {
  id: string;
  title: string;
  artist: string;
}

export interface QueueState {
  items: QueueItem[];
  current: number | null;
}

export type PcMessage =
  | ({ t: 'hello' } & Hello)
  | ({ t: 'state' } & PlayerState)
  | ({ t: 'queue' } & QueueState)
  | { t: 'error'; code: 'ad' | 'unavailable' | 'invalid' }
  | { t: 'pong'; n: number; at: number };

export type Command =
  | { c: 'play' | 'pause' | 'next' | 'previous' }
  | { c: 'seek'; s: number }
  | { c: 'volume'; v: number }
  | { c: 'play_item'; id: string; i: number };

type Json = Record<string, unknown>;

const str = (v: unknown): v is string => typeof v === 'string';
const num = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);
const bool = (v: unknown): v is boolean => typeof v === 'boolean';
const obj = (v: unknown): v is Json => v !== null && typeof v === 'object' && !Array.isArray(v);
const VIDEO_ID = /^[A-Za-z0-9_-]{11}$/;
const COLOR = /^#[0-9a-fA-F]{6}$/;

function track(v: unknown): Track | null {
  if (!obj(v) || !str(v.title) || !str(v.artist) || !str(v.album) || !num(v.duration) || v.duration < 0) return null;
  const id = str(v.id) && VIDEO_ID.test(v.id) ? v.id : null;
  const artwork = str(v.artwork) && v.artwork.startsWith('https://') ? v.artwork : null;
  return { id, title: v.title, artist: v.artist, album: v.album, artwork, duration: v.duration };
}

/** One message from the PC in its fixed shape; null for anything malformed or unknown (it is skipped). */
export function parsePcMessage(bytes: Uint8Array): PcMessage | null {
  let m: unknown;
  try {
    m = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
  } catch {
    return null;
  }
  if (!obj(m)) return null;
  switch (m.t) {
    case 'hello':
      return Number.isInteger(m.v) && Number.isInteger(m.min) && str(m.name)
        ? { t: 'hello', v: m.v as number, min: m.min as number, name: m.name.trim().slice(0, 64) || 'feathertune' }
        : null;
    case 'state': {
      const t = track(m.track);
      const colors = Array.isArray(m.colors) && m.colors.every((c) => str(c) && COLOR.test(c)) ? (m.colors as string[]) : [];
      if (!t || !num(m.position) || !bool(m.paused) || !bool(m.ad) || !num(m.volume) || !bool(m.muted) || !num(m.at)) {
        return null;
      }
      const volume = Math.min(100, Math.max(0, m.volume));
      return { t: 'state', track: t, position: Math.max(0, m.position), paused: m.paused, ad: m.ad, volume, muted: m.muted, colors, at: m.at };
    }
    case 'queue': {
      if (!Array.isArray(m.items)) return null;
      const items: QueueItem[] = [];
      for (const item of m.items.slice(0, 200)) {
        if (!obj(item) || !str(item.id) || !VIDEO_ID.test(item.id) || !str(item.title) || !str(item.artist)) return null;
        items.push({ id: item.id, title: item.title, artist: item.artist });
      }
      const current = Number.isInteger(m.current) && (m.current as number) >= 0 && (m.current as number) < items.length ? (m.current as number) : null;
      return { t: 'queue', items, current };
    }
    case 'error':
      return m.code === 'ad' || m.code === 'unavailable' || m.code === 'invalid' ? { t: 'error', code: m.code } : null;
    case 'pong':
      return Number.isSafeInteger(m.n) && num(m.at) ? { t: 'pong', n: m.n as number, at: m.at } : null;
    default:
      return null;
  }
}

/** Null when this app and the PC share a version; otherwise which side to update. */
export function versionProblem(hello: Hello): 'update_app' | 'update_pc' | null {
  if (PROTOCOL < hello.min) return 'update_app';
  if (PROTOCOL > hello.v) return 'update_pc';
  return null;
}
