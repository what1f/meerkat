import type { ReactNode } from 'react';
import type { UIProps } from '../design/index';
import { LinkButton, Menu } from '../design';
import css from './TopBar.module.css';
export type TopBarProps = Omit<UIProps, 'className' | 'style'> & {
  title?: string;
  back?: { label: string; onClick: () => void } | { label: string; menu: ReactNode };
};
export function TopBar({ title, back, ...rest }: TopBarProps) {
  return (
    <header {...rest} className={css.root}>
      {back &&
        ('menu' in back ? (
          <Menu trigger={<LinkButton onClick={() => {}}>{back.label}</LinkButton>}>
            {back.menu}
          </Menu>
        ) : (
          <LinkButton onClick={back.onClick}>{back.label}</LinkButton>
        ))}
      {title && <h1>{title}</h1>}
    </header>
  );
}
