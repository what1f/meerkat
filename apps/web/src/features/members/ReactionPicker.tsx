import { useState, type ReactElement } from 'react';
import { GridPicker, IconButton, Popover } from '../../design';
import { Reaction } from '../../scene';
import { useT } from '../../lib/i18n';
import { useReactionCatalog } from '../../lib/queries/catalogs';

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
  return (
    <Popover trigger={trigger} open={open} onOpenChange={setOpen}>
      <GridPicker
        data-testid="reaction-picker"
        columns={8}
        value={value ?? undefined}
        cells={(catalog.data?.reactions ?? [])
          .filter((r) => r.available)
          .map((r) => ({
            value: r.reaction_id,
            label: r.display_name,
            content: <Reaction id={r.reaction_id} size={28} />,
          }))}
        onPick={pick}
        leading={<IconButton icon="ban" label={t('members.noReaction')} onClick={() => pick('')} />}
      />
    </Popover>
  );
}
