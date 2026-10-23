import { router } from 'expo-router';
import { Alert, StyleSheet, Text, View } from 'react-native';

import { t } from '../i18n';
import { remote, useRemote } from '../remote/app';
import type { PcRecord } from '../remote/remote';
import { Body, Button, Notice, resetTo, Screen, StatusDot, statusText, Title, useColors } from '../ui';

/**
 * Paired PCs list and device switcher:
 * - Clear diagnostics for offline, sleeping, network mismatch, Windows Firewall (Public network),
 *   remote removal (pair again), and version mismatches using protocol close codes (4000-4009).
 * - Connect to any paired PC (the others remain paired).
 * - Remove a PC with confirmation.
 * - Pair an additional PC.
 */
export default function Pcs() {
  const pcs = useRemote((s) => s.pcs);

  return (
    <Screen
      footer={
        <>
          <Button label={t('pcsPair')} onPress={() => router.push('/pair')} />
          {router.canGoBack() && <Button kind="quiet" label={t('back')} onPress={() => router.back()} />}
        </>
      }
    >
      <Title>{t('pcsTitle')}</Title>

      {pcs.length === 0 ? (
        <Body muted>{t('pcsEmpty')}</Body>
      ) : (
        pcs.map((pc) => <PcCard key={pc.id} pc={pc} />)
      )}
    </Screen>
  );
}

function PcCard({ pc }: { pc: PcRecord }) {
  const c = useColors();
  const active = useRemote((s) => s.activeId === pc.id);
  const status = useRemote((s) => (s.activeId === pc.id ? s.status : 'idle'));
  const problem = useRemote((s) => (s.activeId === pc.id ? s.problem : null));
  const isConnected = active && status === 'connected';
  const label = statusText(status);

  const remove = () =>
    Alert.alert(t('pcRemoveTitle', { name: pc.name }), t('pcRemoveBody'), [
      { text: t('cancel'), style: 'cancel' },
      { text: t('pcRemove'), style: 'destructive', onPress: () => void remote.remove(pc.id) },
    ]);

  return (
    <View
      style={[styles.card, { backgroundColor: c.raised, borderColor: isConnected ? c.accent : c.line }]}
      accessible
      accessibilityLabel={t('pcRow', { name: pc.name, status: isConnected ? t('pcConnectedBadge') : label })}
    >
      {/* Top Header: Name and Status */}
      <View style={styles.cardHeader}>
        <View style={styles.nameRow}>
          <StatusDot status={status} problem={problem} colors={c} />
          <Text style={[styles.name, { color: c.text }]} numberOfLines={1}>
            {pc.name}
          </Text>
        </View>
        <Text style={[styles.statusBadge, { color: isConnected ? c.accent : c.muted }]}>
          {isConnected ? t('pcConnectedBadge') : label}
        </Text>
      </View>

      {/* Diagnostic problem explanation (close codes 4000-4009 & network errors) */}
      {problem && (
        <Notice problem={problem} colors={c}>
          {problem === 'removed' ? (
            <Button
              label={t('pairAgain')}
              onPress={() => router.push('/pair')}
              colors={c}
            />
          ) : problem === 'unreachable' || problem === 'blocked' || problem === 'other_network' ? (
            <Button
              kind="secondary"
              label={t('tryAgain')}
              onPress={() => {
                void remote.connect(pc.id);
              }}
              colors={c}
            />
          ) : null}
        </Notice>
      )}

      {/* Action buttons */}
      <View style={styles.actionsRow}>
        {isConnected ? (
          <Button
            kind="primary"
            label={t('pcOpenNow')}
            onPress={() => resetTo('/now')}
            colors={c}
          />
        ) : (
          <Button
            kind="primary"
            label={t('pcConnect')}
            hint={t('pcConnectHint', { name: pc.name })}
            busy={active && status === 'connecting'}
            onPress={() => {
              void remote.connect(pc.id);
              resetTo('/now');
            }}
            colors={c}
          />
        )}
        <Button
          kind="secondary"
          label={t('pcRemove')}
          hint={t('pcRemoveHint', { name: pc.name })}
          onPress={remove}
          colors={c}
        />
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  card: {
    borderRadius: 18,
    borderWidth: 1,
    padding: 18,
    gap: 14,
  },
  cardHeader: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    gap: 8,
  },
  nameRow: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 10,
    flexShrink: 1,
  },
  name: {
    fontSize: 18,
    lineHeight: 24,
    fontWeight: '700',
  },
  statusBadge: {
    fontSize: 13,
    lineHeight: 18,
    fontWeight: '600',
  },
  actionsRow: {
    gap: 10,
  },
});
