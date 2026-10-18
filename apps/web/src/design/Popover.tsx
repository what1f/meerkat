import type { ReactNode } from 'react';
import type { UIProps } from './types';
import type { ReactElement } from 'react';
import { Popover as BasePopover } from '@base-ui/react/popover';
import css from './Popover.module.css';
export type PopoverProps = Omit<UIProps, 'className' | 'style'> & {
  trigger: ReactElement;
  align?: 'start' | 'center' | 'end';
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  children: ReactNode;
};
export function Popover({
  trigger,
  align = 'start',
  open,
  onOpenChange,
  children,
  ...rest
}: PopoverProps) {
  return (
    <BasePopover.Root open={open} onOpenChange={onOpenChange}>
      <BasePopover.Trigger {...rest} className={css.root} render={trigger} />
      <BasePopover.Portal>
        <BasePopover.Positioner className={css.positioner} align={align} sideOffset={6}>
          <BasePopover.Popup className={css.popup}>{children}</BasePopover.Popup>
        </BasePopover.Positioner>
      </BasePopover.Portal>
    </BasePopover.Root>
  );
}
