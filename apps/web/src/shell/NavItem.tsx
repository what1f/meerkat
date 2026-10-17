import { useSidebarLayout } from './useSidebarLayout';
import type { UIProps } from '../design/index';
import { Icon, Tooltip, type IconName } from '../design';
import css from './NavItem.module.css';
export type NavItemProps = Omit<UIProps, 'className' | 'style'> & {
  icon: IconName;
  label: string;
  active: boolean;
  onClick: () => void;
  testId: string;
};
// A sidebar row. In the rail only the icon shows, so the label moves to a tooltip.
export function NavItem({ icon, label, active, onClick, testId, ...rest }: NavItemProps) {
  const { rail } = useSidebarLayout();
  const row = (
    // eslint-disable-next-line no-restricted-syntax -- The nav row is a shell-only control with no design counterpart.
    <button
      {...rest}
      className={css.root}
      type="button"
      data-testid={testId}
      aria-current={active ? 'page' : undefined}
      aria-label={rail ? label : undefined}
      onClick={onClick}
    >
      <Icon name={icon} />
      {!rail && <span className={css.label}>{label}</span>}
    </button>
  );
  return rail ? <Tooltip label={label}>{row}</Tooltip> : row;
}
