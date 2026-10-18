import type { CSSProperties } from 'react';
import type { UIProps } from '../design/index';
import { Tooltip } from '../design';
import { Figure } from './Figure';
import { figureFrame, type FigurePack } from './figures';
import { desks } from './desks';
import type { CarryHandle } from './drag/useCarry';
import laptop from '../assets/office/desk-laptop.png';
import mini from '../assets/office/desk-mini.png';
import chair from '../assets/office/chair.png';
import css from './Seat.module.css';
import screenCss from './SeatScreen.module.css';
export type SeatProps = Omit<UIProps, 'className' | 'style'> & {
  setup: 'laptop' | 'mini';
  screen: 'dark' | 'asleep' | 'alert' | 'busy' | 'ok';
  occupant: { figure: string; pack?: FigurePack; seed?: string } | 'empty-chair';
  arriving?: boolean;
  label: string;
  onClick?: () => void;
  carry?: CarryHandle;
};
// Seat box and where the desk stands in it; the figure sits in the chair, centered.
const seatWidth = 220;
const seatHeight = 164;
const deskBottom = 18;
const sitter = 0.5;
const cards = {
  ok: ['', 's', 'me'],
  busy: ['s', 'me', 'typing'],
  side: ['', 's', 'long'],
} as const;
function card(lines: readonly string[]) {
  return (
    <span className={screenCss.card}>
      {lines.map((line, i) =>
        line === 'typing' ? (
          <span key={i} className={screenCss.typing}>
            <i />
            <i />
            <i />
          </span>
        ) : (
          <i key={i} data-line={line} />
        ),
      )}
    </span>
  );
}
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
  const desk = desks[setup];
  const k = seatWidth / desk.width;
  const top = seatHeight - deskBottom - desk.height * k;
  const lit = screen !== 'dark' && screen !== 'asleep';
  // Each screen is clipped to its hole so nothing shows through gaps elsewhere in the art.
  const screens = desk.screens.map((quad, i) => {
    const xs = quad.map(([x]) => x * k);
    const ys = quad.map(([, y]) => y * k);
    const l = Math.min(...xs) - 1;
    const t = Math.min(...ys) - 1;
    const clip = quad
      .map(([x, y], c) => `${x * k - l + [-1, 1, 1, -1][c]!}px ${y * k - t + [-1, -1, 1, 1][c]!}px`)
      .join(',');
    const style: CSSProperties = {
      left: l,
      top: top + t,
      width: Math.max(...xs) - l + 1,
      height: Math.max(...ys) - t + 1,
      clipPath: `polygon(${clip})`,
    };
    const state = i && screen === 'alert' ? 'ok' : screen;
    return (
      <span key={i} className={screenCss.screen} data-state={state} data-side={i > 0} style={style}>
        {lit && card(i ? cards.side : screen === 'alert' ? [] : cards[state as 'ok' | 'busy'])}
        {!i && screen === 'alert' && (
          <span className={screenCss.card}>
            <b>!</b>
          </span>
        )}
      </span>
    );
  });
  const figure = occupant === 'empty-chair' ? null : occupant.figure;
  const { pack, seed } = occupant === 'empty-chair' ? {} : occupant;
  const carried = figure ? figureFrame(figure, 'carry', sitter, pack, seed) : null;
  const who = {
    className: css.who,
    'data-who': '',
    'data-carry': carried?.src,
    'data-carry-width': carried?.width,
    'data-carry-height': carried?.height,
    'data-carry-factor': carried?.factor,
  };
  const pose = lit ? 'work' : 'sleep';
  return (
    <div
      {...rest}
      {...carry}
      className={css.root}
      data-setup={setup}
      data-screen={screen}
      data-arriving={arriving}
    >
      <span className={css.floor} />
      {screens}
      <img className={css.desk} src={setup === 'laptop' ? laptop : mini} alt="" />
      {figure ? (
        <Tooltip label={label}>
          {onClick ? (
            <button {...who} type="button" aria-label={label} onClick={onClick}>
              <Figure id={figure} pose={pose} scale={sitter} pack={pack} seed={seed} />
            </button>
          ) : (
            <span {...who}>
              <Figure id={figure} pose={pose} scale={sitter} pack={pack} seed={seed} />
            </span>
          )}
        </Tooltip>
      ) : (
        <img className={css.chair} src={chair} alt={label} />
      )}
      {figure && !lit && (
        <span className={css.zz} aria-hidden="true">
          <i>z</i>
          <i>z</i>
          <i>z</i>
        </span>
      )}
    </div>
  );
}
