import { useCameraPermissions } from 'expo-camera';
import { router } from 'expo-router';
import { StyleSheet, View } from 'react-native';

import { t } from '../../i18n';
import { remote, useRemote } from '../../remote/app';
import { Body, Button, Screen, Title, Wordmark } from '../../ui';

/** Welcome: what pairing needs on the PC. */
export default function Welcome() {
  const [permission] = useCameraPermissions();
  const paired = useRemote((s) => s.pcs.length > 0);

  return (
    <Screen
      footer={
        <>
          <Button
            kind="primary"
            label={t('welcomeStart')}
            onPress={() => router.push(permission?.granted ? '/pair/scan' : '/pair/camera')}
          />
          {paired && router.canGoBack() && (
            <Button
              kind="quiet"
              label={t('cancel')}
              onPress={() => {
                remote.leavePairing();
                router.back();
              }}
            />
          )}
        </>
      }
    >
      <View style={styles.header}>
        <Wordmark />
      </View>
      <Title>{t('welcomeTitle')}</Title>
      <Body>{t('welcomeBody')}</Body>
    </Screen>
  );
}

const styles = StyleSheet.create({
  header: {
    paddingVertical: 12,
  },
});
