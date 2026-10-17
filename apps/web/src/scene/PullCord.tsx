import { useRef, useState, type CSSProperties } from 'react';
import type { UIProps } from '../design/index';
import css from './PullCord.module.css';
export type PullCordProps = Omit<UIProps, 'className' | 'style'> & {
  onToggle: () => void;
  label: string;
};
// Drag down and release, or just click, to toggle. A short tug that stops halfway does nothing.
export function PullCord({ onToggle, label, ...rest }: PullCordProps) {
  const start = useRef<number | null>(null);
  const [pull, setPull] = useState(0);
  const [held, setHeld] = useState(false);
  const [snaps, setSnaps] = useState(0);
  const release = (cancel: boolean) => {
    if (start.current === null) return;
    start.current = null;
    setHeld(false);
    setPull(0);
    setSnaps((n) => n + 1);
    if (!cancel && (pull > 12 || pull < 3)) onToggle();
  };
  return (
    <button
      {...rest}
      className={css.root}
      type="button"
      aria-label={label}
      data-held={held}
      data-snap={snaps === 0 ? undefined : snaps % 2}
      style={{ '--pull': `${pull * 0.9}px` } as CSSProperties}
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        start.current = e.clientY;
        setPull(0);
        setHeld(true);
      }}
      onPointerMove={(e) => {
        if (start.current !== null) setPull(Math.max(0, Math.min(26, e.clientY - start.current)));
      }}
      onPointerUp={() => release(false)}
      onPointerCancel={() => release(true)}
      onClick={(e) => e.detail === 0 && onToggle()}
    >
      <i className={css.line} />
      <i className={css.knob} />
    </button>
  );
}
