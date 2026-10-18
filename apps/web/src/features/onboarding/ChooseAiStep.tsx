import { BrandMark, LinkButton, Spinner, StepHeading, type BrandMarkProps } from '../../design';
import { Console, ConsoleScreen, HarnessRow, RigFigure } from '../../scene';
import { useT } from '../../lib/i18n';
import type { Flow } from './useOnboarding';
import css from './steps.module.css';

const marks = ['codex', 'claude-code', 'pi'];

// The console screen lists the AI found on this Mac; picking one tests it.
export function ChooseAiStep({ flow }: { flow: Flow }) {
  const t = useT();
  const { available, pick, tests, choose, loading, loadFailed, reload } = flow.ai;
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
                const test = tests[id];
                const failed = test?.state === 'fail' ? t('onboarding.testFailed') : undefined;
                return (
                  <HarnessRow
                    key={id}
                    data-testid={`harness-${id}`}
                    data-test={test?.state}
                    mark={
                      marks.includes(id) && (
                        <BrandMark brand={id as BrandMarkProps['brand']} size={22} />
                      )
                    }
                    label={harness.display_name}
                    test={test?.state}
                    reply={test?.state === 'run' ? undefined : test?.reply || failed}
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
