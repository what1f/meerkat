import type { UIProps } from '../design/index';
import { useT } from '../lib/i18n';
import { useSidebarGrip } from './useSidebarLayout';
import css from './SidebarGrip.module.css';
export type SidebarGripProps = Omit<UIProps, 'className' | 'style'> & {};
export function SidebarGrip({ ...rest }: SidebarGripProps) {
  const t = useT();
  const { width, handlers } = useSidebarGrip();
  return (
    <div
      {...rest}
      className={css.root}
      role="separator"
      tabIndex={0}
      aria-orientation="vertical"
      aria-label={t('common.resizeSidebar')}
      aria-valuemin={160}
      aria-valuemax={480}
      aria-valuenow={width}
      {...handlers}
    />
  );
}
