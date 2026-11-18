import { router } from 'expo-router';
import { useState } from 'react';
import { Linking, PermissionsAndroid, Platform } from 'react-native';

import { t } from '../../i18n';
import { remote } from '../../remote/app';
import { Body, Button, resetTo, Screen, Steps, Title } from '../../ui';

/** Android 17 (API 37) asks for local network access at run time; iOS asks by itself on the first connection. */
async function askLocalNetwork(): Promise<boolean> {
  if (Platform.OS !== 'android' || Number(Platform.Version) < 37) return true;
  const result = await PermissionsAndroid.request(PermissionsAndroid.PERMISSIONS.ACCESS_LOCAL_NETWORK);
  return result === PermissionsAndroid.RESULTS.GRANTED;
}

/** Why the local network, once, before the system asks. */
export default function Network() {
  const [denied, setDenied] = useState(false);

  return (
    <Screen
      footer={
        <>
          <Button
            kind="primary"
            label={t('continue')}
            onPress={async () => {
              if (!(await askLocalNetwork())) return setDenied(true);
              await remote.setNetworkExplained();
              router.replace('/pair/name');
            }}
          />
          {denied && <Button kind="secondary" label={t('openSettings')} onPress={() => void Linking.openSettings()} />}
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
      <Steps current={2} total={4} />
      <Title>{t('networkTitle')}</Title>
      <Body>{t('networkBody')}</Body>
      {denied && <Body muted>{t('networkDenied')}</Body>}
    </Screen>
  );
}
