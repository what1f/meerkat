import { useState, type ReactElement } from 'react';
import { GridPicker, Popover } from '../../design';
import { Reaction } from '../../scene';
import { useT } from '../../lib/i18n';
import { useReactionCatalog } from '../../lib/queries/catalogs';

// The common read receipts, in the order the design shows them.
const COMMON = [
  'OnIt',
  'Get',
  'THUMBSUP',
  'OK',
  'DONE',
  'CheckMark',
  'LGTM',
  'GLANCE',
  'Typing',
  'OneSecond',
  'THINKING',
  'SMILE',
  'CLAP',
  'APPLAUSE',
  'Fire',
  'Hundred',
  'MUSCLE',
  'FISTBUMP',
  'HIGHFIVE',
  'THANKS',
  'HEART',
  'PARTY',
  'WOW',
  'Trophy',
  'SALUTE',
  'Coffee',
  'Pin',
  'Alarm',
  'MeMeMe',
  'YEAH',
  'GoGoGo',
];

// The reaction a member leaves on messages it has read.
export function ReactionPicker({
  workspace,
  value,
  onPick,
  trigger,
}: {
  workspace: string;
  value: string | null;
  onPick: (reaction: string) => void;
  trigger: ReactElement;
}) {
  const t = useT();
  const [open, setOpen] = useState(false);
  const catalog = useReactionCatalog({
    request_id: 'reactions',
    workspace: { workspace_id: workspace },
  });
  const pick = (reaction: string) => {
    onPick(reaction);
    setOpen(false);
  };
  const available = new Map(
    (catalog.data?.reactions ?? []).filter((r) => r.available).map((r) => [r.reaction_id, r]),
  );
  // The common set leads; the rest of what the Provider offers follows, out of the way.
  const common = COMMON.filter((id) => available.has(id));
  const ids = [...common, ...[...available.keys()].filter((id) => !common.includes(id))];
  return (
    <Popover trigger={trigger} open={open} onOpenChange={setOpen}>
      <GridPicker
        data-testid="reaction-picker"
        compact
        columns={8}
        maxRows={5.5}
        value={value ?? ''}
        cells={[
          { value: '', label: t('members.noReaction'), none: true },
          ...ids.map((id) => ({
            value: id,
            label: available.get(id)?.display_name,
            content: <Reaction id={id} size={24} />,
          })),
        ]}
        onPick={pick}
      />
    </Popover>
  );
}
