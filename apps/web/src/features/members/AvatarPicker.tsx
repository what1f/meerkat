import type { ReactElement } from 'react';
import { Avatar, GridPicker, IconButton, Popover, pickFile } from '../../design';
import { useT } from '../../lib/i18n';
import { avatarUrl } from '../../lib/assets';
import type { AvatarUpload } from './draft';
import { builtinAvatars, builtinUpload, toUpload } from './avatars';

export function AvatarPicker({
  open,
  onOpenChange,
  onPick,
  trigger,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onPick: (upload: AvatarUpload) => void;
  trigger: ReactElement;
}) {
  const t = useT();
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
        cells={builtinAvatars.map((id) => ({
          value: id,
          label: id,
          content: <Avatar src={avatarUrl(id)} size={40} />,
        }))}
        onPick={async (id) => {
          const upload = await builtinUpload(id);
          if (upload) onPick(upload);
        }}
        leading={
          <IconButton
            icon="upload"
            label={t('members.upload')}
            onClick={async () => {
              const file = await pickFile('image/png,image/jpeg,image/webp');
              if (file) onPick(await toUpload(file));
            }}
          />
        }
      />
    </Popover>
  );
}
