import { Button, EmptyState } from '../../design';
import { Badge, Hang } from '../../scene';
import { useT } from '../../lib/i18n';
import type { MemberDraftState } from './MemberConsole';
import css from './MemberBench.module.css';

// What stands in for the bench until the member has been read.
export function MemberUnread({ m }: { m: MemberDraftState }) {
  const t = useT();
  if (m.notFound) return <EmptyState title={t('members.notFound')} />;
  if (m.failed)
    return (
      <EmptyState
        title={t('members.unavailable')}
        action={
          <Button loading={m.retrying} onClick={m.retry}>
            {t('members.retry')}
          </Button>
        }
      />
    );
  return (
    <div className={css.bench}>
      <span />
      <div className={css.stage}>
        <Hang>
          <Badge size="stage" avatar={{}} name={null} ghost />
        </Hang>
      </div>
    </div>
  );
}
