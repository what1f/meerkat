import type { CSSProperties, ReactNode } from 'react';
import type { Pose } from './figures';
import fish from '../assets/office/props/fish.webp';
import blanket from '../assets/office/props/blanket.webp';
import smoke from '../assets/office/props/smoke.webp';
import question from '../assets/office/props/question.webp';
import note from '../assets/office/props/note.webp';
import keycap from '../assets/office/props/keycap.webp';
import zzz from '../assets/office/props/zzz.webp';
import speech from '../assets/office/props/bubble-speech.webp';
import thought from '../assets/office/props/bubble-thought.webp';
import plane from '../assets/office/props/plane.webp';
import codeShot from '../assets/office/screens/code.webp';
import searchShot from '../assets/office/screens/search.webp';
import chatShot from '../assets/office/screens/chat.webp';
import errorShot from '../assets/office/screens/error.webp';
import doneShot from '../assets/office/screens/done.webp';
import videoShot from '../assets/office/screens/video.webp';
import dashboardShot from '../assets/office/screens/dashboard.webp';
import css from './Seat.module.css';
import screenCss from './SeatScreen.module.css';
// `think`, `reply` and `done` wait on finer activity from the Runtime; only the design preview shows them today.
export type SeatScreen =
  'dark' | 'asleep' | 'boot' | 'alert' | 'busy' | 'think' | 'reply' | 'done' | 'ok';
export const shot = (src: string, kind: string, extra?: ReactNode) => (
  <span className={screenCss.shot} data-kind={kind}>
    <img src={src} alt="" />
    {extra}
  </span>
);
export function mainScreen(screen: SeatScreen, avatar?: string) {
  switch (screen) {
    case 'busy':
      return shot(codeShot, 'code');
    case 'boot':
      return (
        <span className={screenCss.boot}>
          <i />
        </span>
      );
    case 'think':
      return shot(searchShot, 'search', <i />);
    // The reply goes out as a new bubble at the foot of the chat.
    case 'reply':
      return shot(chatShot, 'chat', <i />);
    case 'alert':
      return shot(errorShot, 'error');
    case 'done':
      return shot(doneShot, 'done');
    case 'asleep':
      return (
        <span className={screenCss.saver}>
          <i>{avatar && <img src={avatar} alt="" />}</i>
        </span>
      );
    // Idle: the screen drifts between a cat video, pong, a fish tank, and sleep while the phone is out.
    case 'ok':
      return (
        <>
          {shot(videoShot, 'video', <i />)}
          <span className={screenCss.pong}>
            <i />
            <i />
            <b />
          </span>
          <span className={screenCss.tank}>
            <img src={fish} alt="" />
            <i />
            <i />
            <i />
          </span>
          <span className={screenCss.lock} />
        </>
      );
    default:
      return null;
  }
}
// The pose for each screen; idle desks switch poses as the screen changes.
export const moods: Partial<Record<SeatScreen, Pose>> = {
  dark: 'sleep',
  asleep: 'sleep',
  boot: 'stretch',
  think: 'think',
  alert: 'fret',
  done: 'cheer',
};
export const sideScreen = () => shot(dashboardShot, 'side');
// What sits around the figure in each state, placed against the main screen.
export function MoodProps({
  screen,
  main,
}: {
  screen: SeatScreen;
  main: CSSProperties & { left: number; top: number; width: number };
}) {
  const lit = screen !== 'dark' && screen !== 'asleep';
  return (
    <>
      {screen === 'dark' && <img className={css.blanket} src={blanket} alt="" />}
      {!lit && <img className={css.zz} src={zzz} alt="" />}
      {screen === 'busy' && <img className={css.key} src={keycap} alt="" />}
      {screen === 'ok' && <img className={css.note} src={note} alt="" />}
      {screen === 'think' && <img className={css.thought} src={thought} alt="" />}
      {screen === 'reply' && (
        <>
          <img
            className={css.speech}
            style={{ left: main.left + main.width - 14, top: main.top - 4 }}
            src={speech}
            alt=""
          />
          <img
            className={css.plane}
            style={{ left: main.left + main.width - 4, top: main.top + 2 }}
            src={plane}
            alt=""
          />
        </>
      )}
      {screen === 'alert' && (
        <>
          <img className={css.huh} src={question} alt="" />
          <span
            className={css.smoke}
            style={{ left: main.left + main.width * 0.3, top: main.top - 8 }}
            aria-hidden="true"
          >
            <img src={smoke} alt="" />
            <img src={smoke} alt="" />
            <img src={smoke} alt="" />
          </span>
        </>
      )}
    </>
  );
}
