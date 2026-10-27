// The queue as a sheet over the player. Tap a row to play it; with a version 2 PC, open its ⋯ for Play next and
// Remove (`queue_next`, `queue_remove`). There is no drag: the page did not take one it did not see from a mouse.

import * as Haptics from 'expo-haptics';
import { useEffect, useState } from 'react';
import { Animated, BackHandler, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

import { t } from '../i18n';
import { remote, useRemote } from '../remote/app';
import { hasQueueActions, type QueueItem } from '../remote/protocol';
import { Body, Glyph, IconButton, type, useReducedMotion, type Colors } from '../ui';

const SHEET_MS = 260;

export function QueueSheet({ colors: c, open, onClose }: { colors: Colors; open: boolean; onClose: () => void }) {
  const reduced = useReducedMotion();
  const [mounted, setMounted] = useState(open);
  const [anim] = useState(() => new Animated.Value(open ? 1 : 0));
  const insets = useSafeAreaInsets();

  // Mounted on open, and kept until the closing slide ends.
  if (open && !mounted) setMounted(true);

  useEffect(() => {
    Animated.timing(anim, { toValue: open ? 1 : 0, duration: reduced ? 0 : SHEET_MS, useNativeDriver: true }).start(
      ({ finished }) => finished && !open && setMounted(false)
    );
  }, [open, reduced, anim]);

  useEffect(() => {
    if (!open) return;
    const sub = BackHandler.addEventListener('hardwareBackPress', () => (onClose(), true));
    return () => sub.remove();
  }, [open, onClose]);

  if (!mounted) return null;
  return (
    <View style={StyleSheet.absoluteFill} pointerEvents={open ? 'auto' : 'none'}>
      <Animated.View style={[StyleSheet.absoluteFill, styles.backdrop, { opacity: anim }]}>
        <Pressable style={StyleSheet.absoluteFill} onPress={onClose} accessibilityLabel={t('nowCloseQueue')} />
      </Animated.View>
      <Animated.View
        accessibilityViewIsModal
        style={[
          styles.sheet,
          {
            backgroundColor: c.bg,
            borderColor: c.line,
            paddingBottom: insets.bottom,
            transform: [{ translateY: anim.interpolate({ inputRange: [0, 1], outputRange: [900, 0] }) }],
          },
        ]}
      >
        <QueueList colors={c} onClose={onClose} />
      </Animated.View>
    </View>
  );
}

function QueueList({ colors: c, onClose }: { colors: Colors; onClose: () => void }) {
  const queue = useRemote((s) => s.queue);
  const v2 = useRemote((s) => hasQueueActions(s.hello));
  const isAd = useRemote((s) => s.player?.ad ?? false);
  const refusal = useRemote((s) => s.refusal);
  const [menu, setMenu] = useState<number | null>(null);

  const items = queue?.items ?? [];
  const current = queue?.current ?? null;
  const canEdit = v2 && !isAd;

  const note = !v2 ? t('queueUpdatePc') : isAd ? t('nowAdNote') : refusal === 'unavailable' ? t('nowUnavailable') : null;

  return (
    <>
      <View style={styles.header}>
        <View style={[styles.grabber, { backgroundColor: c.line }]} />
        <View style={styles.headerRow}>
          <View style={styles.grow}>
            <Text accessibilityRole="header" style={[type.title, { color: c.text }]}>
              {t('nowQueue')}
            </Text>
            {items.length > 0 && <Text style={[type.small, { color: c.muted }]}>{t('queueCount', { n: items.length })}</Text>}
          </View>
          <IconButton glyph="down" label={t('nowCloseQueue')} colors={c} onPress={onClose} />
        </View>
        {note && (
          <Text style={[type.small, { color: c.muted }]} accessibilityLiveRegion="polite">
            {note}
          </Text>
        )}
      </View>
      <ScrollView contentContainerStyle={styles.list}>
        {items.length === 0 && <Body muted colors={c}>{t('nowEmptyQueue')}</Body>}
        {items.map((item, index) => (
          <Row
            key={`${item.id}-${index}`}
            item={item}
            index={index}
            current={index === current}
            colors={c}
            isAd={isAd}
            canEdit={canEdit && index !== current}
            menuOpen={menu === index}
            onMenu={() => setMenu(menu === index ? null : index)}
            onAction={(c2) => {
              setMenu(null);
              remote.command({ c: c2, id: item.id, i: index });
            }}
          />
        ))}
      </ScrollView>
    </>
  );
}

function Row(props: {
  item: QueueItem;
  index: number;
  current: boolean;
  colors: Colors;
  isAd: boolean;
  canEdit: boolean;
  menuOpen: boolean;
  onMenu: () => void;
  onAction: (c: 'queue_next' | 'queue_remove') => void;
}) {
  const { item, index, current, colors: c, isAd, canEdit, menuOpen } = props;
  return (
    <View>
      <View
        style={[
          styles.row,
          { backgroundColor: current ? c.raised : 'transparent', borderColor: current ? c.line : 'transparent' },
        ]}
      >
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={`${index + 1}. ${item.title}, ${item.artist}`}
          accessibilityState={{ disabled: isAd, selected: current }}
          disabled={isAd}
          onPress={() => remote.command({ c: 'play_item', id: item.id, i: index })}
          style={({ pressed }) => [styles.rowMain, { opacity: isAd ? 0.4 : pressed ? 0.7 : 1 }]}
        >
          <View style={styles.index}>
            {current ? (
              <Glyph name="playing" size={16} color={c.accent} />
            ) : (
              <Text style={[type.small, type.digits, { color: c.muted }]}>{index + 1}</Text>
            )}
          </View>
          <View style={styles.grow}>
            <Text style={[type.body, { color: current ? c.accent : c.text, fontWeight: current ? '700' : '500' }]} numberOfLines={1}>
              {item.title}
            </Text>
            <Text style={[type.small, { color: c.muted }]} numberOfLines={1}>
              {item.artist}
            </Text>
          </View>
        </Pressable>
        {canEdit && (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={t('queueMore', { title: item.title })}
            accessibilityState={{ expanded: menuOpen }}
            hitSlop={6}
            onPress={props.onMenu}
            style={styles.tool}
          >
            <Glyph name="more" size={20} color={c.muted} />
          </Pressable>
        )}
      </View>
      {menuOpen && (
        <View style={styles.actions}>
          <Action label={t('queuePlayNext')} colors={c} onPress={() => props.onAction('queue_next')} />
          <Action label={t('queueRemove')} colors={c} onPress={() => props.onAction('queue_remove')} />
        </View>
      )}
    </View>
  );
}

function Action({ label, colors: c, onPress }: { label: string; colors: Colors; onPress: () => void }) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      onPress={() => {
        void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
        onPress();
      }}
      style={({ pressed }) => [styles.action, { backgroundColor: c.raised, borderColor: c.line, opacity: pressed ? 0.7 : 1 }]}
    >
      <Text style={[type.small, { color: c.text, fontWeight: '600' }]}>{label}</Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  backdrop: { backgroundColor: 'rgba(0,0,0,0.5)' },
  sheet: {
    position: 'absolute',
    left: 0,
    right: 0,
    bottom: 0,
    height: '88%',
    borderTopLeftRadius: 22,
    borderTopRightRadius: 22,
    borderTopWidth: 1,
    overflow: 'hidden',
  },
  header: { paddingHorizontal: 20, paddingTop: 10, paddingBottom: 8, gap: 8 },
  grabber: { alignSelf: 'center', width: 40, height: 4, borderRadius: 2 },
  headerRow: { flexDirection: 'row', alignItems: 'center', gap: 12 },
  list: { paddingHorizontal: 12, paddingBottom: 24, gap: 4 },
  row: { flexDirection: 'row', alignItems: 'center', borderRadius: 14, borderWidth: 1, minHeight: 60 },
  rowMain: { flex: 1, flexDirection: 'row', alignItems: 'center', gap: 12, paddingVertical: 10, paddingLeft: 10 },
  index: { width: 24, alignItems: 'center' },
  tool: { width: 44, minHeight: 48, alignItems: 'center', justifyContent: 'center' },
  actions: { flexDirection: 'row', gap: 8, paddingLeft: 46, paddingVertical: 6 },
  action: { borderRadius: 18, borderWidth: 1, paddingHorizontal: 14, paddingVertical: 9, minHeight: 40, justifyContent: 'center' },
  grow: { flex: 1 },
});
