import type { UIProps } from './types';
import { Fragment, useEffect, useRef, useState } from 'react';
import { IconButton } from './IconButton';
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
      <code>
        {text.split(' ').map((part, i) => (
          <Fragment key={i}>
            {i > 0 && ' '}
            <span>{part}</span>
          </Fragment>
        ))}
      </code>
      <IconButton
        icon={copied ? 'check' : 'copy'}
        label={copied ? copiedLabel : copyLabel}
        size="sm"
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
      />
    </div>
  );
}
