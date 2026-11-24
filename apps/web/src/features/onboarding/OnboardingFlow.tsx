import { Button, Dashes, Icon, LinkButton } from '../../design';
import { Board } from '../../scene';
import { isMacDesktopShell } from '../../lib/desktopShell';
import { useT } from '../../lib/i18n';
import { useOnboarding } from './useOnboarding';
import { WelcomeStep } from './WelcomeStep';
import { TeamStep } from './TeamStep';
import { FirstMemberStep, type MemberBench } from './FirstMemberStep';
import { ReadyStep } from './ReadyStep';
import css from './OnboardingFlow.module.css';

const last = 3;

// The first launch happens on an empty board: each step hangs one thing on it.
// Onboarding takes the whole window, so it does not use the shell frame.
export function OnboardingFlow({
  navigate,
  selectWorkspace,
  memberBench,
}: {
  navigate: (page: { kind: 'members' }) => void;
  selectWorkspace: (id: string) => void;
  memberBench: MemberBench;
}) {
  const t = useT();
  const flow = useOnboarding((workspace) => {
    if (workspace) selectWorkspace(workspace);
    navigate({ kind: 'members' });
  });
  const { step, foot, workspace } = flow;
  return (
    <div className={css.root} data-testid="onboarding" data-step={step}>
      {isMacDesktopShell() && (
        <div aria-hidden="true" data-desktop-safe-area data-tauri-drag-region />
      )}
      <Board layout="rails" holes={step === 0 || step === last ? 'fade' : 'on'}>
        <div className={css.column} data-span="board">
          {step > 0 && <Dashes total={last} current={step - 1} />}
          <div className={css.body} key={step}>
            {step === 0 && <WelcomeStep flow={flow} />}
            {step === 1 && <TeamStep flow={flow} />}
            {step === last && <ReadyStep flow={flow} />}
          </div>
          {/* The member being made stays on the bench while the team is looked at again. */}
          {workspace && (
            <div className={css.body} hidden={step !== 2}>
              <FirstMemberStep flow={flow} workspace={workspace} bench={memberBench} />
            </div>
          )}
          {step > 0 && (
            <div className={css.foot}>
              {foot.back && (
                <LinkButton data-testid="onboarding-back" onClick={foot.back}>
                  <Icon name="chevl" size={12} />
                  {t('onboarding.back')}
                </LinkButton>
              )}
              <span className={css.go}>
                {step < last && (
                  <LinkButton data-testid="onboarding-later" onClick={foot.later}>
                    {t('onboarding.later')}
                  </LinkButton>
                )}
                {step === last && !foot.ready ? (
                  <LinkButton data-testid="onboarding-next" onClick={foot.next}>
                    {t('onboarding.finish')}
                  </LinkButton>
                ) : (
                  foot.primary && (
                    <Button
                      data-testid="onboarding-next"
                      variant="primary"
                      size="lg"
                      disabled={!foot.ready}
                      loading={foot.loading}
                      onClick={foot.next}
                    >
                      {t(step === last ? 'onboarding.finish' : 'onboarding.next')}
                    </Button>
                  )
                )}
              </span>
            </div>
          )}
        </div>
      </Board>
    </div>
  );
}
