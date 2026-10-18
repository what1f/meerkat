import { IconButton, Icon, InlineText, Menu } from '../../design';
import { useT } from '../../lib/i18n';
import './messages';

export function MachineMenu({
  id,
  removable,
  onRename,
  onRemove,
}: {
  id: string;
  removable: boolean;
  onRename: () => void;
  onRemove: () => void;
}) {
  const t = useT();
  return (
    <Menu
      trigger={
        <IconButton
          data-testid={`machine-menu-${id}`}
          icon="more"
          label={t('office.more')}
          size="sm"
        />
      }
    >
      <Menu.Item leading={<Icon name="pencil" size={14} />} onSelect={onRename}>
        {t('office.rename')}
      </Menu.Item>
      {removable && (
        <>
          <Menu.Separator />
          <Menu.Item leading={<Icon name="trash" size={14} />} danger onSelect={onRemove}>
            {t('office.remove')}
          </Menu.Item>
        </>
      )}
    </Menu>
  );
}

// The row title turns into an editor in place while renaming.
export function MachineTitle({
  name,
  renaming,
  onCommit,
  onDone,
}: {
  name: string;
  renaming: boolean;
  onCommit: (name: string) => void;
  onDone: () => void;
}) {
  const t = useT();
  if (!renaming) return name;
  return (
    <InlineText
      autoFocus
      value={name}
      textStyle="title"
      placeholder={t('office.namePlaceholder')}
      onCommit={(next) => {
        if (next.trim() && next.trim() !== name) onCommit(next.trim());
      }}
      onBlur={onDone}
      onKeyDown={(e) => {
        if ((e.key === 'Enter' || e.key === 'Escape') && !e.nativeEvent.isComposing) onDone();
      }}
    />
  );
}
