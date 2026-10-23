import { Redirect, router } from 'expo-router';

import { t } from '../i18n';
import { useRemote } from '../remote/app';
import { Body, Busy, Button, Notice, Screen, statusText, Title } from '../ui';

/** After connecting: the chosen PC and what it plays. A placeholder until the Now playing screen lands. */
export default function Now() {
  const ready = useRemote((s) => s.ready);
  const paired = useRemote((s) => s.pcs.length > 0);
  const pc = useRemote((s) => s.pcs.find((p) => p.id === s.activeId) ?? null);
  const status = useRemote((s) => s.status);
  const problem = useRemote((s) => s.problem);
  const track = useRemote((s) => s.player?.track ?? null);
  if (ready && !pc) return <Redirect href={paired ? '/pcs' : '/pair'} />;
  if (!pc) return null;
  return (
    <Screen>
      <Title>{t('nowPlaying', { name: pc.name })}</Title>
      {status === 'connecting' ? <Busy label={statusText(status)} /> : <Body muted>{statusText(status)}</Body>}
      {problem && <Notice problem={problem} />}
      {problem === 'removed' && <Button label={t('pairAgain')} onPress={() => router.push('/pair')} />}
      {status === 'connected' &&
        (track ? (
          <>
            <Title>{track.title}</Title>
            <Body muted>{track.artist}</Body>
          </>
        ) : (
          <Body muted>{t('nowNothing')}</Body>
        ))}
      <Button secondary label={t('nowPcs')} hint={t('nowPcsHint')} onPress={() => router.push('/pcs')} />
    </Screen>
  );
}
