import type { ReactNode } from 'react';
import css from './Spec.module.css';
// One component, every state, in light and dark side by side.
export function Spec({
  name,
  children,
  surface,
  frame,
}: {
  name: string;
  children: ReactNode;
  surface?: 'surface' | 'board';
  frame?: boolean;
}) {
  return (
    <section className={css.spec} aria-label={name}>
      <h3>{name}</h3>
      <div className={css.pair}>
        {(['light', 'dark'] as const).map((theme) => (
          <div
            key={theme}
            className={css.cell}
            data-theme={theme}
            data-surface={surface}
            data-frame={frame}
          >
            {children}
          </div>
        ))}
      </div>
    </section>
  );
}
export function State({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className={css.state}>
      <small>{label}</small>
      {children}
    </div>
  );
}
export function Group({ name, children }: { name: string; children: ReactNode }) {
  return (
    <section className={css.group} aria-label={name}>
      <h2>{name}</h2>
      {children}
    </section>
  );
}
