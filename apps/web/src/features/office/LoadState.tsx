import { Button, Callout, Skeleton } from '../../design';
import { useT } from '../../lib/i18n';
import './messages';

type Read = { isFetching: boolean; refetch: () => unknown };

// Shown until the office can be drawn: a placeholder while loading, a retry when reading failed.
export function LoadState({ failed }: { failed: Read[] }) {
  const t = useT();
  if (!failed.length) return <Skeleton shape="line" width={160} />;
  return (
    <Callout
      tone="attention"
      action={
        <Button
          size="sm"
          loading={failed.some((q) => q.isFetching)}
          onClick={() => failed.forEach((q) => void q.refetch())}
        >
          {t('office.retry')}
        </Button>
      }
    >
      {t('office.loadFailed')}
    </Callout>
  );
}
