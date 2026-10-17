import { useSidebarLayout } from './useSidebarLayout';
import type { UIProps } from '../design/index';
import { Button, Icon, Tooltip, type IconName } from '../design';
import css from './NavItem.module.css';
export type NavItemProps = Omit<UIProps, 'className' | 'style'> & {
  icon: IconName;
  label: string;
  active: boolean;
  onClick: () => void;
  testId: string;
};
export function NavItem({ icon, label, active, onClick, testId, ...rest }: NavItemProps) {
  const { rail } = useSidebarLayout();
  return (
    <div className={css.root}>
      <Tooltip label={label}>
        <Button
          {...rest}
          data-testid={testId}
          aria-current={active ? 'page' : undefined}
          aria-label={label}
          onClick={onClick}
          leading={<Icon name={icon} />}
        >
          {rail ? null : label}
        </Button>
      </Tooltip>
    </div>
  );
}
