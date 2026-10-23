import { getRandomBytes } from 'expo-crypto';

import type { RandomBytes } from './cipher';

/** The platform's secure random source, for the app. Kept apart so the protocol code runs in Node tests. */
export const expoRandom: RandomBytes = (length) => getRandomBytes(length);
