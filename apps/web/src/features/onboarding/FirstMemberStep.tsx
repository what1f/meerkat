import type { ComponentType } from 'react';
import { StepHeading } from '../../design';
import { useT } from '../../lib/i18n';
import type { Flow, Provider } from './useOnboarding';
import css from './steps.module.css';

// The bench members are made on, lent by the members page.
export type MemberBench = ComponentType<{
  workspace: string;
  channel: Provider;
  onJoined: (id: string) => void;
}>;

// The first member is made on the same bench as every later one; once they have
// joined the chat app the flow moves on by itself.
export function FirstMemberStep({
  flow,
  workspace,
  bench: Bench,
}: {
  flow: Flow;
  workspace: string;
  bench: MemberBench;
}) {
  const t = useT();
  return (
    <div className={css.step}>
      <StepHeading>{t('onboarding.firstMember')}</StepHeading>
      <div className={css.bench}>
        <Bench workspace={workspace} channel={flow.provider} onJoined={flow.joined} />
      </div>
    </div>
  );
}
