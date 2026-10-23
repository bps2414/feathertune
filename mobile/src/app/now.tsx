import { Redirect, router } from 'expo-router';
import { useEffect, useState } from 'react';
import { Image, Pressable, StyleSheet, Text, View } from 'react-native';

import { t } from '../i18n';
import { remote, useRemote } from '../remote/app';
import {
  Body,
  Busy,
  Button,
  clock,
  Glyph,
  IconButton,
  Notice,
  Screen,
  Slider,
  StatusDot,
  statusText,
  Title,
  type,
  useAlbumColors,
  useForeground,
} from '../ui';

/**
 * Now playing screen:
 * - Dynamic album colors for background and highlights.
 * - Artwork with graceful fallback.
 * - Progress bar with live ticker, drag with haptics, and single seek on release.
 * - Transport controls (play/pause, previous, next) and volume slider.
 * - Ad awareness: controls disabled/muted with informative banner, only play/pause allowed.
 * - Connected PC header with quick access to the PCs switcher.
 * - Integrated Queue view with active item highlight and play_item dispatch.
 */
export default function Now() {
  const ready = useRemote((s) => s.ready);
  const paired = useRemote((s) => s.pcs.length > 0);
  const pc = useRemote((s) => s.pcs.find((p) => p.id === s.activeId) ?? null);
  const status = useRemote((s) => s.status);
  const problem = useRemote((s) => s.problem);
  const player = useRemote((s) => s.player);
  const queue = useRemote((s) => s.queue);
  const clockOffset = useRemote((s) => s.clockOffset);

  const colors = useAlbumColors(player?.colors);
  const foreground = useForeground();

  const [showQueue, setShowQueue] = useState(false);
  const [dragFraction, setDragFraction] = useState<number | null>(null);
  const [nowMs, setNowMs] = useState(Date.now);

  // Position ticker: updates smoothly while playing in the foreground and not dragging.
  useEffect(() => {
    if (!foreground || !player || player.paused || dragFraction !== null) return;
    const interval = setInterval(() => setNowMs(Date.now()), 250);
    return () => clearInterval(interval);
  }, [foreground, player, dragFraction]);

  if (ready && !pc) return <Redirect href={paired ? '/pcs' : '/pair'} />;
  if (!pc) return null;

  const track = player?.track ?? null;
  const duration = track?.duration ?? 0;

  // Calculate current elapsed seconds
  let currentSec = 0;
  if (player && track) {
    if (player.paused) {
      currentSec = Math.min(duration || Infinity, Math.max(0, player.position));
    } else {
      const elapsed = Math.max(0, (nowMs + clockOffset - player.at) / 1000);
      currentSec = Math.min(duration || Infinity, Math.max(0, player.position + elapsed));
    }
  }

  const shownSec = dragFraction !== null ? dragFraction * duration : currentSec;
  const progressFraction = duration > 0 ? Math.min(1, Math.max(0, shownSec / duration)) : 0;
  const isAd = player?.ad ?? false;

  return (
    <Screen colors={colors} scroll={!showQueue}>
      {/* Top Bar: Connected PC on the left, Queue toggle on the right */}
      <View style={styles.topBar}>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={`${t('nowSwitchPc')}, ${pc.name}`}
          accessibilityHint={t('nowPcsHint')}
          hitSlop={8}
          onPress={() => router.push('/pcs')}
          style={({ pressed }) => [
            styles.pcPill,
            { backgroundColor: colors.raised, borderColor: colors.line, opacity: pressed ? 0.7 : 1 },
          ]}
        >
          <StatusDot status={status} problem={problem} colors={colors} />
          <Text style={[styles.pcName, { color: colors.text }]} numberOfLines={1}>
            {pc.name}
          </Text>
        </Pressable>

        <IconButton
          glyph={showQueue ? 'down' : 'queue'}
          label={t(showQueue ? 'nowCloseQueue' : 'nowQueue')}
          colors={colors}
          onPress={() => setShowQueue(!showQueue)}
        />
      </View>

      {/* Non-connected states */}
      {status === 'connecting' && <Busy label={statusText(status)} colors={colors} />}
      {problem && <Notice problem={problem} colors={colors} />}
      {problem === 'removed' && <Button label={t('pairAgain')} onPress={() => router.push('/pair')} colors={colors} />}

      {/* Main Content: Queue or Now Playing */}
      {showQueue ? (
        <View style={styles.queueContainer}>
          <View style={styles.queueHeader}>
            <Title colors={colors}>{t('nowQueue')}</Title>
            {queue?.items && (
              <Text style={[type.small, { color: colors.muted }]}>
                {queue.items.length}
              </Text>
            )}
          </View>

          {queue?.items && queue.items.length > 0 ? (
            <View style={styles.queueList}>
              {queue.items.map((item, index) => {
                const isCurrent = index === queue.current;
                return (
                  <Pressable
                    key={`${item.id}-${index}`}
                    accessibilityRole="button"
                    accessibilityLabel={`${index + 1}. ${item.title}, ${item.artist}`}
                    accessibilityState={{ disabled: isAd, selected: isCurrent }}
                    disabled={isAd}
                    onPress={() => remote.command({ c: 'play_item', id: item.id, i: index })}
                    style={({ pressed }) => [
                      styles.queueRow,
                      {
                        backgroundColor: isCurrent ? colors.raised : 'transparent',
                        borderColor: isCurrent ? colors.line : 'transparent',
                        opacity: isAd ? 0.4 : pressed ? 0.7 : 1,
                      },
                    ]}
                  >
                    <View style={styles.queueIndex}>
                      {isCurrent ? (
                        <Glyph name="playing" size={16} color={colors.accent} />
                      ) : (
                        <Text style={[type.small, type.digits, { color: colors.muted }]}>{index + 1}</Text>
                      )}
                    </View>
                    <View style={styles.queueTrackInfo}>
                      <Text
                        style={[
                          type.body,
                          { color: isCurrent ? colors.accent : colors.text, fontWeight: isCurrent ? '700' : '500' },
                        ]}
                        numberOfLines={1}
                      >
                        {item.title}
                      </Text>
                      <Text style={[type.small, { color: colors.muted }]} numberOfLines={1}>
                        {item.artist}
                      </Text>
                    </View>
                  </Pressable>
                );
              })}
            </View>
          ) : (
            <Body muted colors={colors}>
              {t('nowEmptyQueue')}
            </Body>
          )}
        </View>
      ) : (
        <View style={styles.playerContainer}>
          {/* Large Cover Art with fallback */}
          <View style={[styles.artworkFrame, { borderColor: colors.line, backgroundColor: colors.raised }]}>
            {track?.artwork ? (
              <Image source={{ uri: track.artwork }} style={styles.artwork} resizeMode="cover" />
            ) : (
              <View style={styles.artworkFallback}>
                <Glyph name="playing" size={56} color={colors.accent} />
              </View>
            )}
          </View>

          {/* Track Title and Artist */}
          <View style={styles.metaContainer} accessible accessibilityLabel={track ? t('nowTrack', { title: track.title, artist: track.artist }) : t('nowNothing')}>
            {track ? (
              <>
                <Text style={[type.title, { color: colors.text }]} numberOfLines={2}>
                  {track.title}
                </Text>
                <Text style={[type.heading, { color: colors.muted }]} numberOfLines={1}>
                  {track.artist}
                </Text>
              </>
            ) : (
              <Body muted colors={colors}>
                {t('nowNothing')}
              </Body>
            )}
          </View>

          {/* Ad Banner: shown while an ad is playing */}
          {isAd && (
            <View style={[styles.adNotice, { backgroundColor: colors.raised, borderLeftColor: colors.warn }]}>
              <Text style={[type.small, { color: colors.warn, fontWeight: '600' }]}>{t('nowAdNote')}</Text>
            </View>
          )}

          {/* Seek / Progress Bar */}
          <View style={styles.progressContainer}>
            <Slider
              value={progressFraction}
              disabled={isAd || duration <= 0}
              thick
              ticks={30}
              colors={colors}
              label={t('nowSeek')}
              valueText={`${clock(shownSec)} / ${clock(duration)}`}
              onChange={(f) => setDragFraction(f)}
              onGrab={(grabbed) => !grabbed && setDragFraction(null)}
              onCommit={(f) => {
                setDragFraction(null);
                remote.command({ c: 'seek', s: Math.round(f * duration) });
              }}
              onStep={(dir) => {
                const target = Math.max(0, Math.min(duration, currentSec + dir * 5));
                remote.command({ c: 'seek', s: Math.round(target) });
              }}
            />
            <View style={styles.timeRow}>
              <Text style={[type.small, type.digits, { color: colors.muted }]}>{clock(shownSec)}</Text>
              <Text style={[type.small, type.digits, { color: colors.muted }]}>{clock(duration)}</Text>
            </View>
          </View>

          {/* Playback Controls: Previous, Play/Pause, Next */}
          <View style={styles.controlsRow}>
            <IconButton
              glyph="previous"
              label={t('nowPrevious')}
              colors={colors}
              disabled={isAd}
              onPress={() => remote.command({ c: 'previous' })}
              size={56}
            />
            <IconButton
              glyph={player?.paused ? 'play' : 'pause'}
              label={t(player?.paused ? 'nowPlay' : 'nowPause')}
              colors={colors}
              filled
              onPress={() => remote.command({ c: player?.paused ? 'play' : 'pause' })}
              size={72}
            />
            <IconButton
              glyph="next"
              label={t('nowNext')}
              colors={colors}
              disabled={isAd}
              onPress={() => remote.command({ c: 'next' })}
              size={56}
            />
          </View>

          {/* Volume Control */}
          <View style={styles.volumeRow}>
            <Glyph name="speaker" size={20} color={isAd ? colors.muted : colors.text} />
            <View style={styles.volumeSlider}>
              <Slider
                value={(player?.volume ?? 50) / 100}
                disabled={isAd}
                thick={false}
                ticks={20}
                colors={colors}
                label={t('nowVolume')}
                valueText={`${player?.volume ?? 50}%`}
                onCommit={(f) => remote.command({ c: 'volume', v: Math.round(f * 100) })}
                onStep={(dir) => {
                  const currentVol = player?.volume ?? 50;
                  const v = Math.max(0, Math.min(100, currentVol + dir * 5));
                  remote.command({ c: 'volume', v });
                }}
              />
            </View>
          </View>
        </View>
      )}
    </Screen>
  );
}

