import { useState } from 'react';
import * as D from '../../design';
import { Spec, State } from '../Spec';
const avatar = new URL('../../assets/avatars/01.jpg', import.meta.url).href;
export function DesignFeedback() {
  const [dialog, setDialog] = useState<'md' | 'sm' | 'confirm' | null>(null);
  const close = () => setDialog(null);
  const toast = D.useToast();
  return (
    <>
      <Spec name="Tooltip">
        <D.Tooltip label="Help">
          <D.Button>Hover me</D.Button>
        </D.Tooltip>
      </Spec>
      <Spec name="Toast">
        <D.Button
          onClick={() => toast({ message: 'Saved', action: { label: 'Undo', onAction: () => {} } })}
        >
          With action
        </D.Button>
        <D.Button onClick={() => toast({ message: 'Notice' })}>Plain</D.Button>
      </Spec>
      <Spec name="Dialog">
        <D.Button onClick={() => setDialog('md')}>Dialog</D.Button>
        <D.Button onClick={() => setDialog('sm')}>Small dialog</D.Button>
        <D.Button onClick={() => setDialog('confirm')}>Confirm</D.Button>
      </Spec>
      <D.Dialog
        open={dialog === 'md' || dialog === 'sm'}
        size={dialog === 'sm' ? 'sm' : undefined}
        onClose={close}
        title="Dialog"
        footer={<D.Button onClick={close}>Close</D.Button>}
      >
        Content
      </D.Dialog>
      <D.ConfirmDialog
        open={dialog === 'confirm'}
        onClose={close}
        title="Delete?"
        body="This can't be undone."
        confirmLabel="Delete"
        cancelLabel="Cancel"
        onConfirm={close}
        tone="danger"
      />
      <Spec name="Callout">
        <State label="quiet">
          <D.Callout tone="quiet">Paused</D.Callout>
        </State>
        <State label="attention">
          <D.Callout
            tone="attention"
            leading={<D.Icon name="ban" />}
            action={<D.Button size="sm">Resume</D.Button>}
          >
            Needs attention
          </D.Callout>
        </State>
      </Spec>
      <Spec name="Command" surface="surface">
        <D.Command
          text="curl -fsSL meerkat.shens.dev/i | sh"
          copyLabel="Copy"
          copiedLabel="Copied"
        />
        <div style={{ width: 320 }}>
          <D.Command
            text="meerkat computer pair --server wss://meerkat.shens.dev/ws --code 7KQ2-M9XD --name office-mini"
            copyLabel="Copy"
            copiedLabel="Copied"
          />
        </div>
      </Spec>
      <Spec name="Dashes">
        {[0, 2, 4].map((current) => (
          <State key={current} label={`step ${current + 1}`}>
            <D.Dashes total={5} current={current} />
          </State>
        ))}
      </Spec>
      <Spec name="StepHeading" surface="surface">
        <State label="title">
          <D.StepHeading>Connect Feishu</D.StepHeading>
        </State>
        <State label="with line">
          <D.StepHeading sub="They take work, reply and tick off tasks.">
            Let AI join the team
          </D.StepHeading>
        </State>
      </Spec>
      <Spec name="Skeleton and Spinner">
        <State label="line">
          <D.Skeleton shape="line" width={120} />
        </State>
        <State label="circle">
          <D.Skeleton shape="circle" width={40} height={40} />
        </State>
        <State label="spinner sm">
          <D.Spinner />
        </State>
        <State label="spinner md">
          <D.Spinner size="md" />
        </State>
      </Spec>
      <Spec name="EmptyState" surface="surface">
        <D.EmptyState title="No members yet" action={<D.Button>Add</D.Button>} />
      </Spec>
      <Spec name="Message" surface="surface">
        <State label="them">
          <D.Message from="them" avatar={avatar}>
            Hi, I'm here.
          </D.Message>
        </State>
        <State label="me">
          <D.Message from="me">Hello</D.Message>
        </State>
        <State label="typing">
          <D.Message from="them" avatar={avatar} typing>
            {null}
          </D.Message>
        </State>
        <State label="done">
          <D.Message from="them" variant="done" animate={false}>
            Reply to the team
          </D.Message>
        </State>
      </Spec>
    </>
  );
}
