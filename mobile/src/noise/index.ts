// Noise_IKpsk2 and Noise_KK over 25519, ChaChaPoly and SHA256 (docs/REMOTE.md, Security model).
// Randomness is injected: `expoRandom` (./random) in the app, Node's crypto in tests.

export { CipherState, generateKeyPair, MAX_MESSAGE, type KeyPair, type RandomBytes } from './cipher';
export { createHandshake, Handshake, type HandshakeOptions, type Pattern, type Transport } from './handshake';
