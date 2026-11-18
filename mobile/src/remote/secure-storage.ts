import * as SecureStore from 'expo-secure-store';

import type { KeyStore } from './storage';

/** The Keychain (iOS) or the Keystore-backed store (Android); readable only while the phone is unlocked once. */
const options: SecureStore.SecureStoreOptions = { keychainAccessible: SecureStore.AFTER_FIRST_UNLOCK_THIS_DEVICE_ONLY };

export const secureStorage: KeyStore = {
  get: async (key) => {
    try {
      return await SecureStore.getItemAsync(key, options);
    } catch {
      try {
        await SecureStore.deleteItemAsync(key, options);
      } catch {
        // ignore broken entry cleanup failure
      }
      return null;
    }
  },
  set: async (key, value) => {
    try {
      await SecureStore.setItemAsync(key, value, options);
    } catch {
      // keep running in memory if keystore is temporarily locked
    }
  },
  remove: async (key) => {
    try {
      await SecureStore.deleteItemAsync(key, options);
    } catch {
      // ignore missing/locked keystore entry
    }
  },
};
