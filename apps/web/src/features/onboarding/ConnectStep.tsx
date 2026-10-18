import { BrandMark, LinkButton, Segmented, StepHeading } from '../../design';
import { TokenTag } from '../../scene';
import { useT } from '../../lib/i18n';
import type { Flow, Provider } from './useOnboarding';
import css from './steps.module.css';

const botFather = 'https://t.me/BotFather';

// Pick the chat app here; the first member uses its official creation flow.
export function ConnectStep({ flow }: { flow: Flow }) {
  const t = useT();
  const { provider } = flow;
  const feishu = provider === 'feishu';
  return (
    <div className={css.step}>
      <StepHeading>
        {t(feishu ? 'onboarding.connectFeishu' : 'onboarding.connectTelegram')}
      </StepHeading>
      <span className={css.provider}>
        <Segmented<Provider>
          data-testid="onboarding-provider"
          options={(['feishu', 'telegram'] as const).map((value) => ({
            value,
            label: t(`onboarding.${value}`),
            icon: <BrandMark brand={value} size={16} />,
          }))}
          value={provider}
          onChange={flow.setProvider}
        />
      </span>
      <div className={css.hero}>
        {!feishu ? (
          <TokenTag
            data-testid="onboarding-token"
            label={t('onboarding.token')}
            value={flow.token}
            onChange={flow.setToken}
            invalid={flow.tokenInvalid}
          />
        ) : (
          <BrandMark brand="feishu" />
        )}
      </div>
      <div className={css.under}>
        {!feishu ? (
          <>
            <span>{t('onboarding.tokenHintBefore')}</span>
            <LinkButton onClick={() => window.open(botFather, '_blank', 'noopener')}>
              {'@BotFather'}
            </LinkButton>
            <span>{t('onboarding.tokenHintAfter')}</span>
          </>
        ) : null}
      </div>
    </div>
  );
}
