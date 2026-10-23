// The building blocks every screen uses. Dark first, following the system theme; the playing screens take their
// colors from the album (the PC's `colors`: the ink becomes the background, the accent the one highlight). Text
// follows the system font size (no fixed heights around text); motion respects the OS reduce-motion setting and
// stops while the app is in the background.

import * as Haptics from 'expo-haptics';
import { router, type Href } from 'expo-router';
import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import {
  AccessibilityInfo,
  ActivityIndicator,
  Animated,
  AppState,
  PanResponder,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  useColorScheme,
  View,
  type ViewStyle,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';

import { t, type TextKey } from './i18n';
import { isFinal, type Problem } from './remote/protocol';
import type { Status } from './remote/remote';

// ---- tokens

/** Slate night, not black: the album ink replaces it while something plays. The accent is a pale periwinkle, the
 * one color that is not the album's. */
const DARK = {
  bg: '#171a22',
  raised: '#222632',
  line: '#323746',
  text: '#f1f2f6',
  muted: '#9ca2b2',
  accent: '#a3b6ff',
  onAccent: '#121624',
  warn: '#f0b96e',
  danger: '#ff9a8f',
};
const LIGHT = {
  bg: '#f3f4f8',
  raised: '#e5e8ef',
  line: '#d2d6e0',
  text: '#161922',
  muted: '#575e6e',
  accent: '#3550c4',
  onAccent: '#ffffff',
  warn: '#8a4b00',
  danger: '#b3261e',
};

export type Colors = typeof DARK;

export const type = {
  display: { fontSize: 30, lineHeight: 36, fontWeight: '700', letterSpacing: -0.6 },
  title: { fontSize: 24, lineHeight: 30, fontWeight: '700', letterSpacing: -0.3 },
  heading: { fontSize: 18, lineHeight: 24, fontWeight: '600' },
  body: { fontSize: 16, lineHeight: 23 },
  small: { fontSize: 14, lineHeight: 19 },
  /** Times and codes: digits that do not shift as they change. */
  digits: { fontVariant: ['tabular-nums'] },
} as const;

export function useColors(): Colors {
  return useColorScheme() === 'light' ? LIGHT : DARK;
}

/** The theme with the album's colors (`[accent, ink, text on accent]`); the plain theme without them. */
export function useAlbumColors(colors: string[] | undefined): Colors {
  const scheme = useColorScheme();
  const key = colors?.join() ?? '';
  return useMemo(() => albumColors(scheme === 'light' ? 'light' : 'dark', key ? key.split(',') : []), [scheme, key]);
}

export function albumColors(scheme: 'light' | 'dark', colors: string[]): Colors {
  const base = scheme === 'light' ? LIGHT : DARK;
  if (colors.length < 2) return base;
  const [accent, ink] = colors;
  // The PC's ink is always dark (white text keeps its contrast on it).
  const bg = scheme === 'dark' ? ink : mix(accent, '#ffffff', 0.87);
  const toward = scheme === 'dark' ? '#ffffff' : '#000000';
  const text = scheme === 'dark' ? '#f6f6f9' : base.text;
  const highlight = readable(accent, bg, toward, 3);
  const onAccent = contrast('#ffffff', highlight) >= contrast(base.text, highlight) ? '#ffffff' : base.text;
  return {
    ...base,
    bg,
    raised: mix(bg, toward, 0.09),
    line: mix(bg, toward, 0.18),
    text,
    muted: readable(mix(text, bg, 0.42), bg, text, 4.5),
    accent: highlight,
    onAccent,
  };
}

type Rgb = [number, number, number];
const rgb = (hex: string): Rgb => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)) as Rgb;
const toHex = (c: Rgb) => `#${c.map((v) => Math.round(v).toString(16).padStart(2, '0')).join('')}`;

/** `a` moved `amount` (0 to 1) of the way toward `b`. */
export function mix(a: string, b: string, amount: number): string {
  const [x, y] = [rgb(a), rgb(b)];
  return toHex([0, 1, 2].map((i) => x[i] + (y[i] - x[i]) * amount) as Rgb);
}

