import type { UIProps } from './types';
import { useEffect, useRef, useState } from 'react';
import { Button } from './Button';
import css from './Command.module.css';
export type CommandProps = Omit<UIProps, 'className' | 'style'> & {
  text: string;
  copyLabel: string;
  copiedLabel: string;
};
export function Command({ text, copyLabel, copiedLabel, ...rest }: CommandProps) {
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  return (
    <div {...rest} className={css.root}>
      <code>{text}</code>
      <Button
        onClick={async () => {
          try {
            await navigator.clipboard.writeText(text);
          } catch {
            return;
          }
          setCopied(true);
          clearTimeout(timer.current);
          timer.current = setTimeout(() => setCopied(false), 2000);
        }}
      >
        <span aria-live="polite">{copied ? copiedLabel : copyLabel}</span>
      </Button>
    </div>
  );
}
