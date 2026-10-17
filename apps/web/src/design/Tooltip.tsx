import { cloneElement, isValidElement, useId, useState, type ReactNode } from 'react';
import type { UIProps } from './types';
import css from './Tooltip.module.css';
export type TooltipProps = Omit<UIProps, 'className' | 'style'> & {
  label: string;
  children: ReactNode;
};
export function Tooltip({
  label,
  children,
  onMouseEnter,
  onMouseLeave,
  onFocus,
  onBlur,
  onKeyDown,
  ...rest
}: TooltipProps) {
  const id = useId();
  const [open, setOpen] = useState(false);
  const trigger = isValidElement<{ 'aria-describedby'?: string }>(children) ? (
    cloneElement(children, {
      'aria-describedby': [children.props['aria-describedby'], id].filter(Boolean).join(' '),
    })
  ) : (
    <span tabIndex={0} aria-describedby={id}>
      {children}
    </span>
  );
  return (
    <span
      {...rest}
      className={css.root}
      onMouseEnter={(e) => {
        onMouseEnter?.(e);
        if (!e.defaultPrevented) setOpen(true);
      }}
      onMouseLeave={(e) => {
        onMouseLeave?.(e);
        if (!e.defaultPrevented) setOpen(false);
      }}
      onFocus={(e) => {
        onFocus?.(e);
        if (!e.defaultPrevented) setOpen(true);
      }}
      onBlur={(e) => {
        onBlur?.(e);
        if (!e.defaultPrevented && !e.currentTarget.contains(e.relatedTarget)) setOpen(false);
      }}
      onKeyDown={(e) => {
        onKeyDown?.(e);
        if (!e.defaultPrevented && e.key === 'Escape') setOpen(false);
      }}
    >
      {trigger}
      <span id={id} role="tooltip" hidden={!open}>
        {label}
      </span>
    </span>
  );
}
