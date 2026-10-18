import { useState } from 'react';
import { GridPicker, IconButton, Popover, pickFile, useToast } from '../../design';
import { Figure, RigFigure, figures } from '../../scene';
import { useT } from '../../lib/i18n';
import { useFigurePacks, useImportFigurePack } from '../../lib/queries/catalogs';
import { figureId, packFrames } from '../../lib/assets';

const builtin = Object.keys(figures);

// The figure standing on the console: built-in looks plus imported packs.
export function FigurePicker({
  workspace,
  value,
  seed,
  onPick,
}: {
  workspace: string;
  value: string;
  // The member id; the default look stands in as a built-in figure chosen from it.
  seed: string;
  onPick: (figure: string) => void;
}) {
  const t = useT();
  const toast = useToast();
  const [open, setOpen] = useState(false);
  const ref = { workspace_id: workspace };
  const packs = useFigurePacks({ workspace: ref });
  const importPack = useImportFigurePack();
  const current = (packs.data ?? []).find((pack) => pack.id === value);
  const pick = (figure: string) => {
    onPick(figure);
    setOpen(false);
  };
  return (
    <Popover
      open={open}
      onOpenChange={setOpen}
      align="center"
      trigger={
        <RigFigure
          id={current ? current.id : figureId(value)}
          pack={current && packFrames(current, workspace)}
          seed={seed}
          label={t('members.figure')}
          onClick={() => {}}
        />
      }
    >
      <GridPicker
        data-testid="figure-picker"
        columns={4}
        value={value === 'default' ? undefined : value}
        cells={[
          ...builtin.map((id) => ({
            value: `builtin:${id}`,
            label: id,
            content: <Figure id={id} pose="stand" scale={0.4} />,
          })),
          ...(packs.data ?? []).map((pack) => ({
            value: pack.id,
            label: pack.name,
            content: (
              <Figure id={pack.id} pose="stand" pack={packFrames(pack, workspace)} scale={0.4} />
            ),
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
