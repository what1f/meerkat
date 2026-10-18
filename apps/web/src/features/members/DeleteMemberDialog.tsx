import { Avatar, ConfirmDialog } from '../../design';
import { useT } from '../../lib/i18n';

export function DeleteMemberDialog({
  open,
  name,
  avatar,
  onClose,
  onConfirm,
}: {
  open: boolean;
  name: string;
  avatar?: string;
  onClose: () => void;
  onConfirm: () => void;
}) {
  const t = useT();
  return (
    <ConfirmDialog
      open={open}
      onClose={onClose}
      leading={<Avatar src={avatar} size={44} />}
      title={t('members.deleteTitle')}
      body={name}
      confirmLabel={t('members.delete')}
      cancelLabel={t('common.cancel')}
      onConfirm={onConfirm}
      tone="danger"
      data-testid="confirm-delete-member"
    />
  );
}
