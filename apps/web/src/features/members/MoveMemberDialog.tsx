import { useState } from 'react';
import type * as DTO from '../../lib/api/generated';
import { Button, Dialog } from '../../design';
import { useT } from '../../lib/i18n';
import { BuiltinSignInDialog } from '../builtin-agent';
import { BrainColumns } from './BrainColumns';
import { useBrainDraft } from './useBrainDraft';
import type { Draft } from './draft';
import './messages';

export type MoveBrain = { harness_id: string; model: string; thinking_level: string };
export type MoveMemberDialogProps = {
  agent: DTO.Agent;
  target: DTO.MachineCatalogItem;
  open: boolean;
  onClose: () => void;
  onConfirm: (brain: MoveBrain) => void;
};

// The AI a member keeps on the new computer when it is there, else the first one it offers.
const seedFor = (agent: DTO.Agent, target: DTO.MachineCatalogItem): Partial<Draft> => {
  const kept = target.harnesses.some(
    (h) => h.harness_id === agent.harness_id && h.availability === 'available',
  );
  const first = target.harnesses.find((h) => h.availability === 'available') ?? target.harnesses[0];
  return kept
    ? { machine_id: target.machine_id }
    : {
        machine_id: target.machine_id,
        harness_id: first?.harness_id ?? agent.harness_id,
        model: '',
        thinking_level: '',
      };
};

/** Picks the AI a member runs on the computer they are being moved to. */
export function MoveMemberDialog({
  agent,
  target,
  open,
  onClose,
  onConfirm,
}: MoveMemberDialogProps) {
  const t = useT();
  const [seed] = useState(() => seedFor(agent, target));
  const m = useBrainDraft(agent, seed);
  const [signing, setSigning] = useState(false);
  const { draft } = m;
  const ready = !!draft.harness_id && (!!draft.model || m.models.length === 0);
  return (
    <>
      <Dialog
        data-testid="move-member"
        open={open}
        onClose={onClose}
        size="lg"
        title={t('members.moveTo').replace('{name}', target.display_name)}
        footer={
          <>
            <Button variant="quiet" onClick={onClose}>
              {t('common.cancel')}
            </Button>
            <Button
              variant="primary"
              disabled={!ready}
              onClick={() =>
                onConfirm({
                  harness_id: draft.harness_id,
                  model: draft.model,
                  thinking_level: draft.thinking_level,
                })
              }
            >
              {t('members.moveConfirm')}
            </Button>
          </>
        }
      >
        <BrainColumns m={m} onSignIn={() => setSigning(true)} />
      </Dialog>
      {signing && (
        <BuiltinSignInDialog
          machine={target}
          open={signing}
          onClose={() => setSigning(false)}
          onSignedIn={() => m.set({ model: '', thinking_level: '' })}
        />
      )}
    </>
  );
}
