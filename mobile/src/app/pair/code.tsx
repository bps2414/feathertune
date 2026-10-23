import { Redirect, router } from 'expo-router';
import { useEffect } from 'react';
import { StyleSheet, Text, View } from 'react-native';

import { t } from '../../i18n';
import { remote, useRemote } from '../../remote/app';
import type { Problem } from '../../remote/protocol';
import { Body, Busy, Button, Notice, resetTo, Screen, Steps, Title, type, useColors } from '../../ui';

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

  const isFailed = pairing.status === 'failed' && !!pairing.problem;
  const canRetrySameQr = isFailed && RETRY.includes(pairing.problem!);

  return (
    <Screen
      footer={
        <>
          {canRetrySameQr ? (
            <Button kind="primary" label={t('tryAgain')} onPress={() => void remote.pair(phoneName)} />
          ) : isFailed ? (
            <Button kind="primary" label={t('scanAgain')} onPress={() => router.replace('/pair/scan')} />
          ) : null}
          <Button kind="quiet" label={t('cancel')} onPress={cancel} />
        </>
      }
    >
      <Steps current={4} total={4} />

      {pairing.status === 'connecting' && <Busy label={t('codeConnecting')} />}

      {pairing.status === 'code' && pairing.code && (
        <View style={styles.codeContainer}>
          <Title>{t('codeTitle')}</Title>
          <View style={[styles.codeBox, { backgroundColor: c.raised, borderColor: c.line }]}>
            <Text
              accessibilityLabel={t('codeLabel', { digits: pairing.code.split('').join(' ') })}
              style={[styles.code, type.digits, { color: c.text }]}
            >
              {`${pairing.code.slice(0, 3)} ${pairing.code.slice(3)}`}
            </Text>
          </View>
          <Body>{t('codeBody')}</Body>
          <Busy label={t('codeWaiting')} />
        </View>
      )}

      {pairing.status === 'failed' && pairing.problem && (
        <View style={styles.errorContainer}>
          <Title>{t('codeFailed')}</Title>
          <Notice problem={pairing.problem} />
        </View>
      )}
    </Screen>
  );
}

const styles = StyleSheet.create({
  codeContainer: {
    gap: 16,
    width: '100%',
  },
  codeBox: {
    borderRadius: 18,
    borderWidth: 1,
    paddingVertical: 20,
    alignItems: 'center',
    justifyContent: 'center',
    marginVertical: 8,
  },
  code: {
    fontSize: 44,
    lineHeight: 52,
    fontWeight: '700',
    letterSpacing: 6,
    textAlign: 'center',
  },
  errorContainer: {
    gap: 16,
    width: '100%',
  },
});
