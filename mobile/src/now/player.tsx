// The player tab: artwork, title, seek bar, transport, a live volume slider and the "up next" bar that opens the
// queue. Seek sends one command on release; volume follows the finger (a few commands a second) and the slider
// keeps the value it was dragged to until the PC's state says the same, since the PC reports volume late.

import { useEffect, useRef, useState } from 'react';
import { ActivityIndicator, Image, Pressable, StyleSheet, Text, View } from 'react-native';

import { t } from '../i18n';
import { remote, useRemote } from '../remote/app';
import { clock, Glyph, IconButton, Slider, type, useForeground, type Colors } from '../ui';

/** At most one volume command this often while dragging; the last value always goes. */
const VOLUME_EVERY_MS = 80;
/** How long the dragged volume is shown when the PC's state does not catch up. */
const VOLUME_HOLD_MS = 1500;

export function Player({ colors: c, onOpenQueue }: { colors: Colors; onOpenQueue: () => void }) {
  const pc = useRemote((s) => s.pcs.find((p) => p.id === s.activeId) ?? null);
  const status = useRemote((s) => s.status);
  const player = useRemote((s) => s.player);
  const queue = useRemote((s) => s.queue);
  const clockOffset = useRemote((s) => s.clockOffset);
  const foreground = useForeground();

  const [dragFraction, setDragFraction] = useState<number | null>(null);
  const [nowMs, setNowMs] = useState(Date.now);
  const volume = useLiveVolume(player?.volume ?? null);

  // Position ticker: only while playing in the foreground and not dragging.
  useEffect(() => {
    if (!foreground || !player || player.paused || dragFraction !== null) return;
    const interval = setInterval(() => setNowMs(Date.now()), 250);
    return () => clearInterval(interval);
  }, [foreground, player, dragFraction]);

  if (!player) {
    return (
      <View style={styles.waiting}>
        <View style={[styles.artwork, styles.center, { backgroundColor: c.raised }]}>
          {status === 'connecting' ? <ActivityIndicator color={c.accent} /> : <Glyph name="playing" size={56} color={c.muted} />}
        </View>
        <Text style={[type.heading, { color: c.muted, textAlign: 'center' }]}>
          {status === 'connecting' && pc ? t('nowConnecting', { name: pc.name }) : t('nowNothing')}
        </Text>
      </View>
    );
  }

  const { track } = player;
  const duration = track.duration;
  const elapsed = player.paused ? 0 : Math.max(0, (nowMs + clockOffset - player.at) / 1000);
  const currentSec = Math.min(duration || Infinity, Math.max(0, player.position + elapsed));
  const shownSec = dragFraction !== null ? dragFraction * duration : currentSec;
  const progress = duration > 0 ? Math.min(1, Math.max(0, shownSec / duration)) : 0;
  const isAd = player.ad;
  const next = queue && queue.current !== null ? queue.items[queue.current + 1] : undefined;

  return (
    <View style={styles.player}>
      <View style={[styles.artwork, { backgroundColor: c.raised }]}>
        {track.artwork ? (
          <Image source={{ uri: track.artwork }} style={StyleSheet.absoluteFill} resizeMode="cover" />
        ) : (
          <View style={[StyleSheet.absoluteFill, styles.center]}>
            <Glyph name="playing" size={56} color={c.accent} />
          </View>
        )}
      </View>

      <View style={styles.meta} accessible accessibilityLabel={t('nowTrack', { title: track.title, artist: track.artist })}>
        <Text style={[type.title, { color: c.text }]} numberOfLines={1}>
          {track.title}
        </Text>
        <Text style={[type.body, { color: c.muted }]} numberOfLines={1}>
          {track.album ? `${track.artist} · ${track.album}` : track.artist}
        </Text>
      </View>

      {isAd && (
        <View style={[styles.adNotice, { backgroundColor: c.raised, borderLeftColor: c.warn }]} accessibilityLiveRegion="polite">
          <Text style={[type.small, { color: c.warn, fontWeight: '600' }]}>{t('nowAdNote')}</Text>
        </View>
      )}

      <View style={styles.progress}>
        <Slider
          value={progress}
          disabled={isAd || duration <= 0}
          thick
          ticks={30}
          colors={c}
          label={t('nowSeek')}
          valueText={`${clock(shownSec)} / ${clock(duration)}`}
          onChange={setDragFraction}
          onGrab={(grabbed) => !grabbed && setDragFraction(null)}
          onCommit={(f) => {
            setDragFraction(null);
            remote.command({ c: 'seek', s: Math.round(f * duration) });
          }}
          onStep={(dir) => remote.command({ c: 'seek', s: Math.round(Math.max(0, Math.min(duration, currentSec + dir * 5))) })}
        />
        <View style={styles.times}>
          <Text style={[type.small, type.digits, { color: c.muted }]}>{clock(shownSec)}</Text>
          <Text style={[type.small, type.digits, { color: c.muted }]}>{clock(duration)}</Text>
        </View>
      </View>

      <View style={styles.controls}>
        <IconButton glyph="previous" label={t('nowPrevious')} colors={c} disabled={isAd} onPress={() => remote.command({ c: 'previous' })} size={60} />
        <IconButton
          glyph={player.paused ? 'play' : 'pause'}
          label={t(player.paused ? 'nowPlay' : 'nowPause')}
          colors={c}
          filled
          onPress={() => remote.command({ c: player.paused ? 'play' : 'pause' })}
          size={78}
        />
        <IconButton glyph="next" label={t('nowNext')} colors={c} disabled={isAd} onPress={() => remote.command({ c: 'next' })} size={60} />
      </View>

      <View style={styles.volume}>
        <Glyph name="speaker" size={18} color={isAd ? c.muted : c.text} />
        <View style={styles.grow}>
          <Slider
            value={volume.shown / 100}
            disabled={isAd}
            ticks={20}
            colors={c}
            label={t('nowVolume')}
            valueText={`${volume.shown}%`}
            onChange={(f) => volume.set(Math.round(f * 100), false)}
            onCommit={(f) => volume.set(Math.round(f * 100), true)}
            onStep={(dir) => volume.set(Math.max(0, Math.min(100, volume.shown + dir * 5)), true)}
          />
        </View>
        <Text style={[type.small, type.digits, styles.volumeValue, { color: c.muted }]}>{volume.shown}</Text>
      </View>

      <Pressable
        accessibilityRole="button"
        accessibilityLabel={next ? `${t('nowUpNext')}: ${next.title}, ${next.artist}` : t('nowQueue')}
        accessibilityHint={t('nowOpenQueueHint')}
        onPress={onOpenQueue}
        style={({ pressed }) => [styles.upNext, { backgroundColor: c.raised, opacity: pressed ? 0.7 : 1 }]}
      >
        <Glyph name="queue" size={18} color={c.accent} />
        <View style={styles.grow}>
          <Text style={[type.small, { color: c.muted, fontWeight: '600' }]}>{next ? t('nowUpNext') : t('nowQueue')}</Text>
          {next && (
            <Text style={[type.body, { color: c.text }]} numberOfLines={1}>
              {next.title} · {next.artist}
            </Text>
          )}
        </View>
        <View style={{ transform: [{ rotate: '180deg' }] }}>
          <Glyph name="down" size={16} color={c.muted} />
        </View>
      </Pressable>
    </View>
  );
}

