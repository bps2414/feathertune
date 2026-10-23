import { StyleSheet, View } from 'react-native';

import { t } from '../../i18n';
import { remote, useRemote } from '../../remote/app';
import { Body, Button, Glyph, resetTo, Screen, Title, useColors, Wordmark } from '../../ui';

/** Pairing completed successfully. */
export default function Done() {
  const c = useColors();
  const name = useRemote((s) => s.pcs.find((pc) => pc.id === s.pairing.pcId)?.name ?? s.hello?.name ?? '');

  return (
    <Screen
      footer={
        <Button
          kind="primary"
          label={t('continue')}
          onPress={() => {
            remote.leavePairing();
            resetTo('/now');
          }}
        />
      }
    >
      <View style={styles.center}>
        <Wordmark />
        <View style={[styles.badge, { backgroundColor: c.accent }]}>
          <Glyph name="playing" size={28} color={c.onAccent} />
        </View>
        <Title>{t('doneTitle')}</Title>
        <Body>{t('doneBody', { name })}</Body>
      </View>
    </Screen>
  );
}

const styles = StyleSheet.create({
  center: {
    alignItems: 'center',
    justifyContent: 'center',
    gap: 16,
    paddingVertical: 32,
    textAlign: 'center',
  },
  badge: {
    width: 64,
    height: 64,
    borderRadius: 32,
    alignItems: 'center',
    justifyContent: 'center',
    marginVertical: 12,
  },
});
