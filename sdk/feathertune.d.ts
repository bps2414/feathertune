// Types of `ft`, the API a feathertune extension's page.js gets (docs/EXTENSIONS.md, Page code).
// In page.js: /// <reference path="feathertune.d.ts" />

declare namespace Feathertune {
  type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

  interface PlayerState {
    id: string;
    title: string;
    artist: string;
    album: string;
    artwork: string | null;
    /** Seconds; 0 while unknown. */
    duration: number;
    /** Seconds. */
    position: number;
    paused: boolean;
    /** 0 to 100. */
    volume: number;
    muted: boolean;
    ad: boolean;
    list: string | null;
  }

  interface QueueItem {
    id: string;
    title: string;
    artist: string;
    [key: string]: unknown;
  }

  interface Queue {
    queue: QueueItem[];
    current: unknown;
    next: unknown;
    list: string | null;
  }

  interface Events {
    state: PlayerState;
    track: PlayerState;
    play: PlayerState;
    pause: PlayerState;
    seek: PlayerState;
    ad: boolean;
    queue: Queue;
    lyrics: unknown;
    colors: unknown;
    settings: Readonly<Record<string, Json>>;
    native: Json;
  }

  interface Response {
    status: number;
    headers: Record<string, string>;
    body: string;
  }

  interface Player {
    state(): Readonly<PlayerState> | null;
    queue(): Queue;
    video(): HTMLVideoElement | null;
    clock: { time(video: HTMLVideoElement): number };
    // With the `player.control` permission; false when refused (during an ad, or with no player).
    play?(): boolean;
    pause?(): boolean;
    playPause?(): boolean;
    next?(): boolean;
    previous?(): boolean;
    seek?(seconds: number): boolean;
    setVolume?(percent: number): boolean;
    volumeUp?(): boolean;
    volumeDown?(): boolean;
  }

  interface Api {
    readonly api: 1;
    readonly id: string;
    readonly version: string;
    readonly permissions: readonly string[];
    readonly settings: Readonly<Record<string, Json>>;
    on<K extends keyof Events>(event: K, fn: (data: Events[K]) => void): () => void;
    onSettings(fn: (settings: Events['settings']) => void): () => void;
    addStyle(css: string): HTMLStyleElement;
    log(...args: unknown[]): void;
    error(e: unknown): void;
    readonly player: Player;
    readonly theme: { colors(): unknown };
    readonly lyrics: { current(): unknown };
    /** Permission `storage`. */
    readonly storage: {
      get(key: string): Promise<Json>;
      set(key: string, value: Json): Promise<null>;
      remove(key: string): Promise<null>;
      keys(): Promise<string[]>;
    };
    /** https, to the manifest's `net:` hosts. */
    fetch(
      url: string,
      options?: { method?: string; headers?: Record<string, string>; body?: string },
    ): Promise<Response>;
    /** Permission `notify`. */
    notify(title: string, body?: string): Promise<null>;
    readonly native: { readonly available: boolean; send(data: Json): Promise<null> };
  }
}

declare const ft: Feathertune.Api;
