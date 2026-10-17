import type { UIProps } from '../design/index';
import { Figure } from './Figure';
import type { CarryHandle } from './drag/useCarry';
import laptop from '../assets/office/desk-laptop.png';
import mini from '../assets/office/desk-mini.png';
import chair from '../assets/office/chair.png';
import css from './Seat.module.css';
export type SeatProps = Omit<UIProps, 'className' | 'style'> & {
  setup: 'laptop' | 'mini';
  screen: 'dark' | 'asleep' | 'alert' | 'busy' | 'ok';
  occupant: { figure: string } | 'empty-chair';
  arriving?: boolean;
  label: string;
  onClick?: () => void;
  carry?: CarryHandle;
};
export function Seat({
  setup,
  screen,
  occupant,
  arriving,
  label,
  onClick,
  carry,
  ...rest
}: SeatProps) {
  return (
    <div
      {...rest}
      {...carry}
      className={css.root}
      data-setup={setup}
      data-screen={screen}
      data-arriving={arriving}
    >
      <img src={setup === 'laptop' ? laptop : mini} alt="" />
      {occupant === 'empty-chair' ? (
        <img src={chair} alt="" />
      ) : (
        <Figure
          id={occupant.figure}
          pose={screen === 'asleep' || screen === 'dark' ? 'sleep' : 'work'}
        />
      )}
      {onClick ? (
        <button type="button" onClick={onClick}>
          {label}
        </button>
      ) : (
        <span>{label}</span>
      )}
    </div>
  );
}
