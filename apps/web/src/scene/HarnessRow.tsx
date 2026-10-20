import type { ReactNode } from 'react';
import type { UIProps } from '../design/index';
import { Spinner } from '../design';
import css from './HarnessRow.module.css';
export type HarnessRowProps = Omit<UIProps, 'className' | 'style'> & {
  mark: ReactNode;
  label: string;
  test?: 'run' | 'ok' | 'fail';
  reply?: string;
  selected?: boolean;
  onSelect?: () => void;
  action?: { label: string; onClick: () => void };
};
// One AI on this Mac. Selecting it runs a test; the reply types out below.
export function HarnessRow({
  mark,
  label,
  test,
  reply,
  selected,
  onSelect,
  action,
  ...rest
}: HarnessRowProps) {
  const body = (
    <>
      <span className={css.mark}>{mark}</span>
      <span className={css.name}>{label}</span>
      {test === 'run' ? (
        <span className={css.spin}>
          <Spinner />
        </span>
      ) : (
        !action && <i className={css.led} data-test={test} />
      )}
      {action && (
        <button className={css.get} type="button" onClick={action.onClick}>
          {action.label}
        </button>
      )}
    </>
  );
  return (
    <div {...rest} className={css.root} data-selected={selected}>
      {onSelect && !action ? (
        <button className={css.row} type="button" aria-pressed={selected} onClick={onSelect}>
          {body}
        </button>
      ) : (
        <div className={css.row} data-static>
          {body}
        </div>
      )}
      {selected && reply && (
        <div key={reply} className={css.reply} data-test={test}>
          {reply}
        </div>
      )}
    </div>
  );
}
