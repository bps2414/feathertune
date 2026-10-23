// The few building blocks every screen uses: dark first, following the system theme, with text that follows the
// system font size (no fixed heights around text).

import { router, type Href } from 'expo-router';
import type { ReactNode } from 'react';
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, Text, useColorScheme, View } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';

import { t, type TextKey } from './i18n';
import type { Problem } from './remote/protocol';
import type { Status } from './remote/remote';

const DARK = { bg: '#111111', card: '#1e1e1e', text: '#f2f2f2', muted: '#a3a3a3', accent: '#e0457b', onAccent: '#ffffff', warn: '#ffb454' };
const LIGHT = { bg: '#ffffff', card: '#f1f1f1', text: '#111111', muted: '#5c5c5c', accent: '#c2185b', onAccent: '#ffffff', warn: '#8a4b00' };

export type Colors = typeof DARK;

export function useColors(): Colors {
  return useColorScheme() === 'light' ? LIGHT : DARK;
}

/** Leaves the current flow for `href`, so Back does not lead into it again. */
export function resetTo(href: Href): void {
  if (router.canDismiss()) router.dismissAll();
  router.replace(href);
}

export function Screen({ children }: { children: ReactNode }) {
  const c = useColors();
  return (
    <SafeAreaView style={[styles.screen, { backgroundColor: c.bg }]}>
      <ScrollView contentContainerStyle={styles.content} keyboardShouldPersistTaps="handled">
        {children}
      </ScrollView>
    </SafeAreaView>
  );
}

export function Title({ children }: { children: ReactNode }) {
  const c = useColors();
  return (
    <Text accessibilityRole="header" style={[styles.title, { color: c.text }]}>
      {children}
    </Text>
  );
}

export function Body({ children, muted }: { children: ReactNode; muted?: boolean }) {
  const c = useColors();
  return <Text style={[styles.body, { color: muted ? c.muted : c.text }]}>{children}</Text>;
}

export function Busy({ label }: { label: string }) {
  const c = useColors();
  return (
    <View style={styles.row} accessible accessibilityLabel={label} accessibilityLiveRegion="polite">
      <ActivityIndicator color={c.accent} />
      <Text style={[styles.body, { color: c.muted }]}>{label}</Text>
    </View>
  );
}

export function Button(props: { label: string; onPress: () => void; hint?: string; secondary?: boolean; disabled?: boolean }) {
  const c = useColors();
  const { label, onPress, hint, secondary, disabled } = props;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityHint={hint}
      accessibilityState={{ disabled: !!disabled }}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [
        styles.button,
        { backgroundColor: secondary ? c.card : c.accent, opacity: disabled ? 0.4 : pressed ? 0.75 : 1 },
      ]}
    >
      <Text style={[styles.buttonText, { color: secondary ? c.text : c.onAccent }]}>{label}</Text>
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

/** What went wrong and what to do, announced when it appears. */
export function Notice({ problem }: { problem: Problem }) {
  const c = useColors();
  const [title, body] = PROBLEMS[problem];
  return (
    <View style={[styles.notice, { backgroundColor: c.card, borderLeftColor: c.warn }]} accessible accessibilityLiveRegion="polite">
      <Text style={[styles.noticeTitle, { color: c.text }]}>{t(title)}</Text>
      <Text style={[styles.body, { color: c.muted }]}>{t(body)}</Text>
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

const styles = StyleSheet.create({
  screen: { flex: 1 },
  content: { padding: 24, gap: 16, flexGrow: 1 },
  title: { fontSize: 26, fontWeight: '700' },
  body: { fontSize: 16 },
  row: { flexDirection: 'row', alignItems: 'center', gap: 12 },
  button: { minHeight: 48, borderRadius: 12, paddingHorizontal: 20, paddingVertical: 12, justifyContent: 'center' },
  buttonText: { fontSize: 17, fontWeight: '600', textAlign: 'center' },
  notice: { borderRadius: 12, borderLeftWidth: 4, padding: 16, gap: 6 },
  noticeTitle: { fontSize: 17, fontWeight: '600' },
});
