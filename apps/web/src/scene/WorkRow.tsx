import type { Ref } from 'react';
import type { ReactNode } from 'react';
import type { UIProps, Status } from '../design/index';
import { StatusDot } from '../design';
import css from './WorkRow.module.css';
export type WorkRowProps = Omit<UIProps, 'className' | 'style'> & {
  title: ReactNode;
  status: Status;
  menu?: ReactNode;
  arriving?: boolean;
  ref?: Ref<HTMLElement>;
  children: ReactNode;
};
export function WorkRow({ title, status, menu, arriving, children, ...rest }: WorkRowProps) {
  return (
    <section {...rest} className={css.root} data-arriving={arriving}>
      <header>
        {title}
        <StatusDot status={status} />
        {menu}
      </header>
      <div>{children}</div>
    </section>
  );
}
