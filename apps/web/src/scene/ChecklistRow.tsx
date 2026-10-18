import type { UIProps } from '../design/index';
import { useState } from 'react';
import { Avatar, Icon } from '../design';
import css from './ChecklistRow.module.css';
export type ChecklistRowProps = Omit<UIProps, 'className' | 'style'> & {
  done: boolean;
  text: string;
  avatar?: string;
  onToggle: () => void;
};
// One task on the clipboard: ticked in pen, then struck through.
// A click shows at once; the next `done` from the parent takes over again.
export function ChecklistRow({ done, text, avatar, onToggle, ...rest }: ChecklistRowProps) {
  const [shown, setShown] = useState<boolean | null>(null);
  const [last, setLast] = useState(done);
  if (last !== done) {
    setLast(done);
    setShown(null);
  }
  const checked = shown ?? done;
  return (
    <div {...rest} className={css.root} data-done={checked}>
      <label>
        <span className={css.check}>
          <input
            type="checkbox"
            checked={checked}
            onChange={() => {
              setShown(!checked);
              onToggle();
            }}
          />
          <svg viewBox="0 0 18 18" aria-hidden="true">
            <polyline points="4.5,9.5 7.5,12.5 13.5,6" />
          </svg>
        </span>
        <span className={css.text}>{text}</span>
      </label>
      {avatar && <Avatar src={avatar} size={20} />}
    </div>
  );
}
export type ChecklistRowNewProps = Omit<UIProps, 'className' | 'style'> & {
  placeholder: string;
  onAdd: (text: string) => void;
};
ChecklistRow.New = function New({ placeholder, onAdd, ...rest }: ChecklistRowNewProps) {
  const [text, setText] = useState('');
  return (
    <form
      {...rest}
      className={css.root}
      data-new=""
      onSubmit={(e) => {
        e.preventDefault();
        if (text.trim()) {
          onAdd(text.trim());
          setText('');
        }
      }}
    >
      <label>
        <span className={css.check}>
          <Icon name="plus" size={12} />
        </span>
        <input
          value={text}
          placeholder={placeholder}
          aria-label={placeholder}
          onChange={(e) => setText(e.target.value)}
        />
      </label>
    </form>
  );
};