function luminance(hex: string): number {
  const [r, g, b] = rgb(hex).map((v) => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

export function contrast(a: string, b: string): number {
  const [x, y] = [luminance(a), luminance(b)].sort((p, q) => q - p);
  return (x + 0.05) / (y + 0.05);
}

/** `color`, moved toward `to` until it stands `min`:1 against `bg`. */
function readable(color: string, bg: string, to: string, min: number): string {
  for (let step = 0; step <= 10; step++) {
    const c = mix(color, to, step / 10);
    if (contrast(c, bg) >= min) return c;
  }
  return to;
}

// ---- motion

export function useReducedMotion(): boolean {
  const [reduced, setReduced] = useState(false);
  useEffect(() => {
    let live = true;
    void AccessibilityInfo.isReduceMotionEnabled().then((on) => live && setReduced(on));
    const subscription = AccessibilityInfo.addEventListener('reduceMotionChanged', setReduced);
    return () => {
      live = false;
      subscription.remove();
    };
  }, []);
  return reduced;
}

/** True while the app is on screen; animations and clocks stop when it is not. */
export function useForeground(): boolean {
  const [on, setOn] = useState(AppState.currentState === 'active');
  useEffect(() => {
    const subscription = AppState.addEventListener('change', (state) => setOn(state === 'active'));
    return () => subscription.remove();
  }, []);
  return on;
}

/** Eases an Animated.Value to `to`, or jumps there when motion is reduced. */
export function useEased(to: number, ms = 220): Animated.Value {
  const [value] = useState(() => new Animated.Value(to));
  const reduced = useReducedMotion();
  useEffect(() => {
    if (reduced) value.setValue(to);
    else Animated.timing(value, { toValue: to, duration: ms, useNativeDriver: true }).start();
  }, [to, ms, reduced, value]);
  return value;
}

// ---- layout

/** Leaves the current flow for `href`, so Back does not lead into it again. */
export function resetTo(href: Href): void {
  if (router.canDismiss()) router.dismissAll();
  router.replace(href);
}

/** A scrolling screen; `footer` (the screen's buttons) sits after the content, at the bottom when it fits. */
export function Screen(props: { children: ReactNode; footer?: ReactNode; colors?: Colors; scroll?: boolean }) {
  const base = useColors();
  const c = props.colors ?? base;
  return (
    <SafeAreaView style={[styles.screen, { backgroundColor: c.bg }]}>
      <ScrollView
        contentContainerStyle={styles.content}
        keyboardShouldPersistTaps="handled"
        scrollEnabled={props.scroll ?? true}
      >
        {props.children}
        {props.footer && <View style={styles.footer}>{props.footer}</View>}
      </ScrollView>
    </SafeAreaView>
  );
}

/** The app's name, set light and wide: the one place the name appears. */
export function Wordmark() {
  const c = useColors();
  return (
    <Text accessibilityRole="header" style={[styles.wordmark, { color: c.text }]}>
      feather<Text style={{ color: c.accent }}>tune</Text>
    </Text>
  );
}

/** Where the user is in the pairing flow: `current` of `total` short bars. */
export function Steps({ current, total }: { current: number; total: number }) {
  const c = useColors();
  return (
    <View style={styles.steps} accessible accessibilityLabel={t('stepOf', { n: current, total })}>
      {Array.from({ length: total }, (_, i) => (
        <View key={i} style={[styles.step, { backgroundColor: i < current ? c.accent : c.line }]} />
      ))}
    </View>
  );
}

export function Title({ children, colors }: { children: ReactNode; colors?: Colors }) {
  const base = useColors();
  return (
    <Text accessibilityRole="header" style={[type.title, { color: (colors ?? base).text }]}>
      {children}
    </Text>
  );
}

export function Body({ children, muted, colors }: { children: ReactNode; muted?: boolean; colors?: Colors }) {
  const base = useColors();
  const c = colors ?? base;
  return <Text style={[type.body, { color: muted ? c.muted : c.text }]}>{children}</Text>;
}

export function Busy({ label, colors }: { label: string; colors?: Colors }) {
  const base = useColors();
  const c = colors ?? base;
  return (
    <View style={styles.row} accessible accessibilityLabel={label} accessibilityLiveRegion="polite">
      <ActivityIndicator color={c.accent} />
      <Text style={[type.body, { color: c.muted, flexShrink: 1 }]}>{label}</Text>
    </View>
  );
}

/** `primary`: the one main action of a screen; `secondary`: a quieter one next to it; `quiet`: text only. */
export function Button(props: {
  label: string;
  onPress: () => void;
  hint?: string;
  kind?: 'primary' | 'secondary' | 'quiet';
  /** @deprecated use kind */
  secondary?: boolean;
  disabled?: boolean;
  busy?: boolean;
  colors?: Colors;
}) {
  const base = useColors();
  const c = props.colors ?? base;
  const kind = props.kind ?? (props.secondary ? 'secondary' : 'primary');
  const { label, onPress, hint, disabled, busy } = props;
  const fill = kind === 'primary' ? c.accent : kind === 'secondary' ? c.raised : 'transparent';
  const ink = kind === 'primary' ? c.onAccent : kind === 'secondary' ? c.text : c.accent;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityHint={hint}
      accessibilityState={{ disabled: !!disabled, busy: !!busy }}
      disabled={disabled || busy}
      onPress={onPress}
      style={({ pressed }) => [
        styles.button,
        kind === 'quiet' && styles.quiet,
        { backgroundColor: fill, opacity: disabled ? 0.4 : pressed ? 0.7 : 1 },
      ]}
    >
      {busy ? <ActivityIndicator color={ink} /> : <Text style={[styles.buttonText, { color: ink }]}>{label}</Text>}
    </Pressable>
  );
}

const PROBLEMS: Record<Problem, [TextKey, TextKey]> = {
  denied: ['problemDenied', 'problemDeniedBody'],
  expired: ['problemExpired', 'problemExpiredBody'],
  removed: ['problemRemoved', 'problemRemovedBody'],
  busy: ['problemBusy', 'problemBusyBody'],
  timeout: ['problemLost', 'problemLostBody'],
  lost: ['problemLost', 'problemLostBody'],
  invalid: ['problemInvalid', 'problemInvalidBody'],
  too_big: ['problemTooBig', 'problemTooBigBody'],
  off: ['problemOff', 'problemOffBody'],
  full: ['problemFull', 'problemFullBody'],
  unreachable: ['problemUnreachable', 'problemUnreachableBody'],
  blocked: ['problemBlocked', 'problemBlockedBody'],
  other_network: ['problemOtherNetwork', 'problemOtherNetworkBody'],
  update_app: ['problemUpdateApp', 'problemUpdateAppBody'],
  update_pc: ['problemUpdatePc', 'problemUpdatePcBody'],
  not_this_pc: ['problemNotThisPc', 'problemNotThisPcBody'],
};

/** What went wrong and what to do, announced when it appears. The stripe is red when trying again will not help. */
export function Notice({ problem, colors, children }: { problem: Problem; colors?: Colors; children?: ReactNode }) {
  const base = useColors();
  const c = colors ?? base;
  const [title, body] = PROBLEMS[problem];
  return (
    <View
      style={[styles.notice, { backgroundColor: c.raised, borderLeftColor: isFinal(problem) ? c.danger : c.warn }]}
      accessibilityLiveRegion="polite"
    >
      <View accessible>
        <Text style={[type.heading, { color: c.text }]}>{t(title)}</Text>
        <Text style={[type.body, { color: c.muted }]}>{t(body)}</Text>
      </View>
      {children}
    </View>
  );
}

const STATUS: Record<Status, TextKey> = {
  idle: 'statusIdle',
  connecting: 'statusConnecting',
  connected: 'statusConnected',
  waiting: 'statusWaiting',
  failed: 'statusIdle',
};

export function statusText(status: Status): string {
  return t(STATUS[status]);
}

/** A PC's connection as a dot: accent when connected, a slow pulse while connecting, warn or danger otherwise. */
export function StatusDot({ status, problem, colors }: { status: Status; problem: Problem | null; colors?: Colors }) {
  const base = useColors();
  const c = colors ?? base;
  const reduced = useReducedMotion();
  const foreground = useForeground();
  const [pulse] = useState(() => new Animated.Value(1));
  const moving = status === 'connecting' && foreground && !reduced;
  useEffect(() => {
    if (!moving) return pulse.setValue(1);
    const loop = Animated.loop(
      Animated.sequence([
        Animated.timing(pulse, { toValue: 0.3, duration: 700, useNativeDriver: true }),
        Animated.timing(pulse, { toValue: 1, duration: 700, useNativeDriver: true }),
      ])
    );
    loop.start();
    return () => loop.stop();
  }, [moving, pulse]);
  const color =
    status === 'connected' ? c.accent : problem && isFinal(problem) ? c.danger : problem ? c.warn : c.muted;
  return <Animated.View style={[styles.dot, { backgroundColor: color, opacity: pulse }]} />;
}

// ---- the slider

/**
 * A horizontal slider over 0..1, for the seek bar and the volume. `onChange` follows the finger; `onCommit` comes
 * once, on release. A light tick every 1/`ticks` of the way. Screen readers adjust it with `onStep`.
 */
export function Slider(props: {
  value: number;
  onChange?: (fraction: number) => void;
  onCommit: (fraction: number) => void;
  onStep: (direction: 1 | -1) => void;
  onGrab?: (grabbed: boolean) => void;
  label: string;
  valueText: string;
  thick?: boolean;
  ticks?: number;
  disabled?: boolean;
  colors: Colors;
}) {
  const latest = useRef(props);
  useEffect(() => {
    latest.current = props;
  });
  const track = useRef<React.ElementRef<typeof View>>(null);
  const frame = useRef({ x: 0, width: 1 });
  const lastTick = useRef(-1);
  const [drag, setDrag] = useState<number | null>(null);

  const responder = useMemo(() => {
    const fraction = (pageX: number) => Math.min(1, Math.max(0, (pageX - frame.current.x) / frame.current.width));
    const move = (pageX: number) => {
      const f = fraction(pageX);
      const tick = Math.floor(f * (latest.current.ticks ?? 20));
      if (tick !== lastTick.current) {
        lastTick.current = tick;
        void Haptics.selectionAsync();
      }
      setDrag(f);
      latest.current.onChange?.(f);
    };
    const end = (pageX: number | null) => {
      if (pageX !== null) latest.current.onCommit(fraction(pageX));
      setDrag(null);
      latest.current.onGrab?.(false);
    };
    // eslint-disable-next-line react-hooks/refs
    return PanResponder.create({
      onStartShouldSetPanResponder: () => !latest.current.disabled,
      onMoveShouldSetPanResponderCapture: () => !latest.current.disabled,
      onPanResponderTerminationRequest: () => false,
      onPanResponderGrant: (event) => {
        const pageX = event.nativeEvent.pageX;
        latest.current.onGrab?.(true);
        void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
        track.current?.measureInWindow((x: number, _y: number, width: number) => {
          frame.current = { x, width: Math.max(1, width) };
          lastTick.current = Math.floor(fraction(pageX) * (latest.current.ticks ?? 20));
          move(pageX);
        });
      },
      onPanResponderMove: (event) => move(event.nativeEvent.pageX),
      onPanResponderRelease: (event) => end(event.nativeEvent.pageX),
      onPanResponderTerminate: () => end(null),
    });
  }, []);

  const { colors: c, thick, disabled } = props;
  const shown = drag ?? Math.min(1, Math.max(0, props.value));
  const height = thick ? (drag !== null ? 14 : 10) : 4;
  return (
    <View
      {...responder.panHandlers}
      accessible
      accessibilityRole="adjustable"
      accessibilityLabel={props.label}
      accessibilityValue={{ text: props.valueText }}
      accessibilityState={{ disabled: !!disabled }}
      accessibilityActions={[{ name: 'increment' }, { name: 'decrement' }]}
      onAccessibilityAction={(event) => !disabled && props.onStep(event.nativeEvent.actionName === 'increment' ? 1 : -1)}
      style={[styles.sliderHit, disabled && { opacity: 0.35 }]}
    >
      <View ref={track} style={[styles.sliderTrack, { height, borderRadius: height / 2, backgroundColor: c.line }]}>
        <View style={{ width: `${shown * 100}%`, height, borderRadius: height / 2, backgroundColor: c.accent }} />
      </View>
      {!thick && (
        <View
          pointerEvents="none"
          style={[styles.knob, { left: `${shown * 100}%`, backgroundColor: c.text }]}
        />
      )}
    </View>
  );
}

// ---- glyphs (drawn, so the app needs no icon font)

export function Glyph({ name, size, color }: { name: GlyphName; size: number; color: string }) {
  const s = size;
  const tri = (flip: boolean): ViewStyle => ({
    width: 0,
    height: 0,
    borderTopWidth: s * 0.42,
    borderBottomWidth: s * 0.42,
    borderTopColor: 'transparent',
    borderBottomColor: 'transparent',
    ...(flip ? { borderRightWidth: s * 0.62, borderRightColor: color } : { borderLeftWidth: s * 0.62, borderLeftColor: color }),
  });
  const bar = (w: number, h: number): ViewStyle => ({ width: w, height: h, borderRadius: w / 2, backgroundColor: color });
  let body: ReactNode = null;
  switch (name) {
    case 'play':
      body = <View style={[tri(false), { marginLeft: s * 0.14 }]} />;
      break;
    case 'pause':
      body = (
        <View style={[styles.row, { gap: s * 0.22 }]}>
          <View style={bar(s * 0.2, s * 0.78)} />
          <View style={bar(s * 0.2, s * 0.78)} />
        </View>
      );
      break;
    case 'next':
    case 'previous': {
      const flip = name === 'previous';
      const parts = [<View key="t" style={tri(flip)} />, <View key="b" style={bar(s * 0.14, s * 0.84)} />];
      body = <View style={styles.row}>{flip ? parts.reverse() : parts}</View>;
      break;
    }
    case 'queue':
      body = (
        <View style={{ gap: s * 0.16 }}>
          {[1, 1, 0.6].map((w, i) => (
            <View key={i} style={bar(s * 0.9 * w, s * 0.12)} />
          ))}
        </View>
      );
      break;
    case 'down':
      body = (
        <View
          style={{
            width: s * 0.5,
            height: s * 0.5,
            marginTop: -s * 0.25,
            borderRightWidth: s * 0.12,
            borderBottomWidth: s * 0.12,
            borderColor: color,
            transform: [{ rotate: '45deg' }],
          }}
        />
      );
      break;
    case 'speaker':
      body = (
        <View style={styles.row}>
          <View style={{ width: s * 0.26, height: s * 0.36, backgroundColor: color, borderRadius: 1 }} />
          <View style={[tri(true), { borderTopWidth: s * 0.36, borderBottomWidth: s * 0.36, borderRightWidth: s * 0.34 }]} />
        </View>
      );
      break;
    case 'playing':
      body = (
        <View style={[styles.row, { alignItems: 'flex-end', gap: s * 0.12, height: s * 0.8 }]}>
          {[0.5, 1, 0.7].map((h, i) => (
            <View key={i} style={{ width: s * 0.18, height: s * 0.8 * h, borderRadius: 1, backgroundColor: color }} />
          ))}
        </View>
      );
      break;
  }
  return (
    <View pointerEvents="none" style={{ width: s, height: s, alignItems: 'center', justifyContent: 'center' }}>
      {body}
    </View>
  );
}

export type GlyphName = 'play' | 'pause' | 'next' | 'previous' | 'queue' | 'down' | 'speaker' | 'playing';

/** A round button holding a glyph; `size` is its diameter (at least 48, the touch target). */
export function IconButton(props: {
  glyph: GlyphName;
  label: string;
  onPress: () => void;
  size?: number;
  filled?: boolean;
  disabled?: boolean;
  colors: Colors;
}) {
  const { glyph, label, onPress, filled, disabled, colors: c } = props;
  const size = Math.max(48, props.size ?? 48);
  const reduced = useReducedMotion();
  const [scale] = useState(() => new Animated.Value(1));
  const press = (to: number) => !reduced && Animated.spring(scale, { toValue: to, speed: 40, bounciness: 6, useNativeDriver: true }).start();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled: !!disabled }}
      disabled={disabled}
      hitSlop={8}
      onPress={() => {
        void Haptics.impactAsync(filled ? Haptics.ImpactFeedbackStyle.Medium : Haptics.ImpactFeedbackStyle.Light);
        onPress();
      }}
      onPressIn={() => press(0.9)}
      onPressOut={() => press(1)}
      style={{ opacity: disabled ? 0.35 : 1 }}
    >
      <Animated.View
        style={{
          width: size,
          height: size,
          borderRadius: size / 2,
          alignItems: 'center',
          justifyContent: 'center',
          backgroundColor: filled ? c.accent : 'transparent',
          transform: [{ scale }],
        }}
      >
        <Glyph name={glyph} size={size * (filled ? 0.38 : 0.5)} color={filled ? c.onAccent : c.text} />
      </Animated.View>
    </Pressable>
  );
}

