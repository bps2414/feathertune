import { useCameraPermissions } from 'expo-camera';
import { router } from 'expo-router';
import { Linking } from 'react-native';

import { t } from '../../i18n';
import { Body, Button, Screen, Steps, Title } from '../../ui';

/** Why the camera, before the system asks; the phone's settings once it can no longer ask. */
export default function Camera() {
  const [permission, request] = useCameraPermissions();
  const blocked = permission !== null && !permission.granted && !permission.canAskAgain;

  return (
    <Screen
      footer={
        <>
          {blocked ? (
            <Button kind="primary" label={t('openSettings')} onPress={() => void Linking.openSettings()} />
          ) : (
            <Button
              kind="primary"
              label={t('cameraAllow')}
              onPress={async () => {
                if ((await request()).granted) router.replace('/pair/scan');
              }}
            />
          )}
          <Button kind="quiet" label={t('back')} onPress={() => router.back()} />
        </>
      }
    >
      <Steps current={1} total={4} />
      <Title>{t('cameraTitle')}</Title>
      <Body>{t('cameraBody')}</Body>
      {blocked && <Body muted>{t('cameraDenied')}</Body>}
    </Screen>
  );
}
