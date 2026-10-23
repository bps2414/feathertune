import { Stack } from 'expo-router';
import { StatusBar } from 'expo-status-bar';
import { useEffect } from 'react';

import { startRemote } from '../remote/app';
import { useColors } from '../ui';

export default function RootLayout() {
  const c = useColors();
  useEffect(startRemote, []);
  return (
    <>
      <StatusBar style="auto" />
      <Stack screenOptions={{ headerShown: false, contentStyle: { backgroundColor: c.bg } }} />
    </>
  );
}
