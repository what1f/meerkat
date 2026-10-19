import type { UIProps } from '../design/index';
import { Icon, Menu } from '../design';
import css from './TeamSwitcher.module.css';
export type TeamSwitcherProps = Omit<UIProps, 'className' | 'style'> & {
  teams: { id: string; name: string }[];
  current: string;
  onSelect: (id: string) => void;
  onCreate: () => void;
  createLabel: string;
  onSettings: () => void;
  settingsLabel: string;
};
// The dock keeps team actions and settings in one upward menu.
export function TeamSwitcher({
  teams,
  current,
  onSelect,
  onCreate,
  createLabel,
  onSettings,
  settingsLabel,
  ...rest
}: TeamSwitcherProps) {
  const name = teams.find((team) => team.id === current)?.name;
  return (
    <Menu
      side="top"
      trigger={
        // eslint-disable-next-line no-restricted-syntax -- The team title is a shell-only control with no design counterpart.
        <button {...rest} className={css.root} type="button">
          <span className={css.name}>{name}</span>
          <Icon name="updown" size={14} />
        </button>
      }
    >
      {teams.map((team) => (
        <Menu.Item
          key={team.id}
          checked={team.id === current}
          onSelect={() => team.id !== current && onSelect(team.id)}
        >
          {team.name}
        </Menu.Item>
      ))}
      <Menu.Separator />
      <Menu.Item leading={<Icon name="plus" size={14} />} onSelect={onCreate}>
        {createLabel}
      </Menu.Item>
      <Menu.Separator />
      <Menu.Item
        data-testid="team-settings"
        leading={<Icon name="settings" size={14} />}
        onSelect={onSettings}
      >
        {settingsLabel}
      </Menu.Item>
    </Menu>
  );
}
