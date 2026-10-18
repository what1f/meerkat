import { ConfirmDialog, Icon } from '../../design';
import { useT } from '../../lib/i18n';
import './messages';

export function RemoveMachineDialog({
  name,
  open,
  onClose,
  onConfirm,
}: {
  name: string;
  open: boolean;
  onClose: () => void;
  onConfirm: () => void;
}) {
  const t = useT();
  return (
    <ConfirmDialog
      data-testid="confirm-remove-machine"
      open={open}
      onClose={onClose}
      tone="danger"
      leading={<Icon name="laptop" size={22} />}
      title={t('office.removeTitle').replace('{name}', name)}
      body={t('office.removeBody')}
      confirmLabel={t('office.remove')}
      cancelLabel={t('common.cancel')}
      onConfirm={onConfirm}
    />
  );
}
