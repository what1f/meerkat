import type { CSSProperties } from 'react';
import type { UIProps } from '../design/index';
import { Tooltip } from '../design';
import { Figure } from './Figure';
import { figureFrame, type FigurePack, type Pose } from './figures';
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
import css from './Seat.module.css';
import screenCss from './SeatScreen.module.css';
import { mainScreen, moods, MoodProps, sideScreen, type SeatScreen } from './SeatMood';
export type { SeatScreen } from './SeatMood';
export type SeatProps = Omit<UIProps, 'className' | 'style'> & {
  setup: 'laptop' | 'mini';
  screen: SeatScreen;
  occupant: { figure: string; pack?: FigurePack; seed?: string } | 'empty-chair';
  arriving?: boolean;
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
  screen,
  occupant,
  arriving,
  size = 'seat',
  label,
  badge,
  onClick,
  carry,
  ...rest
}: SeatProps) {
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
  // A figure pack without a desk mood keeps its working pose.
  const has = (p: Pose) => !pack || !!pack.frames[p];
  const mood = moods[screen];
  const pose = mood && has(mood) ? mood : 'work';
  const slack = screen === 'ok' ? (['sip', 'phone'] as const).filter(has) : [];
  const body = (
    <>
      <Figure id={figure ?? ''} pose={pose} scale={sitter} pack={pack} seed={seed} />
      {slack.map((p) => (
        <Figure
          key={p}
          id={figure ?? ''}
          pose={p}
          scale={sitter}
          pack={pack}
          seed={seed}
          data-pose={p}
        />
      ))}
    </>
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
      data-size={size}
      data-away={!figure}
      style={{ '--phase': phase } as CSSProperties}
    >
      <span className={css.floor} />
      {screens}
      <img className={css.desk} src={setup === 'laptop' ? laptop : mini} alt="" />
      {badge && (
        <span className={css.mug} style={mug} aria-hidden="true">
          <img src={mugArt} alt="" />
          <i />
          <i />
        </span>
      )}
      {badge && desk.plant && (
        <img
          className={css.plant}
          style={{ left: desk.plant[0] * k, top: top + desk.plant[1] * k }}
          src={plant}
          alt=""
        />
      )}
      {badge && (
        <span
          className={css.badge}
          style={
            figure
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
        <img className={css.chair} src={badge ? jacketChair : chair} alt={label} />
      )}
      {figure && <MoodProps screen={screen} main={main} />}
    </div>
  );
}
