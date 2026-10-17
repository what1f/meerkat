import { useId, useState } from 'react';
import * as D from '../../design';
export function DesignFeedback() {
  const descriptionId = useId();
  const [dialog, setDialog] = useState(false);
  const [smallDialog, setSmallDialog] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const toast = D.useToast();
  return (
    <>
      <span id={descriptionId}>Existing description</span>
      <D.Tooltip label="Help">
        <D.Button aria-describedby={descriptionId}>Help</D.Button>
      </D.Tooltip>
      <D.Button
        onClick={() => toast({ message: 'Saved', action: { label: 'Undo', onAction: () => {} } })}
      >
        Toast
      </D.Button>
      <D.Button onClick={() => toast({ message: 'Notice' })}>Plain toast</D.Button>
      <D.Button onClick={() => setSmallDialog(true)}>Small dialog</D.Button>
      <D.Button onClick={() => setDialog(true)}>Dialog</D.Button>
      <D.Button onClick={() => setConfirm(true)}>Confirm</D.Button>
      <D.Dialog
        open={dialog}
        onClose={() => setDialog(false)}
        title="Dialog"
        footer={<D.Button onClick={() => setDialog(false)}>Close</D.Button>}
      >
        Content
      </D.Dialog>
      <D.Dialog
        open={smallDialog}
        onClose={() => setSmallDialog(false)}
        title="Small dialog"
        size="sm"
        footer={<D.Button onClick={() => setSmallDialog(false)}>Close</D.Button>}
      >
        Content
      </D.Dialog>
      <D.ConfirmDialog
        open={confirm}
        onClose={() => setConfirm(false)}
        title="Delete?"
        body="Delete this item?"
        confirmLabel="Delete"
        cancelLabel="Cancel"
        onConfirm={() => setConfirm(false)}
        tone="danger"
      />
    </>
  );
}
