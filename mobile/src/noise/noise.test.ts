import { x25519 } from '@noble/curves/ed25519.js';
import { bytesToHex, hexToBytes } from '@noble/hashes/utils.js';

import { CipherState, createHandshake, generateKeyPair, MAX_MESSAGE, type Handshake, type Transport } from './index';
import fixture from './fixtures/cacophony.json';

/** Node's crypto, through its Web Crypto global (no Node typings in this project). */
const nodeRandom = (length: number) => crypto.getRandomValues(new Uint8Array(length));
const utf8 = (text: string) => new TextEncoder().encode(text);

interface Vector {
  protocol_name: string;
  init_prologue: string;
  init_psks?: string[];
  init_static: string;
  init_ephemeral: string;
  init_remote_static?: string;
  resp_prologue: string;
  resp_psks?: string[];
  resp_static: string;
  resp_ephemeral: string;
  resp_remote_static?: string;
  handshake_hash: string;
  messages: { payload: string; ciphertext: string }[];
}

/** Hands out the vector's ephemeral key once, as the handshake's only call for randomness. */
function fixedRandom(hex: string) {
  let used = false;
  return (length: number) => {
    if (used || length !== 32) throw new Error('unexpected call for randomness');
    used = true;
    return hexToBytes(hex);
  };
}

function staticKey(hex: string) {
  const privateKey = hexToBytes(hex);
  return { privateKey, publicKey: x25519.getPublicKey(privateKey) };
}

describe('cacophony vectors', () => {
  const vectors = fixture.vectors as Vector[];

  it('has one vector for each pattern', () => {
    expect(vectors.map((v) => v.protocol_name).sort()).toEqual([
      'Noise_IKpsk2_25519_ChaChaPoly_SHA256',
      'Noise_KK_25519_ChaChaPoly_SHA256',
    ]);
  });

  it.each(vectors.map((v) => [v.protocol_name, v] as const))('%s', (_, v) => {
    const pattern = v.protocol_name.split('_')[1] as 'IKpsk2' | 'KK';
    const initiator = createHandshake({
      pattern,
      initiator: true,
      staticKey: staticKey(v.init_static),
      remoteStatic: v.init_remote_static ? hexToBytes(v.init_remote_static) : undefined,
      prologue: hexToBytes(v.init_prologue),
      psk: v.init_psks ? hexToBytes(v.init_psks[0]) : undefined,
      random: fixedRandom(v.init_ephemeral),
    });
    const responder = createHandshake({
      pattern,
      initiator: false,
      staticKey: staticKey(v.resp_static),
      remoteStatic: v.resp_remote_static ? hexToBytes(v.resp_remote_static) : undefined,
      prologue: hexToBytes(v.resp_prologue),
      psk: v.resp_psks ? hexToBytes(v.resp_psks[0]) : undefined,
      random: fixedRandom(v.resp_ephemeral),
    });

    let transports: [Transport, Transport] | null = null;
    v.messages.forEach((m, i) => {
      const payload = hexToBytes(m.payload);
      const fromInitiator = i % 2 === 0;
      let ciphertext: Uint8Array;
      let received: Uint8Array;
      if (!transports) {
        const [sender, receiver] = fromInitiator ? [initiator, responder] : [responder, initiator];
        ciphertext = sender.writeMessage(payload);
        received = receiver.readMessage(ciphertext);
        if (initiator.finished && responder.finished) transports = [initiator.split(), responder.split()];
      } else {
        const [sender, receiver] = fromInitiator ? transports : [transports[1], transports[0]];
        ciphertext = sender.send.encrypt(payload);
        received = receiver.receive.decrypt(ciphertext);
      }
      expect(bytesToHex(ciphertext)).toBe(m.ciphertext);
      expect(bytesToHex(received)).toBe(m.payload);
    });

    expect(transports).not.toBeNull();
    expect(bytesToHex(initiator.handshakeHash())).toBe(v.handshake_hash);
    expect(bytesToHex(responder.handshakeHash())).toBe(v.handshake_hash);
  });
});

/** Runs the two handshake messages; returns both transports. */
function handshake(initiator: Handshake, responder: Handshake, payloads = [utf8('hi'), utf8('hello')]) {
  expect(responder.readMessage(initiator.writeMessage(payloads[0]))).toEqual(payloads[0]);
  expect(initiator.readMessage(responder.writeMessage(payloads[1]))).toEqual(payloads[1]);
  expect(initiator.finished && responder.finished).toBe(true);
  return [initiator.split(), responder.split()] as const;
}

