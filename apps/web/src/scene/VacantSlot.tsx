import type { UIProps } from '../design/index';

import css from './VacantSlot.module.css';
export type VacantSlotProps = Omit<UIProps, 'className' | 'style'> & {
  label: string;
  onClick: () => void;
  shape: 'badge' | 'clipboard';
};
export function VacantSlot({ label, onClick, shape, ...rest }: VacantSlotProps) {
  return (
    <button {...rest} className={css.root} type="button" onClick={onClick} data-shape={shape}>
      {label}
    </button>
  );
}
