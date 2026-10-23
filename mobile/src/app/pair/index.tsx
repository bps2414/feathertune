import { useCameraPermissions } from 'expo-camera';
import { router } from 'expo-router';

import { t } from '../../i18n';
import { remote, useRemote } from '../../remote/app';
import { Body, Button, Screen, Title } from '../../ui';

/** Welcome: what pairing needs on the PC. */
export default function Welcome() {
  const [permission] = useCameraPermissions();
  const paired = useRemote((s) => s.pcs.length > 0);
  return (
    <Screen>
      <Title>{t('welcomeTitle')}</Title>
      <Body>{t('welcomeBody')}</Body>
      <Button label={t('welcomeStart')} onPress={() => router.push(permission?.granted ? '/pair/scan' : '/pair/camera')} />
      {paired && router.canGoBack() && (
        <Button
          secondary
          label={t('cancel')}
          onPress={() => {
            remote.leavePairing();
            router.back();
          }}
        />
      )}
    </Screen>
  );
}
