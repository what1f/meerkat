import type { ReactNode } from 'react';
import type { UIProps } from '../design/index';
import { Button, StatusDot } from '../design';
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
  return (
    <div {...rest} className={css.root} data-selected={selected}>
      {onSelect ? (
        <button type="button" aria-pressed={selected} onClick={onSelect}>
          {mark}
          {label}
        </button>
      ) : (
        <span>
          {mark}
          {label}
        </span>
      )}
      {test && <StatusDot status={test === 'run' ? 'busy' : test === 'ok' ? 'ok' : 'err'} />}
      {action && <Button onClick={action.onClick}>{action.label}</Button>}
      {selected && reply && <div>{reply}</div>}
    </div>
  );
}
