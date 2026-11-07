import type { ReactElement } from 'react';
import { Avatar, GridPicker, IconButton, Popover, pickFile } from '../design';

export type AvatarChoice = { builtin: string } | { file: File };

// Built-in avatars in a grid, with an upload button in front.
export function AvatarPicker({
  open,
  onOpenChange,
  onPick,
  avatars,
  uploadLabel,
  trigger,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onPick: (choice: AvatarChoice) => void;
  avatars: { id: string; src?: string }[];
  uploadLabel: string;
  trigger: ReactElement;
}) {
  return (
    <Popover
      trigger={trigger}
      align="center"
      open={open}
      // The badge itself is the anchor; only the pencil on the avatar opens the picker.
      onOpenChange={(next) => !next && onOpenChange(false)}
    >
      <GridPicker
        data-testid="avatar-picker"
        columns={6}
        cells={avatars.map(({ id, src }) => ({
          value: id,
          label: id,
          content: <Avatar src={src} size={40} />,
        }))}
        onPick={(id) => onPick({ builtin: id })}
        leading={
          <IconButton
            icon="upload"
            label={uploadLabel}
            onClick={async () => {
              const file = await pickFile('image/png,image/jpeg,image/webp');
              if (file) onPick({ file });
            }}
          />
        }
      />
    </Popover>
  );
}
