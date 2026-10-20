import { Button, StepHeading } from '../../design';
import { Seat } from '../../scene';
import { useT } from '../../lib/i18n';
import { avatarUrl } from '../../lib/assets';
import type { Flow } from './useOnboarding';
import { ChatPreview } from './ChatPreview';
import css from './steps.module.css';

export function WelcomeStep({ flow }: { flow: Flow }) {
  const t = useT();
  return (
    <div className={css.step}>
      <StepHeading sub={t('onboarding.welcomeSub')}>{t('onboarding.welcomeTitle')}</StepHeading>
      <div className={`${css.hero} ${css.hello}`}>
        <Seat
          setup="laptop"
          size="hero"
          screen="busy"
          occupant={{ figure: '01' }}
          label={t('onboarding.welcomeTitle')}
        />
        <ChatPreview
          avatar={avatarUrl('01')}
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