const styles = StyleSheet.create({
  topBar: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    marginBottom: 8,
  },
  pcPill: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 8,
    borderWidth: 1,
    borderRadius: 20,
    paddingHorizontal: 14,
    paddingVertical: 7,
    maxWidth: '70%',
  },
  pcName: {
    fontSize: 14,
    fontWeight: '600',
  },
  playerContainer: {
    gap: 20,
    alignItems: 'center',
    width: '100%',
  },
  artworkFrame: {
    width: '100%',
    maxWidth: 340,
    aspectRatio: 1,
    borderRadius: 20,
    borderWidth: 1,
    overflow: 'hidden',
  },
  artwork: {
    width: '100%',
    height: '100%',
  },
  artworkFallback: {
    flex: 1,
    alignItems: 'center',
    justifyContent: 'center',
  },
  metaContainer: {
    width: '100%',
    alignItems: 'flex-start',
    gap: 4,
  },
  adNotice: {
    width: '100%',
    padding: 12,
    borderRadius: 12,
    borderLeftWidth: 4,
  },
  progressContainer: {
    width: '100%',
    gap: 4,
  },
  timeRow: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    paddingHorizontal: 2,
  },
  controlsRow: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'center',
    gap: 32,
    width: '100%',
    marginVertical: 4,
  },
  volumeRow: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 14,
    width: '100%',
    paddingHorizontal: 8,
  },
  volumeSlider: {
    flex: 1,
  },
  queueContainer: {
    width: '100%',
    gap: 16,
  },
  queueHeader: {
    flexDirection: 'row',
    alignItems: 'baseline',
    justifyContent: 'space-between',
  },
  queueList: {
    gap: 8,
    width: '100%',
  },
  queueRow: {
    flexDirection: 'row',
    alignItems: 'center',
    padding: 12,
    borderRadius: 14,
    borderWidth: 1,
    gap: 12,
  },
  queueIndex: {
    width: 24,
    alignItems: 'center',
    justifyContent: 'center',
  },
  queueTrackInfo: {
    flex: 1,
    gap: 2,
  },
});
