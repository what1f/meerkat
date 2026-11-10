import { useState } from 'react';
import { BrandMark, Popover, type Status } from '../../design';
import { ConsoleScreen, ScreenReadout } from '../../scene';
import { useT } from '../../lib/i18n';
import { harnessName, levelLabel } from './labels';
import type { MemberDraftState } from './MemberConsole';
import { isBrand, useModelFace } from './MemberCard';
import { BuiltinSignInDialog } from '../builtin-agent';
import { needsSignIn } from './harnessDraft';
import { BrainColumns, brainLevels } from './BrainColumns';

export function BrainPicker({
  m,
  workspace,
  status,
}: {
  m: MemberDraftState;
  workspace: string;
  status: Status | undefined;
}) {
  const t = useT();
  const [open, setOpen] = useState(false);
  const [signing, setSigning] = useState(false);
  const { draft } = m;
  const machine = m.machine;
  const { harness, levels, level } = brainLevels(m);
  const face = useModelFace(workspace, draft.machine_id, draft.harness_id, draft.model);
  const signedOut = needsSignIn(machine, draft.harness_id);
  const place: Status = status ?? (machine?.observed_state === 'running' ? 'ok' : 'idle');

  return (
    <>
      <Popover
        open={open}
        onOpenChange={setOpen}
        trigger={
          <ConsoleScreen variant="readout" onClick={() => {}} data-testid="brain-picker">
            <ScreenReadout
              ai={{
                mark: isBrand(draft.harness_id) ? <BrandMark brand={draft.harness_id} /> : null,
                label: harnessName(harness, draft.harness_id),
              }}
              model={signedOut ? { label: t('builtinAgent.connectModel'), mark: null } : face}
              place={{ label: machine?.display_name ?? '', status: place }}
              level={{
                index: level,
                total: signedOut ? 0 : levels.length,
                label: !signedOut && levels.length ? levelLabel(levels[level]!) : '',
              }}
            />
          </ConsoleScreen>
        }
      >
        <BrainColumns
          m={m}
          place={place}
          onSignIn={() => {
            setOpen(false);
            setSigning(true);
          }}
        />
      </Popover>
      {machine && signing && (
        <BuiltinSignInDialog
          machine={machine}
          open={signing}
          onClose={() => setSigning(false)}
          onSignedIn={() => m.set({ model: '', thinking_level: '' })}
        />
      )}
    </>
  );
}
