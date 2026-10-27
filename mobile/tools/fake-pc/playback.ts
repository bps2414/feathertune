// The made-up player behind the fake PC: a short queue of invented tracks that moves on by itself, with the
// controls the real page offers (docs/REMOTE.md, Protocol). No audio: only numbers.

export interface Track {
  id: string;
  title: string;
  artist: string;
  album: string;
  duration: number;
  /** `["#accent","#ink","#text on accent"]`, as the PC computes them from the cover. */
  colors: [string, string, string];
}

const TRACKS: Track[] = [
  ['Paper Lanterns', 'The Quiet Orchard', 'Slow Weather', 187, '#e0457b', '#1b1220', '#ffffff'],
  ['Low Tide Radio', 'Marlow & Finch', 'Harbor Lights', 212, '#2a9d8f', '#0e1f1e', '#ffffff'],
  ['Seventeen Windows', 'Ode to Hollis', 'Small Rooms', 154, '#f4a261', '#2a1a0c', '#1b1220'],
  ['Midnight Cartography', 'Vesper Lane', 'Maps of Nowhere', 241, '#5e60ce', '#12122b', '#ffffff'],
  ['Glass Garden', 'Nine Pines', 'Greenhouse', 98, '#8ac926', '#17210a', '#12210a'],
  ['Telegraph Hill', 'Juniper Wire', 'Static Bloom', 203, '#ff595e', '#2b0f10', '#ffffff'],
  ['A Long Way Home', 'Bright Cedar', 'Slow Weather', 176, '#1982c4', '#0a1b2a', '#ffffff'],
  ['Last Train Lullaby', 'Marlow & Finch', 'Harbor Lights', 265, '#6a4c93', '#1a1224', '#ffffff'],
].map(([title, artist, album, duration, ...colors], i) => ({
  id: `fakeTrk${String(i + 1).padStart(4, '0')}`,
  title: title as string,
  artist: artist as string,
  album: album as string,
  duration: duration as number,
  colors: colors as Track['colors'],
}));

/** The made-up library: each playlist is the tracks in its own order (`order` indexes TRACKS). */
const LIBRARY = [
  { list: 'LM', title: 'Liked music', order: [0, 1, 2, 3, 4, 5, 6, 7] },
  { list: 'PLfakeRoadTrip01', title: 'Road trip', order: [5, 3, 1, 7] },
  { list: 'PLfakeQuietHours', title: 'Quiet hours', order: [6, 0, 4, 2] },
];

/** The colors of a new track arrive a little after the track itself (docs/REMOTE.md, `state`). */
const COLORS_DELAY_MS = 600;
/** Like the site: Previous past this restarts the track instead of leaving it. */
const RESTART_AFTER = 3;

export class Playback {
  /** The queue, which the queue actions reorder and shorten. */
  tracks: Track[] = [...TRACKS];
  readonly library = LIBRARY.map(({ list, title }) => ({ list, title }));
  index = 0;
  paused = false;
  ad = false;
  volume = 70;
  muted = false;
  private position0 = 0;
  private at: number;
  private colorsAt = 0;

  constructor(
    private readonly clock: () => number = Date.now,
    private readonly artwork: string | null = null
  ) {
    this.at = clock();
  }

  get track(): Track {
    return this.tracks[this.index];
  }

  /** Seconds into the track now; it stands still while paused or during an ad. */
  position(): number {
    if (this.paused || this.ad) return this.position0;
    return Math.min(this.track.duration, this.position0 + (this.clock() - this.at) / 1000);
  }

  private rebase(): void {
    this.position0 = this.position();
    this.at = this.clock();
  }

  private go(index: number): void {
    this.index = (index + this.tracks.length) % this.tracks.length;
    this.position0 = 0;
    this.at = this.clock();
    this.colorsAt = this.at + COLORS_DELAY_MS;
  }

  play(): void {
    this.rebase();
    this.paused = false;
  }

  pause(): void {
    this.rebase();
    this.paused = true;
  }

  next(): void {
    this.go(this.index + 1);
  }

  previous(): void {
    if (this.position() > RESTART_AFTER) this.seek(0);
    else this.go(this.index - 1);
  }

  seek(seconds: number): void {
    this.position0 = Math.min(seconds, this.track.duration);
    this.at = this.clock();
  }

  setVolume(value: number): void {
    this.volume = Math.round(value);
    if (value > 0) this.muted = false;
  }

  /** The queue row `index` if it holds `id`, else the row of `id` closest to it (like the page); -1: none. */
  private row(id: string, index: number): number {
    let best = -1;
    this.tracks.forEach((t, i) => {
      if (t.id === id && (best < 0 || Math.abs(i - index) < Math.abs(best - index))) best = i;
    });
    return best;
  }

  playItem(id: string, index: number): void {
    const row = this.row(id, index);
    if (row >= 0) this.go(row);
  }

  /** Moved right after the playing track (the PC's "play next" moves the row, it leaves no copy behind). */
  queueNext(id: string, index: number): void {
    const from = this.row(id, index);
    if (from < 0 || from === this.index) return;
    const playing = this.track;
    const [moved] = this.tracks.splice(from, 1);
    this.tracks.splice(this.tracks.indexOf(playing) + 1, 0, moved);
    this.index = this.tracks.indexOf(playing);
  }

  /** Out of the queue; the playing track cannot be removed (the site's menu does not offer it). */
  queueRemove(id: string, index: number): void {
    const from = this.row(id, index);
    if (from < 0 || from === this.index) return;
    const playing = this.track;
    this.tracks.splice(from, 1);
    this.index = this.tracks.indexOf(playing);
  }

  /** Plays a playlist of the library from its first track; an unknown id changes nothing. */
  openPlaylist(list: string): void {
    const playlist = LIBRARY.find((p) => p.list === list);
    if (!playlist) return;
    this.tracks = playlist.order.map((i) => TRACKS[i]);
    this.go(0);
    this.paused = false;
  }

  setAd(on: boolean): void {
    this.rebase();
    this.ad = on;
  }

  /** Moves on at the end of a track; true when what a controller sees changed (a new track, its colors). */
  tick(): boolean {
    const wasWaiting = this.colorsAt > 0;
    if (!this.paused && !this.ad && this.position() >= this.track.duration) {
      this.go(this.index + 1);
      return true;
    }
    if (wasWaiting && this.clock() >= this.colorsAt) {
      this.colorsAt = 0;
      return true;
    }
    return false;
  }

  stateMessage() {
    const t = this.track;
    return {
      t: 'state',
      track: { id: t.id, title: t.title, artist: t.artist, album: t.album, artwork: this.artwork, duration: t.duration },
      position: this.position(),
      paused: this.paused,
      ad: this.ad,
      volume: this.volume,
      muted: this.muted,
      colors: this.colorsAt > 0 ? [] : t.colors,
      at: this.clock(),
    };
  }

  queueMessage() {
    return {
      t: 'queue',
      items: this.tracks.map(({ id, title, artist }) => ({ id, title, artist })),
      current: this.index,
    };
  }

  libraryMessage() {
    return { t: 'library', items: this.library };
  }
}
