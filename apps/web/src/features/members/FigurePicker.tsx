import { useState } from 'react';
import { GridPicker, IconButton, Popover, useToast } from '../../design';
import { Figure, RigFigure } from '../../scene';
import { figures } from '../../scene/figures';
import { useT } from '../../lib/i18n';
import { useFigurePacks, useImportFigurePack } from '../../lib/queries/catalogs';
import { pickFile } from './AvatarPicker';

const builtin = Object.keys(figures);
// The Runtime stores built-in looks as `builtin:<id>` and falls back to `default`.
const scene = (value: string) =>
  value.startsWith('builtin:') ? value.slice('builtin:'.length) : value;

// The figure standing on the console: built-in looks plus imported packs.
export function FigurePicker({
  workspace,
  value,
  onPick,
}: {
  workspace: string;
  value: string;
  onPick: (figure: string) => void;
}) {
  const t = useT();
  const toast = useToast();
  const [open, setOpen] = useState(false);
  const ref = { workspace_id: workspace };
  const packs = useFigurePacks({ workspace: ref });
  const importPack = useImportFigurePack();
  const shown = builtin.includes(scene(value)) ? scene(value) : (builtin[0] ?? value);
  const isPack = (packs.data ?? []).some((pack) => pack.id === value);
  const pick = (figure: string) => {
    onPick(figure);
    setOpen(false);
  };
  return (
    <Popover
      open={open}
      onOpenChange={setOpen}
      align="center"
      trigger={<RigFigure id={shown} label={t('members.figure')} onClick={() => {}} />}
    >
      <GridPicker
        data-testid="figure-picker"
        columns={4}
        value={isPack ? value : `builtin:${shown}`}
        cells={[
          ...builtin.map((id) => ({
            value: `builtin:${id}`,
            label: id,
            content: <Figure id={id} pose="stand" scale={0.4} />,
          })),
          // Figure only draws built-in looks; packs show their names until it takes a frame URL.
          ...(packs.data ?? []).map((pack) => ({
            value: pack.id,
            label: pack.name,
            content: pack.name,
          })),
        ]}
        onPick={pick}
        trailing={
          <IconButton
            icon="upload"
            label={t('members.importFigure')}
            onClick={async () => {
              const file = await pickFile('.zip,application/zip');
              if (!file) return;
              importPack.mutate(
                { workspace: ref, file },
                {
                  onSuccess: (pack) => pick(pack.id),
                  onError: () => toast({ message: t('members.failed') }),
                },
              );
            }}
          />
        }
      />
    </Popover>
  );
}
