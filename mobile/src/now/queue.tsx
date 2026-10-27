// The queue as a sheet over the player. Tap a row to play it; with a version 2 PC, drag a row by its handle to
// move it (`queue_move`) and open its ⋯ for the safe actions, Play next and Remove (`queue_next`, `queue_remove`).
// A drag shows its result at once; when the PC's queue does not follow (the page may not take a drag it did not
// see from a mouse), the order goes back, the handles go away and the ⋯ actions stay.

import * as Haptics from 'expo-haptics';
import { useEffect, useMemo, useRef, useState } from 'react';
import { Animated, BackHandler, PanResponder, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

import { t } from '../i18n';
import { remote, useRemote } from '../remote/app';
import { hasQueueActions, type QueueItem } from '../remote/protocol';
import { Body, Glyph, IconButton, type, useReducedMotion, type Colors } from '../ui';

/** How long a drag waits for the PC's queue to show it before it is taken back. */
const MOVE_WAIT_MS = 2500;
const SHEET_MS = 260;

/** Once the PC ignored a drag, this app run uses the ⋯ actions only. */
let dragRefused = false;

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

interface Pending {
  items: QueueItem[];
  current: number | null;
  /** The ids before the move, to tell an unchanged queue from the moved one. */
  before: string;
}

function QueueList({ colors: c, onClose }: { colors: Colors; onClose: () => void }) {
  const queue = useRemote((s) => s.queue);
  const v2 = useRemote((s) => hasQueueActions(s.hello));
  const isAd = useRemote((s) => s.player?.ad ?? false);
  const refusal = useRemote((s) => s.refusal);

  const [pending, setPendingState] = useState<Pending | null>(null);
  const [refused, setRefused] = useState(dragRefused);
  const [menu, setMenu] = useState<number | null>(null);
  const [drag, setDragState] = useState<{ from: number; to: number } | null>(null);
  const [dragY] = useState(() => new Animated.Value(0));
  // Rows are one line each, so the first one's height is every row's.
  const [rowHeight, setRowHeight] = useState(64);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Mirrors of the two states above, for the gesture and timer callbacks.
  const pendingRef = useRef<Pending | null>(null);
  const dragRef = useRef<{ from: number; to: number } | null>(null);
  const setPending = (p: Pending | null) => {
    pendingRef.current = p;
    setPendingState(p);
  };
  const setDrag = (d: { from: number; to: number } | null) => {
    dragRef.current = d;
    setDragState(d);
  };

  const refuse = () => {
    dragRefused = true;
    setRefused(true);
    setPending(null);
  };

  // The PC's queue came: a moved one ends the wait, the same one as before means the drag did not take.
  useEffect(() => {
    const p = pendingRef.current;
    if (!p) return;
    if (timer.current) clearTimeout(timer.current);
    if (queue && queue.items.map((x) => x.id).join() === p.before) refuse();
    else setPending(null);
    // Only a new queue from the PC matters here.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [queue]);

  useEffect(() => () => {
    if (timer.current) clearTimeout(timer.current);
  }, []);

  const items = pending?.items ?? queue?.items ?? [];
  const current = pending ? pending.current : (queue?.current ?? null);
  const canEdit = v2 && !isAd && pending === null;
  const canDrag = canEdit && !refused;

  const move = (from: number, to: number) => {
    if (!queue || from === to || to < 0 || to >= items.length) return;
    const moved = [...items];
    const [item] = moved.splice(from, 1);
    moved.splice(to, 0, item);
    const playing = current !== null ? items[current] : null;
    setPending({ items: moved, current: playing ? moved.indexOf(playing) : null, before: items.map((x) => x.id).join() });
    remote.command({ c: 'queue_move', id: item.id, i: from, to });
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(refuse, MOVE_WAIT_MS);
  };

  const dragging = {
    start: (from: number) => {
      setMenu(null);
      dragY.setValue(0);
      setDrag({ from, to: from });
      void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
    },
    move: (dy: number) => {
      dragY.setValue(dy);
      const d = dragRef.current;
      if (!d) return;
      const to = Math.max(0, Math.min(items.length - 1, d.from + Math.round(dy / rowHeight)));
      if (to === d.to) return;
      void Haptics.selectionAsync();
      setDrag({ ...d, to });
    },
    end: () => {
      const d = dragRef.current;
      setDrag(null);
      dragY.setValue(0);
      if (d && d.to !== d.from) move(d.from, d.to);
    },
  };

  const shift = (k: number): number => {
    if (!drag || k === drag.from) return 0;
    const { from, to } = drag;
    if (from < to && k > from && k <= to) return -rowHeight;
    if (to < from && k >= to && k < from) return rowHeight;
    return 0;
  };

  const note = !v2
    ? t('queueUpdatePc')
    : isAd
      ? t('nowAdNote')
      : refusal === 'unavailable'
        ? t('nowUnavailable')
        : refused
          ? t('queueDragFailed')
          : null;

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
      <ScrollView contentContainerStyle={styles.list} scrollEnabled={drag === null}>
        {items.length === 0 && <Body muted colors={c}>{t('nowEmptyQueue')}</Body>}
        {items.map((item, index) => (
          <Row
            key={`${item.id}-${index}`}
            item={item}
            index={index}
            count={items.length}
            current={index === current}
            colors={c}
            isAd={isAd}
            canEdit={canEdit && index !== current}
            canDrag={canDrag && index !== current}
            menuOpen={menu === index}
            dragged={drag?.from === index}
            dragY={dragY}
            shift={shift(index)}
            onMenu={() => setMenu(menu === index ? null : index)}
            onLayout={(h) => index === 0 && menu === null && setRowHeight(Math.round(h))}
            dragging={dragging}
            onMove={move}
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
  count: number;
  current: boolean;
  colors: Colors;
  isAd: boolean;
  canEdit: boolean;
  canDrag: boolean;
  menuOpen: boolean;
  dragged: boolean;
  dragY: Animated.Value;
  shift: number;
  onMenu: () => void;
  onLayout: (height: number) => void;
  dragging: { start: (from: number) => void; move: (dy: number) => void; end: () => void };
  onMove: (from: number, to: number) => void;
  onAction: (c: 'queue_next' | 'queue_remove') => void;
}) {
  const { item, index, count, current, colors: c, isAd, canEdit, canDrag, menuOpen, dragged } = props;
  const latest = useRef(props);
  useEffect(() => {
    latest.current = props;
  });

  const responder = useMemo(
    () =>
      // eslint-disable-next-line react-hooks/refs
      PanResponder.create({
        onStartShouldSetPanResponder: () => latest.current.canDrag,
        onMoveShouldSetPanResponder: () => latest.current.canDrag,
        onPanResponderTerminationRequest: () => false,
        onPanResponderGrant: () => latest.current.dragging.start(latest.current.index),
        onPanResponderMove: (_, g) => latest.current.dragging.move(g.dy),
        onPanResponderRelease: () => latest.current.dragging.end(),
        onPanResponderTerminate: () => latest.current.dragging.end(),
      }),
    []
  );

  return (
    <Animated.View
      onLayout={(e) => props.onLayout(e.nativeEvent.layout.height)}
      style={[
        dragged
          ? { zIndex: 10, elevation: 8, transform: [{ translateY: props.dragY }, { scale: 1.02 }] }
          : { transform: [{ translateY: props.shift }] },
      ]}
    >
      <View
        style={[
          styles.row,
          {
            backgroundColor: dragged ? c.raised : current ? c.raised : 'transparent',
            borderColor: current || dragged ? c.line : 'transparent',
          },
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
        {canDrag && (
          <View
            {...responder.panHandlers}
            accessible
            accessibilityRole="adjustable"
            accessibilityLabel={t('queueDrag', { title: item.title })}
            accessibilityHint={t('queueDragHint')}
            accessibilityActions={[
              { name: 'increment', label: t('queueMoveDown') },
              { name: 'decrement', label: t('queueMoveUp') },
            ]}
            onAccessibilityAction={(e) => {
              const to = index + (e.nativeEvent.actionName === 'increment' ? 1 : -1);
              if (to >= 0 && to < count) props.onMove(index, to);
            }}
            style={styles.tool}
          >
            <Glyph name="handle" size={22} color={c.muted} />
          </View>
        )}
      </View>
      {menuOpen && (
        <View style={styles.actions}>
          <Action label={t('queuePlayNext')} colors={c} onPress={() => props.onAction('queue_next')} />
          <Action label={t('queueRemove')} colors={c} onPress={() => props.onAction('queue_remove')} />
        </View>
      )}
    </Animated.View>
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
