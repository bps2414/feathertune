import { useCameraPermissions } from 'expo-camera';
import { router } from 'expo-router';
import { Linking } from 'react-native';

import { t } from '../../i18n';
import { Body, Button, Screen, Title } from '../../ui';

/** Why the camera, before the system asks; the phone's settings once it can no longer ask. */
export default function Camera() {
  const [permission, request] = useCameraPermissions();
  const blocked = permission !== null && !permission.granted && !permission.canAskAgain;
  return (
    <Screen>
      <Title>{t('cameraTitle')}</Title>
      <Body>{t('cameraBody')}</Body>
      {blocked && <Body muted>{t('cameraDenied')}</Body>}
      {blocked ? (
        <Button label={t('openSettings')} onPress={() => void Linking.openSettings()} />
      ) : (
        <Button
          label={t('cameraAllow')}
          onPress={async () => {
            if ((await request()).granted) router.replace('/pair/scan');
          }}
        />
      )}
      <Button secondary label={t('back')} onPress={() => router.back()} />
    </Screen>
  );
}
