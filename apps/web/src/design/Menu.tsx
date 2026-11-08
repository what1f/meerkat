import { createContext, useContext, useRef, type ReactNode, type RefObject } from 'react';
import type { UIProps } from './types';
import type { ReactElement } from 'react';
import { Menu as BaseMenu } from '@base-ui/react/menu';
import { Icon } from './Icon';
import { useLayer } from './layer';
import css from './Menu.module.css';
export type MenuProps = Omit<UIProps, 'className' | 'style'> & {
  trigger: ReactElement;
  align?: 'start' | 'end';
  side?: 'top' | 'bottom';
  children: ReactNode;
};
// Set by an item that hands focus on, e.g. to an editor it opens in place.
const KeepFocus = createContext<RefObject<boolean> | null>(null);
export function Menu({ trigger, align = 'start', side = 'bottom', children, ...rest }: MenuProps) {
  const keep = useRef(false);
  const layer = useLayer();
  return (
    <KeepFocus value={keep}>
      <BaseMenu.Root>
        <BaseMenu.Trigger {...rest} className={css.root} render={trigger} />
        <BaseMenu.Portal container={layer.container}>
          <BaseMenu.Positioner
            positionMethod={layer.positionMethod}
            className={css.positioner}
            align={align}
            side={side}
            sideOffset={4}
          >
            <BaseMenu.Popup
              className={css.popup}
              finalFocus={() => {
                const back = !keep.current;
                keep.current = false;
                return back;
              }}
            >
              {children}
            </BaseMenu.Popup>
          </BaseMenu.Positioner>
        </BaseMenu.Portal>
      </BaseMenu.Root>
    </KeepFocus>
  );
}
export type MenuItemProps = Omit<UIProps, 'className' | 'style'> & {
  leading?: ReactNode;
  checked?: boolean;
  danger?: boolean;
  /** False when the item opens something that takes focus, such as an inline editor. */
  returnFocus?: boolean;
  onSelect: () => void;
  children: ReactNode;
};
Menu.Item = function Item({
  leading,
  checked,
  danger,
  returnFocus = true,
  onSelect: select,
  children,
  ...rest
}: MenuItemProps) {
  const keep = useContext(KeepFocus);
  const onSelect = () => {
    if (keep) keep.current = !returnFocus;
    select();
  };
  const content = (
    <>
      {leading}
      <span className={css.label}>{children}</span>
      {checked && <Icon name="check" size={14} />}
    </>
  );
  return checked === undefined ? (
    <BaseMenu.Item {...rest} className={css.item} onClick={onSelect} data-danger={danger}>
      {content}
    </BaseMenu.Item>
  ) : (
    <BaseMenu.CheckboxItem
      {...rest}
      className={css.item}
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
  return <BaseMenu.Separator {...props} className={css.separator} />;
};
