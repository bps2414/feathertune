import { LinearGradient } from 'expo-linear-gradient';
import { Redirect, router } from 'expo-router';
import { useCallback, useState } from 'react';
import { Image, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';

import { t } from '../i18n';
import { Library } from '../now/library';
import { Player } from '../now/player';
import { QueueSheet } from '../now/queue';
import { remote, useRemote } from '../remote/app';
import { Button, Glyph, IconButton, mix, Notice, StatusDot, type, useAlbumColors, type Colors } from '../ui';

type Tab = 'player' | 'library';

/**
 * The connected PC: the player and the library as two tabs, the queue as a sheet over them. The background is
 * a gradient from the album's accent into its ink; the PC's name on top opens the PCs list.
 */
export default function Now() {
  const ready = useRemote((s) => s.ready);
  const paired = useRemote((s) => s.pcs.length > 0);
  const pc = useRemote((s) => s.pcs.find((p) => p.id === s.activeId) ?? null);
  const status = useRemote((s) => s.status);
  const problem = useRemote((s) => s.problem);
  const albumColors = useRemote((s) => s.player?.colors);
  const c = useAlbumColors(albumColors);

  const [tab, setTab] = useState<Tab>('player');
  const [queueOpen, setQueueOpen] = useState(false);
  const openQueue = useCallback(() => setQueueOpen(true), []);
  const closeQueue = useCallback(() => setQueueOpen(false), []);

  if (ready && !pc) return <Redirect href={paired ? '/pcs' : '/pair'} />;
  if (!pc) return null;

  return (
    <View style={[styles.fill, { backgroundColor: c.bg }]}>
      <LinearGradient colors={[mix(c.accent, c.bg, 0.62), c.bg]} locations={[0, 0.7]} style={StyleSheet.absoluteFill} />
      <SafeAreaView style={styles.fill} edges={['top', 'bottom']}>
        <View style={styles.topBar}>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={`${t('nowSwitchPc')}, ${pc.name}`}
            accessibilityHint={t('nowPcsHint')}
            hitSlop={8}
            onPress={() => router.push('/pcs')}
            style={({ pressed }) => [styles.pcPill, { backgroundColor: c.raised, opacity: pressed ? 0.7 : 1 }]}
          >
            <StatusDot status={status} problem={problem} colors={c} />
            <Text style={[styles.pcName, { color: c.text }]} numberOfLines={1}>
              {pc.name}
            </Text>
          </Pressable>
          <IconButton glyph="queue" label={t('nowQueue')} colors={c} onPress={openQueue} />
        </View>

        <ScrollView contentContainerStyle={styles.content}>
          {problem && <Notice problem={problem} colors={c} />}
          {problem === 'removed' && <Button label={t('pairAgain')} onPress={() => router.push('/pair')} colors={c} />}
          {tab === 'player' ? (
            <Player colors={c} onOpenQueue={openQueue} />
          ) : (
            <Library colors={c} onPlayed={() => setTab('player')} />
          )}
        </ScrollView>

        {tab === 'library' && <MiniPlayer colors={c} onOpen={() => setTab('player')} />}
        <View style={[styles.tabs, { borderTopColor: c.line, backgroundColor: c.bg }]} accessibilityRole="tablist">
          <TabButton label={t('nowTabPlayer')} glyph="playing" active={tab === 'player'} colors={c} onPress={() => setTab('player')} />
          <TabButton label={t('nowTabLibrary')} glyph="library" active={tab === 'library'} colors={c} onPress={() => setTab('library')} />
        </View>
      </SafeAreaView>
      <QueueSheet colors={c} open={queueOpen} onClose={closeQueue} />
    </View>
  );
}

function TabButton(props: { label: string; glyph: 'playing' | 'library'; active: boolean; colors: Colors; onPress: () => void }) {
  const { label, glyph, active, colors: c } = props;
  return (
    <Pressable
      accessibilityRole="tab"
      accessibilityLabel={label}
      accessibilityState={{ selected: active }}
      onPress={props.onPress}
      style={styles.tab}
    >
      <View style={[styles.tabIcon, active && { backgroundColor: mix(c.accent, c.bg, 0.7) }]}>
        <Glyph name={glyph} size={20} color={active ? c.text : c.muted} />
      </View>
      <Text style={[type.small, { color: active ? c.text : c.muted, fontWeight: active ? '700' : '500' }]}>{label}</Text>
    </Pressable>
  );
}

/** What plays, above the tabs while the library is open: tap it for the player, or play/pause from here. */
function MiniPlayer({ colors: c, onOpen }: { colors: Colors; onOpen: () => void }) {
  const player = useRemote((s) => s.player);
  if (!player) return null;
  const { track } = player;
  return (
    <View style={[styles.mini, { backgroundColor: c.raised }]}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={t('nowTrack', { title: track.title, artist: track.artist })}
        onPress={onOpen}
        style={styles.miniMain}
      >
        <View style={[styles.miniArt, { backgroundColor: c.line }]}>
          {track.artwork && <Image source={{ uri: track.artwork }} style={StyleSheet.absoluteFill} />}
        </View>
        <View style={styles.fill}>
          <Text style={[type.body, { color: c.text, fontWeight: '600' }]} numberOfLines={1}>
            {track.title}
          </Text>
          <Text style={[type.small, { color: c.muted }]} numberOfLines={1}>
            {track.artist}
          </Text>
        </View>
      </Pressable>
      <IconButton
        glyph={player.paused ? 'play' : 'pause'}
        label={t(player.paused ? 'nowPlay' : 'nowPause')}
        colors={c}
        onPress={() => remote.command({ c: player.paused ? 'play' : 'pause' })}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  topBar: { flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', paddingHorizontal: 20, paddingTop: 8 },
  pcPill: { flexDirection: 'row', alignItems: 'center', gap: 8, borderRadius: 20, paddingHorizontal: 14, paddingVertical: 8, maxWidth: '70%' },
  pcName: { fontSize: 14, fontWeight: '600' },
  content: { paddingHorizontal: 24, paddingTop: 12, paddingBottom: 24, gap: 16, flexGrow: 1 },
  tabs: { flexDirection: 'row', borderTopWidth: StyleSheet.hairlineWidth, paddingTop: 6, paddingBottom: 4 },
  tab: { flex: 1, alignItems: 'center', gap: 2, minHeight: 56, justifyContent: 'center' },
  tabIcon: { width: 56, height: 30, borderRadius: 15, alignItems: 'center', justifyContent: 'center' },
  mini: { flexDirection: 'row', alignItems: 'center', marginHorizontal: 12, marginBottom: 8, borderRadius: 14, paddingLeft: 8, paddingRight: 4 },
  miniMain: { flex: 1, flexDirection: 'row', alignItems: 'center', gap: 12, paddingVertical: 8 },
  miniArt: { width: 44, height: 44, borderRadius: 6, overflow: 'hidden' },
});