describe('initiator and responder', () => {
  const pc = generateKeyPair(nodeRandom);
  const phone = generateKeyPair(nodeRandom);
  const psk = nodeRandom(32);
  const prologue = utf8('feathertune remote v1');

  const pairing = (initiatorPsk = psk) => [
    createHandshake({
      pattern: 'IKpsk2',
      initiator: true,
      staticKey: phone,
      remoteStatic: pc.publicKey,
      prologue,
      psk: initiatorPsk,
      random: nodeRandom,
    }),
    createHandshake({ pattern: 'IKpsk2', initiator: false, staticKey: pc, prologue, psk, random: nodeRandom }),
  ];

  const kk = (phoneKey = phone) => [
    createHandshake({
      pattern: 'KK',
      initiator: true,
      staticKey: phoneKey,
      remoteStatic: pc.publicKey,
      prologue,
      random: nodeRandom,
    }),
    createHandshake({
      pattern: 'KK',
      initiator: false,
      staticKey: pc,
      remoteStatic: phone.publicKey,
      prologue,
      random: nodeRandom,
    }),
  ];

  it('pairs with IKpsk2: the PC learns the phone key, both share the hash and talk both ways', () => {
    const [i, r] = pairing();
    expect(r.remoteStaticKey).toBeNull();
    const [phoneT, pcT] = handshake(i, r);
    expect(r.remoteStaticKey).toEqual(phone.publicKey);
    expect(i.handshakeHash()).toEqual(r.handshakeHash());
    for (let n = 0; n < 3; n++) {
      expect(pcT.receive.decrypt(phoneT.send.encrypt(utf8(`cmd ${n}`)))).toEqual(utf8(`cmd ${n}`));
      expect(phoneT.receive.decrypt(pcT.send.encrypt(utf8(`state ${n}`)))).toEqual(utf8(`state ${n}`));
    }
  });

  it('connects with KK', () => {
    const [i, r] = kk();
    const [phoneT, pcT] = handshake(i, r);
    expect(i.handshakeHash()).toEqual(r.handshakeHash());
    expect(pcT.receive.decrypt(phoneT.send.encrypt(utf8('ping')))).toEqual(utf8('ping'));
  });

  it('fails IKpsk2 with a different psk, on the responder message', () => {
    const [i, r] = pairing(nodeRandom(32));
    r.readMessage(i.writeMessage());
    expect(() => i.readMessage(r.writeMessage())).toThrow('decryption failed');
    expect(() => i.readMessage(new Uint8Array(48))).toThrow('failed earlier');
  });

  it('refuses KK from an unknown phone on its first message', () => {
    const [i, r] = kk(generateKeyPair(nodeRandom));
    expect(() => r.readMessage(i.writeMessage())).toThrow('decryption failed');
  });

  it('refuses a tampered, replayed or reordered transport message', () => {
    const [phoneT, pcT] = handshake(...(kk() as [Handshake, Handshake]));
    const first = phoneT.send.encrypt(utf8('one'));
    const second = phoneT.send.encrypt(utf8('two'));
    const tampered = first.slice();
    tampered[0] ^= 1;
    expect(() => pcT.receive.decrypt(tampered)).toThrow('decryption failed');
    expect(() => pcT.receive.decrypt(second)).toThrow('decryption failed');
    expect(pcT.receive.decrypt(first)).toEqual(utf8('one'));
    expect(() => pcT.receive.decrypt(first)).toThrow('decryption failed');
    expect(pcT.receive.decrypt(second)).toEqual(utf8('two'));
  });

  it('keeps messages within 65535 bytes', () => {
    const [phoneT, pcT] = handshake(...(kk() as [Handshake, Handshake]));
    const largest = phoneT.send.encrypt(new Uint8Array(MAX_MESSAGE - 16));
    expect(largest.length).toBe(MAX_MESSAGE);
    expect(pcT.receive.decrypt(largest).length).toBe(MAX_MESSAGE - 16);
    expect(() => phoneT.send.encrypt(new Uint8Array(MAX_MESSAGE - 15))).toThrow('too long');
    expect(() => pcT.receive.decrypt(new Uint8Array(MAX_MESSAGE + 1))).toThrow('too long');
    const [i, r] = kk();
    expect(() => i.writeMessage(new Uint8Array(MAX_MESSAGE))).toThrow('too long');
    expect(() => r.readMessage(new Uint8Array(MAX_MESSAGE + 1))).toThrow('too long');
  });

  it('enforces turns and the options each pattern needs', () => {
    const [i, r] = kk();
    expect(() => r.writeMessage()).toThrow("not this side's turn");
    expect(() => i.readMessage(new Uint8Array(64))).toThrow("not this side's turn");
    expect(() => i.split()).toThrow('not finished');
    expect(() => i.handshakeHash()).toThrow('not finished');
    const base = { random: nodeRandom };
    expect(() => createHandshake({ ...base, pattern: 'KK', initiator: true, staticKey: phone })).toThrow(
      'needs a remote static key',
    );
    expect(() =>
      createHandshake({ ...base, pattern: 'IKpsk2', initiator: false, staticKey: pc, remoteStatic: phone.publicKey, psk }),
    ).toThrow('refuses a remote static key');
    expect(() =>
      createHandshake({ ...base, pattern: 'IKpsk2', initiator: true, staticKey: phone, remoteStatic: pc.publicKey }),
    ).toThrow('needs a psk');
    expect(() =>
      createHandshake({ ...base, pattern: 'KK', initiator: true, staticKey: phone, remoteStatic: pc.publicKey, psk }),
    ).toThrow('refuses a psk');
  });

  it('refuses a message too short for its keys', () => {
    const [i, r] = kk();
    i.writeMessage();
    expect(() => r.readMessage(nodeRandom(40))).toThrow('too short');
  });
});

describe('CipherState', () => {
  it('stops at the reserved nonce 2^64 - 1', () => {
    const key = nodeRandom(32);
    const send = new CipherState();
    const receive = new CipherState();
    send.initializeKey(key);
    receive.initializeKey(key);
    send.setNonce(0xffff_ffff_ffff_fffen);
    receive.setNonce(0xffff_ffff_ffff_fffen);
    expect(receive.decrypt(send.encrypt(utf8('last')))).toEqual(utf8('last'));
    expect(() => send.encrypt(utf8('more'))).toThrow('nonce exhausted');
    expect(() => receive.decrypt(new Uint8Array(32))).toThrow('nonce exhausted');
  });

  it('passes data through before it has a key', () => {
    const c = new CipherState();
    expect(c.encryptWithAd(new Uint8Array(0), utf8('plain'))).toEqual(utf8('plain'));
  });
});
