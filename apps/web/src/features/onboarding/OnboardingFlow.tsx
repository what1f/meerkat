import { Button, Dashes, LinkButton } from '../../design';
import { Board } from '../../scene';
import { isDesktopShell } from '../../lib/desktopShell';
import { useT } from '../../lib/i18n';
import { useOnboarding } from './useOnboarding';
import { WelcomeStep } from './WelcomeStep';
import { ConnectStep } from './ConnectStep';
import { ChooseAiStep } from './ChooseAiStep';
import { FirstMemberStep } from './FirstMemberStep';
import { ReadyStep } from './ReadyStep';
import css from './OnboardingFlow.module.css';

const steps = [WelcomeStep, ConnectStep, ChooseAiStep, FirstMemberStep, ReadyStep] as const;

// The first launch happens on an empty board: each step hangs one thing on it.
// Onboarding takes the whole window, so it does not use the shell frame.
export function OnboardingFlow({
  navigate,
  selectWorkspace,
}: {
  navigate: (page: { kind: 'members' }) => void;
  selectWorkspace: (id: string) => void;
}) {
  const t = useT();
  const flow = useOnboarding((workspace) => {
    if (workspace) selectWorkspace(workspace);
    navigate({ kind: 'members' });
  });
  const { step, foot } = flow;
  const Body = steps[step];
  const label = step === 3 ? 'create' : step === 4 ? 'finish' : 'next';
  return (
    <div className={css.root} data-testid="onboarding" data-step={step}>
      {isDesktopShell() && <div aria-hidden="true" data-desktop-safe-area data-tauri-drag-region />}
      <Board layout="rails" holes={step === 0 || step === 4 ? 'fade' : 'on'}>
        <div className={css.column}>
          {step > 0 && <Dashes total={4} current={step - 1} />}
          <div className={css.body} key={step}>
            <Body flow={flow} />
          </div>
          {step > 0 && (
            <div className={css.foot}>
              {step < 4 && (
                <LinkButton data-testid="onboarding-later" onClick={foot.later}>
                  {t('onboarding.later')}
                </LinkButton>
              )}
              <span className={css.go}>
                {step === 4 && !foot.ready ? (
                  <LinkButton data-testid="onboarding-next" onClick={foot.next}>
                    {t('onboarding.finish')}
                  </LinkButton>
                ) : (
                  <Button
                    data-testid="onboarding-next"
                    variant="primary"
                    size="lg"
                    disabled={!foot.ready}
                    loading={foot.loading}
                    onClick={foot.next}
                  >
                    {t(`onboarding.${label}`)}
                  </Button>
                )}
              </span>
            </div>
          )}
        </div>
      </Board>
    </div>
  );
}
