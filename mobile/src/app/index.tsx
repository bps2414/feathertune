import { Redirect } from 'expo-router';
import { StyleSheet, View } from 'react-native';

import { t } from '../i18n';
import { useRemote } from '../remote/app';
import { Busy, Screen, Wordmark } from '../ui';

/** The first screen: pairing when no PC is paired yet, otherwise the chosen PC. */
export default function Index() {
  const ready = useRemote((s) => s.ready);
  const paired = useRemote((s) => s.pcs.length > 0);
  if (!ready) {
    return (
      <Screen>
        <View style={styles.center}>
          <Wordmark />
          <Busy label={t('title')} />
        </View>
      </Screen>
    );
  }
  return <Redirect href={paired ? '/now' : '/pair'} />;
}

const styles = StyleSheet.create({
  center: {
    flex: 1,
    alignItems: 'center',
    justifyContent: 'center',
    gap: 24,
    minHeight: 280,
  },
});