/**
 * The volume the slider shows and how to change it: `set` sends at most one command every VOLUME_EVERY_MS while
 * dragging and the value itself when `final`; the set value is shown until the PC reports it (or a moment passes).
 */
function useLiveVolume(reported: number | null) {
  const [pending, setPending] = useState<number | null>(null);
  const sent = useRef({ value: -1, at: 0, latest: -1, timer: null as ReturnType<typeof setTimeout> | null });

  // A moment after the last change the PC has caught up (or is not going to): show what it says again.
  useEffect(() => {
    if (pending === null) return;
    const hold = setTimeout(() => setPending(null), VOLUME_HOLD_MS);
    return () => clearTimeout(hold);
  }, [pending]);

  useEffect(() => () => {
    if (sent.current.timer) clearTimeout(sent.current.timer);
  }, []);

  const send = (v: number) => {
    const s = sent.current;
    if (s.timer) clearTimeout(s.timer);
    s.timer = null;
    s.at = Date.now();
    if (v === s.value) return;
    s.value = v;
    remote.command({ c: 'volume', v });
  };

  const set = (v: number, final: boolean) => {
    setPending(v);
    const s = sent.current;
    s.latest = v;
    const wait = VOLUME_EVERY_MS - (Date.now() - s.at);
    if (final || wait <= 0) return send(v);
    s.timer ??= setTimeout(() => send(s.latest), wait);
  };

  return { shown: pending ?? reported ?? 50, set };
}

const styles = StyleSheet.create({
  player: { gap: 18, width: '100%' },
  waiting: { gap: 20, width: '100%', alignItems: 'center', paddingTop: 12 },
  center: { alignItems: 'center', justifyContent: 'center' },
  artwork: {
    width: '100%',
    maxWidth: 360,
    aspectRatio: 1,
    alignSelf: 'center',
    borderRadius: 14,
    overflow: 'hidden',
    elevation: 12,
    shadowColor: '#000',
    shadowOpacity: 0.35,
    shadowRadius: 18,
    shadowOffset: { width: 0, height: 10 },
  },
  meta: { gap: 2 },
  adNotice: { padding: 12, borderRadius: 12, borderLeftWidth: 4 },
  progress: { gap: 2 },
  times: { flexDirection: 'row', justifyContent: 'space-between', paddingHorizontal: 2 },
  controls: { flexDirection: 'row', alignItems: 'center', justifyContent: 'space-evenly' },
  volume: { flexDirection: 'row', alignItems: 'center', gap: 12 },
  volumeValue: { minWidth: 28, textAlign: 'right' },
  grow: { flex: 1 },
  upNext: { flexDirection: 'row', alignItems: 'center', gap: 12, borderRadius: 14, paddingHorizontal: 16, paddingVertical: 12, minHeight: 56 },
});
