import type { UIProps } from './types';
import { useLayoutEffect, useRef } from 'react';
import css from './DocArea.module.css';
export type DocAreaProps = Omit<UIProps, 'className' | 'style'> & {
  value: string;
  onChange: (value: string) => void;
  placeholder: string;
  minRows?: number;
};
export function DocArea({ value, onChange, placeholder, minRows = 3, ...rest }: DocAreaProps) {
  const ref = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (el) {
      el.style.height = 'auto';
      el.style.height = `${el.scrollHeight}px`;
    }
  }, [value, minRows]);
  return (
    <textarea
      {...rest}
      ref={ref}
      className={css.root}
      value={value}
      rows={minRows}
      placeholder={placeholder}
      onChange={(e) => onChange(e.target.value)}
    />
  );
}
