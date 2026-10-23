import { CameraView, useCameraPermissions } from 'expo-camera';
import { Redirect, router } from 'expo-router';
import { useRef, useState } from 'react';
import { StyleSheet, View } from 'react-native';

import { t } from '../../i18n';
import { remote, useRemote } from '../../remote/app';
import { parsePairingUri } from '../../remote/protocol';
import { Body, Button, Screen, Steps, Title, useColors } from '../../ui';

/** The QR scanner. A code that is not a pairing URI says so and keeps scanning. */
export default function Scan() {
  const [permission] = useCameraPermissions();
  const explained = useRemote((s) => s.networkExplained);
  const [wrong, setWrong] = useState(false);
  const taken = useRef(false);
  const c = useColors();

  if (permission && !permission.granted) return <Redirect href="/pair/camera" />;

  return (
    <Screen
      footer={<Button kind="quiet" label={t('back')} onPress={() => router.back()} />}
    >
      <Steps current={1} total={4} />
      <Title>{t('scanTitle')}</Title>
      <Body muted>{t('scanHint')}</Body>

      <View style={[styles.frame, { borderColor: wrong ? c.warn : c.line }]}>
        {permission?.granted && (
          <CameraView
            style={StyleSheet.absoluteFill}
            facing="back"
            accessibilityLabel={t('scanView')}
            barcodeScannerSettings={{ barcodeTypes: ['qr'] }}
            onBarcodeScanned={({ data }) => {
              if (taken.current) return;
              const qr = parsePairingUri(data);
              if (!qr) return setWrong(true);
              taken.current = true;
              remote.setPairingQr(qr);
              router.replace(explained ? '/pair/name' : '/pair/network');
            }}
          />
        )}
      </View>

      {wrong && (
        <View accessible accessibilityLiveRegion="polite">
          <Body muted>{t('scanWrong')}</Body>
        </View>
      )}
    </Screen>
  );
}

const styles = StyleSheet.create({
  frame: {
    width: '100%',
    aspectRatio: 1,
    borderRadius: 20,
    borderWidth: 2,
    overflow: 'hidden',
    backgroundColor: '#000000',
  },
});
