import { BrandMark, LinkButton, Segmented, Spinner, StepHeading } from '../../design';
import { QrTag, TokenTag } from '../../scene';
import { useT } from '../../lib/i18n';
import type { Flow, Provider } from './useOnboarding';
import { OwnerBadge } from './OwnerBadge';
import css from './steps.module.css';

const botFather = 'https://t.me/BotFather';

// Feishu signs you in by a code that hangs on the board and turns into your badge;
// Telegram takes the token of a bot you manage.
export function ConnectStep({ flow }: { flow: Flow }) {
  const t = useT();
  const { provider, qr } = flow;
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
        ) : qr.state === 'done' ? (
          <OwnerBadge workspace={flow.workspace} provider="feishu" />
        ) : (
          <QrTag
            data-testid="onboarding-qr"
            state={qr.state}
            value={qr.value}
            brand="feishu"
            onRefresh={flow.refreshQr}
            refreshLabel={t('onboarding.refresh')}
          />
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
        ) : qr.state === 'wait' ? (
          <>
            <Spinner />
            <span>{t('onboarding.scan')}</span>
          </>
        ) : qr.state === 'scanned' ? (
          <span>{t('onboarding.confirmOnPhone')}</span>
        ) : null}
      </div>
    </div>
  );
}
