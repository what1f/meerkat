import { BrandMark, LinkButton, StepHeading } from '../../design';
import { Seat } from '../../scene';
import { useT } from '../../lib/i18n';
import type { Flow } from './useOnboarding';
import { ChatPreview } from './ChatPreview';
import css from './steps.module.css';

// Where the new member answers in the chat app.
function chatLink(provider: Flow['provider'], configJson = '{}') {
  const config = JSON.parse(configJson || '{}') as { app_id?: string; bot_username?: string };
  return provider === 'feishu'
    ? `https://applink.feishu.cn/client/bot/open?appId=${config.app_id ?? ''}`
    : `https://t.me/${config.bot_username ?? ''}`;
}

// The member sits at the desk; the step ends once they answer your first message.
export function ReadyStep({ flow }: { flow: Flow }) {
  const t = useT();
  const { provider, replied } = flow;
  const name = flow.member?.name ?? '';
  const figure = flow.member?.figure.replace('builtin:', '') || '01';
  const feishu = provider === 'feishu';
  return (
    <div className={css.step}>
      <StepHeading>{t('onboarding.ready').replace('{name}', name)}</StepHeading>
      <div className={`${css.hero} ${css.hello}`}>
        <Seat
          setup="laptop"
          size="hero"
          screen={replied ? 'ok' : 'busy'}
          occupant={{ figure }}
          label={name}
        />
        <ChatPreview
          waiting={
            replied ? undefined : t(feishu ? 'onboarding.sayHiFeishu' : 'onboarding.sayHiTelegram')
          }
          lines={[
            { from: 'me', text: t('onboarding.hello') },
            { from: 'them', text: t('onboarding.reply').replace('{name}', name), delay: 500 },
          ]}
        />
      </div>
      {flow.reply.failed && (
        <div className={css.under}>
          <span>{t('onboarding.loadFailed')}</span>
          <LinkButton data-testid="onboarding-retry" onClick={flow.reply.retry}>
            {t('onboarding.retry')}
          </LinkButton>
        </div>
      )}
      <span className={css.go}>
        <LinkButton
          data-testid="onboarding-open"
          external
          onClick={() =>
            window.open(chatLink(provider, flow.member?.provider_agent_config_json), '_blank')
          }
        >
          <BrandMark brand={provider} size={14} />
          {t(feishu ? 'onboarding.openFeishu' : 'onboarding.openTelegram')}
        </LinkButton>
      </span>
    </div>
  );
}
