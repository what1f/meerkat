import { BrandMark, LinkButton, Spinner, StepHeading, type BrandMarkProps } from '../../design';
import { Console, ConsoleScreen, HarnessRow, RigFigure } from '../../scene';
import { useT } from '../../lib/i18n';
import type { Flow } from './useOnboarding';
import css from './steps.module.css';

const marks = ['codex', 'claude_code', 'pi'];

// The console screen lists the AI found on this Mac.
export function ChooseAiStep({ flow }: { flow: Flow }) {
  const t = useT();
  const { available, pick, choose, loading, loadFailed, reload } = flow.ai;
  return (
    <div className={css.step}>
      <StepHeading>{t('onboarding.chooseAi')}</StepHeading>
      <span className={css.rig}>
        <Console
          figure={<RigFigure id="01" />}
          screen={
            <ConsoleScreen variant="list">
              {loading ? (
                <Spinner />
              ) : (
                !loadFailed && available.length === 0 && <span>{t('onboarding.noAi')}</span>
              )}
              {available.map((harness) => {
                const id = harness.harness_id;
                return (
                  <HarnessRow
                    key={id}
                    data-testid={`harness-${id}`}
                    mark={
                      marks.includes(id) && (
                        <BrandMark brand={id as BrandMarkProps['brand']} size={22} />
                      )
                    }
                    label={harness.display_name}
                    selected={id === pick}
                    onSelect={() => choose(id)}
                  />
                );
              })}
            </ConsoleScreen>
          }
          deck={null}
        />
      </span>
      {loadFailed && (
        <div className={css.under}>
          <span>{t('onboarding.loadFailed')}</span>
          <LinkButton data-testid="onboarding-retry" onClick={reload}>
            {t('onboarding.retry')}
          </LinkButton>
        </div>
      )}
    </div>
  );
}
