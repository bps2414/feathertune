import { router } from 'expo-router';
import { Alert, StyleSheet, Text, View } from 'react-native';

import { t } from '../i18n';
import { remote, useRemote } from '../remote/app';
import type { PcRecord } from '../remote/remote';
import { Body, Button, Notice, resetTo, Screen, statusText, Title, useColors } from '../ui';

/** The paired PCs: connect to one (the others stay paired), remove one, pair another. */
export default function Pcs() {
  const pcs = useRemote((s) => s.pcs);
  return (
    <Screen>
      <Title>{t('pcsTitle')}</Title>
      {pcs.map((pc) => (
        <Row key={pc.id} pc={pc} />
      ))}
      <Button label={t('pcsPair')} onPress={() => router.push('/pair')} />
      {router.canGoBack() && <Button secondary label={t('back')} onPress={() => router.back()} />}
    </Screen>
  );
}

function Row({ pc }: { pc: PcRecord }) {
  const c = useColors();
  const active = useRemote((s) => s.activeId === pc.id);
  const status = useRemote((s) => (s.activeId === pc.id ? s.status : 'idle'));
  const problem = useRemote((s) => (s.activeId === pc.id ? s.problem : null));
  const label = statusText(status);
  const remove = () =>
    Alert.alert(t('pcRemoveTitle', { name: pc.name }), t('pcRemoveBody'), [
      { text: t('cancel'), style: 'cancel' },
      { text: t('pcRemove'), style: 'destructive', onPress: () => void remote.remove(pc.id) },
    ]);
  return (
    <View style={[styles.row, { backgroundColor: c.raised }]}>
      <View accessible accessibilityLabel={t('pcRow', { name: pc.name, status: label })}>
        <Text style={[styles.name, { color: c.text }]}>{pc.name}</Text>
        <Body muted>{label}</Body>
      </View>
      {problem && <Notice problem={problem} />}
      {problem === 'removed' && <Button label={t('pairAgain')} onPress={() => router.push('/pair')} />}
      {active && status === 'connected' ? null : (
        <Button
          label={t('pcConnect')}
          hint={t('pcConnectHint', { name: pc.name })}
          onPress={() => {
            void remote.connect(pc.id);
            resetTo('/now');
          }}
        />
      )}
      <Button secondary label={t('pcRemove')} hint={t('pcRemoveHint', { name: pc.name })} onPress={remove} />
    </View>
  );
}

const styles = StyleSheet.create({
  row: { borderRadius: 16, padding: 16, gap: 12 },
  name: { fontSize: 19, fontWeight: '600' },
});