/** `m:ss`, or `h:mm:ss` from an hour. */
export function clock(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, '0');
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${ss}` : `${m}:${ss}`;
}

const styles = StyleSheet.create({
  screen: { flex: 1 },
  content: { paddingHorizontal: 24, paddingTop: 20, paddingBottom: 24, gap: 16, flexGrow: 1 },
  footer: { marginTop: 'auto', gap: 8, paddingTop: 16 },
  wordmark: { fontSize: 34, fontWeight: '300', letterSpacing: 1.5 },
  steps: { flexDirection: 'row', gap: 6 },
  step: { flex: 1, height: 3, borderRadius: 2 },
  row: { flexDirection: 'row', alignItems: 'center', gap: 12 },
  button: { minHeight: 52, borderRadius: 26, paddingHorizontal: 24, paddingVertical: 12, justifyContent: 'center' },
  quiet: { minHeight: 48, paddingVertical: 8 },
  buttonText: { fontSize: 17, lineHeight: 22, fontWeight: '600', textAlign: 'center' },
  notice: { borderRadius: 14, borderLeftWidth: 4, padding: 16, gap: 12 },
  dot: { width: 10, height: 10, borderRadius: 5 },
  sliderHit: { minHeight: 44, justifyContent: 'center' },
  sliderTrack: { overflow: 'hidden' },
  knob: { position: 'absolute', width: 16, height: 16, borderRadius: 8, marginLeft: -8, top: '50%', marginTop: -8 },
});
