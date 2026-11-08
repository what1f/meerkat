import { useEffect, type CSSProperties, type ReactElement, type ReactNode } from 'react';
import type { UIProps } from '../design/index';
import { Tooltip } from '../design';
import { Figure } from './Figure';
import { deskFigure, deskMoves, figureFrame, moveStrip, type FigurePack } from './figures';
import { desks } from './desks';
import type { CarryHandle } from './drag/useCarry';
import laptop from '../assets/office/desk-laptop.png';
import mini from '../assets/office/desk-mini.png';
import chair from '../assets/office/chair.png';
import jacketChair from '../assets/office/props/jacket-chair.webp';
import plant from '../assets/office/props/plant.webp';
import mugArt from '../assets/office/props/mug.webp';
import badgeArt from '../assets/office/props/badge.webp';
import confetti from '../assets/office/props/confetti.webp';
import movingBox from '../assets/office/props/moving-box.webp';
import css from './Seat.module.css';
import screenCss from './SeatScreen.module.css';
import { mainScreen, moves, MoodProps, sideScreen, type SeatScreen } from './SeatMood';
export type { SeatScreen } from './SeatMood';
export type SeatProps = Omit<UIProps, 'className' | 'style'> & {
  setup: 'laptop' | 'mini';
  screen: SeatScreen;
  occupant: { figure: string; pack?: FigurePack; seed?: string } | 'empty-chair';
  arriving?: boolean;
  // Moving in from another office: the chair waits and the member's things sit boxed on the floor.
  moving?: boolean;
  wrapBox?: (box: ReactElement) => ReactNode; // e.g. a menu on the box; a tooltip by default
  // `hero` is the desk at 1.35 times, for the first-run steps.
  size?: 'seat' | 'hero';
  label: string;
  // The member's badge, hanging from the monitor; it also puts the mug and plant on the desk.
  badge?: { name: string; avatar?: string };
  onClick?: () => void;
  carry?: CarryHandle;
};
// Seat box and where the desk stands in it; the figure sits in the chair, centered.
const seatWidth = 220;
const seatHeight = 164;
const deskBottom = 18;
const sitter = 0.5;
// A stable spread in [0, 1) so desks in a row never move in step.
const phaseOf = (seed: string) =>
  ([...seed].reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 11) % 997) / 997;
export function Seat({
  setup,
  screen: requested,
  occupant,
  arriving,
  moving,
  wrapBox = (box) => <Tooltip label={label}>{box}</Tooltip>,
  size = 'seat',
  label,
  badge,
  onClick,
  carry,
  ...rest
}: SeatProps) {
  const screen = moving ? 'boot' : requested;
  const desk = desks[setup];
  const k = seatWidth / desk.width;
  const top = seatHeight - deskBottom - desk.height * k;
  const lit = screen !== 'dark' && screen !== 'asleep';
  // Each screen is clipped to its hole so nothing shows through gaps elsewhere in the art.
  const rects = desk.screens.map((quad) => {
    const xs = quad.map(([x]) => x * k);
    const ys = quad.map(([, y]) => y * k);
    const l = Math.min(...xs) - 1;
    const t = Math.min(...ys) - 1;
    const clip = quad
      .map(([x, y], c) => `${x * k - l + [-1, 1, 1, -1][c]!}px ${y * k - t + [-1, -1, 1, 1][c]!}px`)
      .join(',');
    return {
      left: l,
      top: top + t,
      width: Math.max(...xs) - l + 1,
      height: Math.max(...ys) - t + 1,
      clipPath: `polygon(${clip})`,
    } satisfies CSSProperties;
  });
  const screens = rects.map((style, i) => {
    const state = i ? (lit ? 'ok' : screen) : screen;
    return (
      <span key={i} className={screenCss.screen} data-state={state} data-side={i > 0} style={style}>
        {i ? lit && sideScreen() : mainScreen(screen, badge?.avatar)}
      </span>
    );
  });
  const figure = occupant === 'empty-chair' || moving ? null : occupant.figure;
  const { pack, seed } = occupant === 'empty-chair' ? {} : occupant;
  const settled = badge && !moving;
  const carried = figure ? figureFrame(figure, 'carry', sitter, pack, seed) : null;
  const who = {
    className: css.who,
    'data-who': '',
    'data-carry': carried?.src,
    'data-carry-width': carried?.width,
    'data-carry-height': carried?.height,
    'data-carry-factor': carried?.factor,
  };
  // Built-in figures play a strip of frames per move; a figure pack sits still in its own frame.
  const live = figure ? deskFigure(figure, pack, seed) : null;
  useEffect(() => {
    // Warm every move so a change of screen never shows an empty chair.
    if (live) for (const m of deskMoves) new Image().src = moveStrip(live, m) ?? '';
  }, [live]);
  const asleep = screen === 'dark' || screen === 'asleep';
  const body = live ? (
    <span className={css.live}>
      {moves[screen].map((m) => (
        <span key={m} className={css.frames} data-move={m}>
          <img className={css.strip} src={moveStrip(live, m)} alt="" />
        </span>
      ))}
      <img className={css.back} src={chair} alt="" />
    </span>
  ) : (
    <Figure
      id={figure ?? ''}
      pose={asleep ? 'sleep' : 'work'}
      scale={sitter}
      pack={pack}
      seed={seed}
    />
  );
  const main = rects[0]!;
  const phase = phaseOf(seed ?? label);
  const mug = { left: desk.mug[0] * k, top: top + desk.mug[1] * k };
  return (
    <div
      {...rest}
      {...carry}
      className={css.root}
      data-setup={setup}
      data-screen={screen}
      data-arriving={arriving}
      data-moving={moving}
      data-size={size}
      data-away={!figure}
      style={{ '--phase': phase } as CSSProperties}
    >
      <span className={css.floor} />
      {screens}
      <img className={css.desk} src={setup === 'laptop' ? laptop : mini} alt="" />
      {settled && (
        <span className={css.mug} style={mug} aria-hidden="true">
          <img src={mugArt} alt="" />
          <i />
          <i />
        </span>
      )}
      {settled && desk.plant && (
        <img
          className={css.plant}
          style={{ left: desk.plant[0] * k, top: top + desk.plant[1] * k }}
          src={plant}
          alt=""
        />
      )}
      {moving &&
        wrapBox(
          <button className={css.box} type="button" aria-label={label} onClick={onClick}>
            <img src={movingBox} alt="" />
          </button>,
        )}
      {badge && (
        <span
          className={css.badge}
          style={
            moving
              ? { left: 68, top: 122 }
              : figure
                ? { left: main.left - 17, top: main.top - 3 }
                : { left: mug.left - 30, top: mug.top - 4 }
          }
          aria-hidden="true"
        >
          <img src={badgeArt} alt="" />
          <span className={css.face}>{badge.avatar && <img src={badge.avatar} alt="" />}</span>
          <b>{badge.name}</b>
        </span>
      )}
      {figure && screen === 'done' && <img className={css.confetti} src={confetti} alt="" />}
      {figure ? (
        <Tooltip label={label}>
          {onClick ? (
            <button {...who} type="button" aria-label={label} onClick={onClick}>
              {body}
            </button>
          ) : (
            <span {...who}>{body}</span>
          )}
        </Tooltip>
      ) : (
        <img className={css.chair} src={settled ? jacketChair : chair} alt={moving ? '' : label} />
      )}
      {figure && <MoodProps screen={screen} main={main} />}
    </div>
  );
}
