// The library tab: the playlists of the site's sidebar on the PC (version 2 PCs send them); a tap plays one there
// (`open_playlist`) and goes back to the player.

import * as Haptics from 'expo-haptics';
import { Pressable, StyleSheet, Text, View } from 'react-native';

import { t } from '../i18n';
import { remote, useRemote } from '../remote/app';
import { hasQueueActions } from '../remote/protocol';
import { Body, Glyph, mix, Title, type, type Colors } from '../ui';

export function Library({ colors: c, onPlayed }: { colors: Colors; onPlayed: () => void }) {
  const v2 = useRemote((s) => hasQueueActions(s.hello));
  const connected = useRemote((s) => s.status === 'connected');
  const library = useRemote((s) => s.library);
  const isAd = useRemote((s) => s.player?.ad ?? false);

  return (
    <View style={styles.library}>
      <Title colors={c}>{t('libraryTitle')}</Title>
      {isAd && <Text style={[type.small, { color: c.warn }]}>{t('nowAdNote')}</Text>}
      {connected && !v2 ? (
        <Body muted colors={c}>
          {t('queueUpdatePc')}
        </Body>
      ) : library && library.length === 0 ? (
        <Body muted colors={c}>
          {t('libraryEmpty')}
        </Body>
      ) : (
        (library ?? []).map((p) => (
          <Pressable
            key={p.list}
            accessibilityRole="button"
            accessibilityLabel={p.title}
            accessibilityHint={t('libraryPlayHint')}
            accessibilityState={{ disabled: isAd }}
            disabled={isAd}
            onPress={() => {
              void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
              remote.command({ c: 'open_playlist', list: p.list });
              onPlayed();
            }}
            style={({ pressed }) => [styles.row, { opacity: isAd ? 0.4 : pressed ? 0.7 : 1 }]}
          >
            <Tile title={p.title} colors={c} />
            <Text style={[type.body, styles.grow, { color: c.text, fontWeight: '500' }]} numberOfLines={2}>
              {p.title}
            </Text>
            <Glyph name="play" size={16} color={c.muted} />
          </Pressable>
        ))
      )}
    </View>
  );
}

/** A square standing in for the playlist's cover (the PC sends no artwork for it): its initial on a tint of its own. */
function Tile({ title, colors: c }: { title: string; colors: Colors }) {
  let hash = 0;
  for (const ch of title) hash = (hash * 31 + ch.charCodeAt(0)) >>> 0;
  const hue = TINTS[hash % TINTS.length];
  return (
    <View style={[styles.tile, { backgroundColor: mix(hue, c.bg, 0.55) }]}>
      <Text style={[type.heading, { color: c.text }]}>{[...title.trim()][0]?.toUpperCase() ?? '♪'}</Text>
    </View>
  );
}

const TINTS = ['#5b4bdb', '#2a9d8f', '#e0457b', '#f4a261', '#1982c4', '#8ac926', '#6a4c93', '#ff595e'];

const styles = StyleSheet.create({
  library: { gap: 10, width: '100%' },
  row: { flexDirection: 'row', alignItems: 'center', gap: 14, paddingVertical: 6, minHeight: 64 },
  tile: { width: 52, height: 52, borderRadius: 8, alignItems: 'center', justifyContent: 'center' },
  grow: { flex: 1 },
});
