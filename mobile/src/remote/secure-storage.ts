import * as SecureStore from 'expo-secure-store';

import type { KeyStore } from './storage';

/** The Keychain (iOS) or the Keystore-backed store (Android); readable only while the phone is unlocked once. */
const options: SecureStore.SecureStoreOptions = { keychainAccessible: SecureStore.AFTER_FIRST_UNLOCK_THIS_DEVICE_ONLY };

export const secureStorage: KeyStore = {
  get: (key) => SecureStore.getItemAsync(key, options),
  set: (key, value) => SecureStore.setItemAsync(key, value, options),
  remove: (key) => SecureStore.deleteItemAsync(key, options),
};
