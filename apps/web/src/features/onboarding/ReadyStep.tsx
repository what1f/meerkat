import { BrandMark, Button, LinkButton } from '../../design';
import { Seat } from '../../scene';
import { useT } from '../../lib/i18n';
import { memberFigure, type Flow } from './useOnboarding';
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
  const name = flow.member?.name || flow.draft.name.trim();
  const feishu = provider === 'feishu';
  return (
    <div className={css.step}>
      <h1>{t('onboarding.ready').replace('{name}', name)}</h1>
      <div className={`${css.hero} ${css.hello}`}>
        <Seat
          setup="laptop"
          screen={replied ? 'ok' : 'busy'}
          occupant={{ figure: memberFigure }}
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
        <Button
          data-testid="onboarding-open"
          variant={replied ? 'default' : 'primary'}
          size="lg"
          leading={<BrandMark brand={provider} size={16} />}
          onClick={() =>
            window.open(chatLink(provider, flow.member?.provider_agent_config_json), '_blank')
          }
        >
          {t(feishu ? 'onboarding.openFeishu' : 'onboarding.openTelegram')}
        </Button>
      </span>
    </div>
  );
}
