import { Redirect, router } from 'expo-router';
import { useEffect } from 'react';
import { StyleSheet, Text } from 'react-native';

import { t } from '../../i18n';
import { remote, useRemote } from '../../remote/app';
import type { Problem } from '../../remote/protocol';
import { Body, Busy, Button, Notice, resetTo, Screen, Title, useColors } from '../../ui';

/** Problems after which the same QR can be tried again: nothing reached the PC, so its secret is not spent. */
const RETRY: Problem[] = ['unreachable', 'blocked', 'other_network'];

/** The 6 digits to compare with the PC's screen, while the PC asks the user. */
export default function Code() {
  const c = useColors();
  const pairing = useRemote((s) => s.pairing);
  const phoneName = useRemote((s) => s.phoneName);
  useEffect(() => {
    if (pairing.status === 'done') router.replace('/pair/done');
  }, [pairing.status]);
  if (pairing.status === 'idle') return <Redirect href="/" />;
  const cancel = () => {
    remote.leavePairing();
    resetTo('/');
  };
  return (
    <Screen>
      {pairing.status === 'connecting' && <Busy label={t('codeConnecting')} />}
      {pairing.status === 'code' && pairing.code && (
        <>
          <Title>{t('codeTitle')}</Title>
          <Text
            accessibilityLabel={t('codeLabel', { digits: pairing.code.split('').join(' ') })}
            style={[styles.code, { color: c.text }]}
          >
            {`${pairing.code.slice(0, 3)} ${pairing.code.slice(3)}`}
          </Text>
          <Body>{t('codeBody')}</Body>
          <Busy label={t('codeWaiting')} />
        </>
      )}
      {pairing.status === 'failed' && pairing.problem && (
        <>
          <Title>{t('codeFailed')}</Title>
          <Notice problem={pairing.problem} />
          {RETRY.includes(pairing.problem) && <Button label={t('tryAgain')} onPress={() => void remote.pair(phoneName)} />}
          <Button
            secondary={RETRY.includes(pairing.problem)}
            label={t('scanAgain')}
            onPress={() => router.replace('/pair/scan')}
          />
        </>
      )}
      <Button secondary label={t('cancel')} onPress={cancel} />
    </Screen>
  );
}

const styles = StyleSheet.create({
  code: { fontSize: 48, fontWeight: '700', letterSpacing: 4, textAlign: 'center', fontVariant: ['tabular-nums'] },
});
