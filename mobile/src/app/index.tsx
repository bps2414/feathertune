import { Redirect } from 'expo-router';

import { t } from '../i18n';
import { useRemote } from '../remote/app';
import { Busy, Screen } from '../ui';

/** The first screen: pairing when no PC is paired yet, otherwise the chosen PC. */
export default function Index() {
  const ready = useRemote((s) => s.ready);
  const paired = useRemote((s) => s.pcs.length > 0);
  if (!ready) {
    return (
      <Screen>
        <Busy label={t('title')} />
      </Screen>
    );
  }
  return <Redirect href={paired ? '/now' : '/pair'} />;
}
