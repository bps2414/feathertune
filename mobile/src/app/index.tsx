import { StyleSheet, Text, useColorScheme, View } from 'react-native';

import { t } from '../i18n';

export default function Home() {
  const dark = useColorScheme() !== 'light';
  return (
    <View style={[styles.screen, { backgroundColor: dark ? '#111111' : '#ffffff' }]}>
      <Text style={[styles.title, { color: dark ? '#f2f2f2' : '#111111' }]}>{t('title')}</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  screen: { flex: 1, alignItems: 'center', justifyContent: 'center' },
  title: { fontSize: 20, fontWeight: '600' },
});
