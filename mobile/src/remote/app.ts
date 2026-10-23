// The app's one Remote, on the platform's pieces: the secure store, React Native's WebSocket, expo-crypto and
// expo-network. The screens read it through useRemote().

import * as Network from 'expo-network';
import { AppState } from 'react-native';
import { useStore } from 'zustand';

import { expoRandom } from '../noise/random';
import type { SocketLike } from './client';
import { Remote, type NetworkInfo, type RemoteState } from './remote';
import { secureStorage } from './secure-storage';

async function network(): Promise<NetworkInfo> {
  const { type, isConnected } = await Network.getNetworkStateAsync();
  const vpn = type === Network.NetworkStateType.VPN;
  const lan = isConnected !== false && (type === Network.NetworkStateType.WIFI || type === Network.NetworkStateType.ETHERNET || vpn);
  const ip = lan ? await Network.getIpAddressAsync().catch(() => null) : null;
  return { lan, vpn, ip: ip && /^\d+\.\d+\.\d+\.\d+$/.test(ip) && ip !== '0.0.0.0' ? ip : null };
}

export const remote = new Remote({
  storage: secureStorage,
  openSocket: (url) => new WebSocket(url) as unknown as SocketLike,
  random: expoRandom,
  network,
});

export function useRemote<T>(selector: (state: RemoteState) => T): T {
  return useStore(remote.store, selector);
}

let started = false;

/** Loads the paired PCs and follows the app in and out of the foreground. Once per app run. */
export function startRemote(): void {
  if (started) return;
  started = true;
  void remote.load();
  // `inactive` (iOS: a system sheet over the app) is still the foreground.
  AppState.addEventListener('change', (state) => remote.setForeground(state !== 'background'));
}
