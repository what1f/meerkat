import type { UIProps } from '../design/index';
import { Button, Select } from '../design';
import css from './TeamSwitcher.module.css';
export type TeamSwitcherProps = Omit<UIProps, 'className' | 'style'> & {
  teams: { id: string; name: string }[];
  current: string;
  onSelect: (id: string) => void;
  onCreate: () => void;
  createLabel: string;
};
export function TeamSwitcher({
  teams,
  current,
  onSelect,
  onCreate,
  createLabel,
  ...rest
}: TeamSwitcherProps) {
  return (
    <div {...rest} className={css.root}>
      <Select
        value={current}
        onChange={onSelect}
        options={teams.map((team) => ({ value: team.id, label: team.name }))}
      />
      <Button onClick={onCreate}>{createLabel}</Button>
    </div>
  );
}
