import { Button } from '../../design';
import { Seat } from '../../scene';
import { useT } from '../../lib/i18n';
import type { Flow } from './useOnboarding';
import { ChatPreview } from './ChatPreview';
import css from './steps.module.css';

export function WelcomeStep({ flow }: { flow: Flow }) {
  const t = useT();
  return (
    <div className={css.step}>
      <h1>{t('onboarding.welcomeTitle')}</h1>
      <p>{t('onboarding.welcomeSub')}</p>
      <div className={`${css.hero} ${css.hello}`}>
        <Seat
          setup="laptop"
          screen="busy"
          occupant={{ figure: '01' }}
          label={t('onboarding.welcomeTitle')}
        />
        <ChatPreview
          lines={[
            { from: 'me', text: t('onboarding.welcomeAsk'), delay: 400 },
            { from: 'them', text: t('onboarding.welcomeAnswer'), delay: 1400 },
            { from: 'done', text: t('onboarding.welcomeTask'), delay: 2200 },
          ]}
        />
      </div>
      <span className={css.go}>
        <Button data-testid="onboarding-next" variant="primary" size="lg" onClick={flow.foot.next}>
          {t('onboarding.start')}
        </Button>
      </span>
    </div>
  );
}
