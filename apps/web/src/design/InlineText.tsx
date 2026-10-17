import type { UIProps } from './types';
import { useRef, useState, type ChangeEvent, type KeyboardEvent } from 'react';
import css from './InlineText.module.css';
export type InlineTextProps = Omit<UIProps, 'className' | 'style'> & {
  mode?: 'commit' | 'live';
  value: string;
  onCommit?: (value: string) => void;
  onChange?: (value: string) => void;
  placeholder: string;
  multiline?: boolean;
  textStyle: 'display' | 'heading' | 'title' | 'body' | 'small';
  autoFocus?: boolean;
};
export function InlineText({
  mode = 'commit',
  value,
  onCommit,
  onChange,
  placeholder,
  multiline,
  textStyle,
  autoFocus,
  onClick,
  onBlur,
  onKeyDown,
  ...rest
}: InlineTextProps) {
  const [editing, setEditing] = useState(!!autoFocus);
  const [draft, setDraft] = useState(value);
  const finished = useRef(false);
  const live = mode === 'live';
  const finish = (commit: boolean) => {
    if (live || finished.current) return;
    finished.current = true;
    setEditing(false);
    if (commit && draft !== value) onCommit?.(draft);
  };
  if (!live && !editing)
    return (
      <button
        {...rest}
        className={css.root}
        type="button"
        onBlur={onBlur}
        onKeyDown={onKeyDown}
        data-text-style={textStyle}
        onClick={(e) => {
          onClick?.(e);
          if (e.defaultPrevented) return;
          finished.current = false;
          setDraft(value);
          setEditing(true);
        }}
      >
        {value || placeholder}
      </button>
    );
  const input = {
    onClick,
    value: live ? value : draft,
    placeholder,
    autoFocus: live ? autoFocus : true,
    onBlur: (e: React.FocusEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      onBlur?.(e);
      if (!e.defaultPrevented) finish(true);
    },
    onChange: (e: ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) =>
      live ? onChange?.(e.target.value) : setDraft(e.target.value),
    onKeyDown: (e: KeyboardEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      onKeyDown?.(e);
      if (e.defaultPrevented || e.nativeEvent.isComposing || live) return;
      if (e.key === 'Escape') {
        e.preventDefault();
        finish(false);
      }
      if (e.key === 'Enter' && (!multiline || e.metaKey)) {
        e.preventDefault();
        finish(true);
      }
    },
  };
  return multiline ? (
    <textarea
      {...rest}
      {...input}
      className={css.root}
      data-mode={mode}
      data-text-style={textStyle}
    />
  ) : (
    <input {...rest} {...input} className={css.root} data-mode={mode} data-text-style={textStyle} />
  );
}
