import type { UIProps } from '../design/index';
import { useState } from 'react';
import { Avatar } from '../design';
import css from './ChecklistRow.module.css';
export type ChecklistRowProps = Omit<UIProps, 'className' | 'style'> & {
  done: boolean;
  text: string;
  avatar?: string;
  onToggle: () => void;
};
export function ChecklistRow({ done, text, avatar, onToggle, ...rest }: ChecklistRowProps) {
  return (
    <div {...rest} className={css.root}>
      <label>
        <input type="checkbox" checked={done} onChange={onToggle} />
        {text}
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
      onSubmit={(e) => {
        e.preventDefault();
        if (text.trim()) {
          onAdd(text.trim());
          setText('');
        }
      }}
    >
      <input
        value={text}
        placeholder={placeholder}
        aria-label={placeholder}
        onChange={(e) => setText(e.target.value)}
      />
    </form>
  );
};
