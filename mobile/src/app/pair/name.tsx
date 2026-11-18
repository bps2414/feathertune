import { Redirect, router } from 'expo-router';
import { useState } from 'react';
import { StyleSheet, TextInput } from 'react-native';

import { t } from '../../i18n';
import { remote, useRemote } from '../../remote/app';
import { MAX_NAME, validName } from '../../remote/protocol';
import { Body, Button, resetTo, Screen, Steps, Title, useColors } from '../../ui';

/** The name the PC shows when it asks to allow this phone. */
export default function Name() {
  const c = useColors();
  const scanned = useRemote((s) => s.pairing.qr !== null);
  const [name, setName] = useState(() => remote.store.getState().phoneName);
  const [invalid, setInvalid] = useState(false);

  if (!scanned) return <Redirect href="/pair" />;

  const submit = () => {
    if (!validName(name)) return setInvalid(true);
    void remote.pair(name);
    router.replace('/pair/code');
  };

  return (
    <Screen
      footer={
        <>
          <Button kind="primary" label={t('namePair')} onPress={submit} />
          <Button
            kind="quiet"
            label={t('cancel')}
            onPress={() => {
              remote.leavePairing();
              if (router.canGoBack()) router.back();
              else resetTo(remote.store.getState().pcs.length > 0 ? '/pcs' : '/pair');
            }}
          />
        </>
      }
    >
      <Steps current={3} total={4} />
      <Title>{t('nameTitle')}</Title>
      <Body>{t('nameBody')}</Body>

      <TextInput
        accessibilityLabel={t('nameLabel')}
        placeholder={t('namePlaceholder')}
        placeholderTextColor={c.muted}
        value={name}
        onChangeText={(text) => {
          setName(text);
          setInvalid(false);
        }}
        maxLength={MAX_NAME}
        autoFocus
        returnKeyType="done"
        onSubmitEditing={submit}
        style={[styles.input, { color: c.text, backgroundColor: c.raised, borderColor: invalid ? c.danger : c.line }]}
      />

      {invalid && <Body muted>{t('nameInvalid')}</Body>}
    </Screen>
  );
}

const styles = StyleSheet.create({
  input: {
    fontSize: 18,
    borderRadius: 14,
    borderWidth: 1,
    paddingHorizontal: 16,
    paddingVertical: 14,
    marginTop: 8,
  },
});
