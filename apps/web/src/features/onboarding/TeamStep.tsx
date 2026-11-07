import { LinkButton, StepHeading, TextInput } from '../../design';
import { AppTag, PaperLabel, Scribble } from '../../scene';
import { useT } from '../../lib/i18n';
import type { Flow } from './useOnboarding';
import css from './steps.module.css';

const botFather = 'https://t.me/BotFather';

// The team's name is taped on the board; under it hang the chat apps it can work in.
export function TeamStep({ flow }: { flow: Flow }) {
  const t = useT();
  const telegram = flow.provider === 'telegram' && !flow.workspace;
  return (
    <div className={css.step}>
      <StepHeading>{t('onboarding.teamWhere')}</StepHeading>
      <div className={css.team}>
        <PaperLabel
          data-testid="onboarding-team-name"
          value={flow.teamName}
          onCommit={flow.setTeamName}
          label={t('onboarding.teamLabel')}
        />
        {!flow.teamNamed && <Scribble arrow="beside" text={t('onboarding.nameTeam')} />}
      </div>
      <div className={css.apps} role="radiogroup" data-testid="onboarding-provider">
        {(['feishu', 'telegram'] as const).map((value) => (
          <AppTag
            key={value}
            data-testid={`onboarding-provider-${value}`}
            brand={value}
            label={t(`onboarding.${value}`)}
            selected={flow.provider === value}
            disabled={flow.locked}
            onSelect={() => flow.setProvider(value)}
          >
            {value === 'telegram' && telegram && (
              <span className={css.token}>
                <TextInput
                  data-testid="onboarding-token"
                  value={flow.token}
                  onChange={flow.setToken}
                  placeholder={t('onboarding.token')}
                  invalid={flow.tokenInvalid}
                />
              </span>
            )}
          </AppTag>
        ))}
      </div>
      {telegram && (
        <div className={css.under}>
          <span>{t('onboarding.tokenHintBefore')}</span>
          <LinkButton onClick={() => window.open(botFather, '_blank', 'noopener')}>
            {'@BotFather'}
          </LinkButton>
          <span>{t('onboarding.tokenHintAfter')}</span>
        </div>
      )}
    </div>
  );
}
