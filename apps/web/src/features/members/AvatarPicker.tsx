import type { ReactElement } from 'react';
import { AvatarPicker as Picker } from '../../scene';
import { useT } from '../../lib/i18n';
import { avatarChoices, choiceUpload } from '../../lib/avatars';
import type { AvatarUpload } from './draft';

export function AvatarPicker({
  onPick,
  ...rest
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onPick: (upload: AvatarUpload) => void;
  trigger: ReactElement;
}) {
  const t = useT();
  return (
    <Picker
      {...rest}
      avatars={avatarChoices}
      uploadLabel={t('members.upload')}
      onPick={async (choice) => {
        const upload = await choiceUpload(choice);
        if (upload) onPick(upload);
      }}
    />
  );
}
