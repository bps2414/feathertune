import { t } from '../../i18n';
import { remote, useRemote } from '../../remote/app';
import { Body, Button, resetTo, Screen, Title } from '../../ui';

export default function Done() {
  const name = useRemote((s) => s.pcs.find((pc) => pc.id === s.pairing.pcId)?.name ?? s.hello?.name ?? '');
  return (
    <Screen>
      <Title>{t('doneTitle')}</Title>
      <Body>{t('doneBody', { name })}</Body>
      <Button
        label={t('continue')}
        onPress={() => {
          remote.leavePairing();
          resetTo('/now');
        }}
      />
    </Screen>
  );
}
