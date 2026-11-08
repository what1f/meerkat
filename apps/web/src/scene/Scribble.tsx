import type { UIProps } from '../design/index';
import css from './Scribble.module.css';
export type ScribbleProps = Omit<UIProps, 'className' | 'style'> & {
  /** Line breaks are kept. */
  text: string;
  /** below: drops left from under the note; beside: runs left from it; after: runs right from it; up: rises from above it. */
  arrow?: 'below' | 'beside' | 'after' | 'up';
};
// A note scribbled on the board, with a looping arrow pointing at what it is about.
// Give it a key of its text to draw it again when the text changes.
export function Scribble({ text, arrow = 'below', ...rest }: ScribbleProps) {
  return (
    <figure {...rest} className={css.root} data-arrow={arrow}>
      <figcaption className={css.text}>{text}</figcaption>
      <span className={css.arrow} aria-hidden="true" />
    </figure>
  );
}
