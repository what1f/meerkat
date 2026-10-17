import type { ReactNode } from 'react';
import type { UIProps } from './types';
import type { ReactElement } from 'react';
import { Menu as BaseMenu } from '@base-ui/react/menu';
import { Icon } from './Icon';
import css from './Menu.module.css';
export type MenuProps = Omit<UIProps, 'className' | 'style'> & {
  trigger: ReactElement;
  align?: 'start' | 'end';
  children: ReactNode;
};
export function Menu({ trigger, align = 'start', children, ...rest }: MenuProps) {
  return (
    <BaseMenu.Root>
      <BaseMenu.Trigger {...rest} className={css.root} render={trigger} />
      <BaseMenu.Portal>
        <BaseMenu.Positioner align={align}>
          <BaseMenu.Popup>{children}</BaseMenu.Popup>
        </BaseMenu.Positioner>
      </BaseMenu.Portal>
    </BaseMenu.Root>
  );
}
export type MenuItemProps = Omit<UIProps, 'className' | 'style'> & {
  leading?: ReactNode;
  checked?: boolean;
  danger?: boolean;
  onSelect: () => void;
  children: ReactNode;
};
Menu.Item = function Item({
  leading,
  checked,
  danger,
  onSelect,
  children,
  ...rest
}: MenuItemProps) {
  const content = (
    <>
      {leading}
      {children}
      {checked && <Icon name="check" />}
    </>
  );
  return checked === undefined ? (
    <BaseMenu.Item {...rest} onClick={onSelect} data-danger={danger}>
      {content}
    </BaseMenu.Item>
  ) : (
    <BaseMenu.CheckboxItem
      {...rest}
      checked={checked}
      onClick={onSelect}
      closeOnClick
      data-danger={danger}
    >
      {content}
    </BaseMenu.CheckboxItem>
  );
};
Menu.Separator = function Separator(props: Omit<UIProps, 'className' | 'style'>) {
  return <BaseMenu.Separator {...props} />;
};
