// expect: type-contract
import { createRef } from 'react';
import * as D from '../src/design';
import * as S from '../src/scene';
import { t, registerMessages } from '../src/lib/i18n';
import type { UIProps } from '../src/design';
type Assert<T extends true> = T;
type NoOverrides<T> = Extract<keyof T, 'className' | 'style'> extends never ? true : false;
export type IconContract = Assert<NoOverrides<Parameters<typeof D.Icon>[0]>>;
export type BrandMarkContract = Assert<NoOverrides<Parameters<typeof D.BrandMark>[0]>>;
export type ButtonContract = Assert<NoOverrides<Parameters<typeof D.Button>[0]>>;
export type IconButtonContract = Assert<NoOverrides<Parameters<typeof D.IconButton>[0]>>;
export type LinkButtonContract = Assert<NoOverrides<Parameters<typeof D.LinkButton>[0]>>;
export type TextInputContract = Assert<NoOverrides<Parameters<typeof D.TextInput>[0]>>;
export type DocAreaContract = Assert<NoOverrides<Parameters<typeof D.DocArea>[0]>>;
export type InlineTextContract = Assert<NoOverrides<Parameters<typeof D.InlineText>[0]>>;
export type SegmentedContract = Assert<NoOverrides<Parameters<typeof D.Segmented>[0]>>;
export type SelectContract = Assert<NoOverrides<Parameters<typeof D.Select>[0]>>;
export type GroupContract = Assert<NoOverrides<Parameters<typeof D.Group>[0]>>;
export type RevealContract = Assert<NoOverrides<Parameters<typeof D.Reveal>[0]>>;
export type StatusDotContract = Assert<NoOverrides<Parameters<typeof D.StatusDot>[0]>>;
export type AvatarContract = Assert<NoOverrides<Parameters<typeof D.Avatar>[0]>>;
export type MenuContract = Assert<NoOverrides<Parameters<typeof D.Menu>[0]>>;
export type PopoverContract = Assert<NoOverrides<Parameters<typeof D.Popover>[0]>>;
export type GridPickerContract = Assert<NoOverrides<Parameters<typeof D.GridPicker>[0]>>;
export type TooltipContract = Assert<NoOverrides<Parameters<typeof D.Tooltip>[0]>>;
export type DialogContract = Assert<NoOverrides<Parameters<typeof D.Dialog>[0]>>;
export type ConfirmDialogContract = Assert<NoOverrides<Parameters<typeof D.ConfirmDialog>[0]>>;
export type ToastContract = Assert<NoOverrides<Parameters<typeof D.ToastProvider>[0]>>;
export type CommandContract = Assert<NoOverrides<Parameters<typeof D.Command>[0]>>;
export type DashesContract = Assert<NoOverrides<Parameters<typeof D.Dashes>[0]>>;
export type StepHeadingContract = Assert<NoOverrides<Parameters<typeof D.StepHeading>[0]>>;
export type SkeletonContract = Assert<NoOverrides<Parameters<typeof D.Skeleton>[0]>>;
export type SpinnerContract = Assert<NoOverrides<Parameters<typeof D.Spinner>[0]>>;
export type EmptyStateContract = Assert<NoOverrides<Parameters<typeof D.EmptyState>[0]>>;
export type CalloutContract = Assert<NoOverrides<Parameters<typeof D.Callout>[0]>>;
export type MessageContract = Assert<NoOverrides<Parameters<typeof D.Message>[0]>>;
export type QrCodeContract = Assert<NoOverrides<Parameters<typeof D.QrCode>[0]>>;
export type SearchFieldContract = Assert<NoOverrides<Parameters<typeof D.SearchField>[0]>>;
export type OptionColumnContract = Assert<NoOverrides<Parameters<typeof D.OptionColumn>[0]>>;
export type ThemeThumbContract = Assert<NoOverrides<Parameters<typeof D.ThemeThumb>[0]>>;
export type BoardContract = Assert<NoOverrides<Parameters<typeof S.Board>[0]>>;
// Things that hang on the board take a peg position for the drag hooks, never appearance.
type PegOnly<T> = T extends { style?: infer P }
  ? Exclude<keyof NonNullable<P>, 'left' | 'top'> extends never
    ? Extract<keyof T, 'className'> extends never
      ? true
      : false
    : false
  : false;
export type HangContract = Assert<PegOnly<Parameters<typeof S.Hang>[0]>>;
export type BadgeContract = Assert<PegOnly<Parameters<typeof S.Badge>[0]>>;
export type BadgeVersoContract = Assert<NoOverrides<Parameters<typeof S.BadgeVerso>[0]>>;
export type FlipCardContract = Assert<NoOverrides<Parameters<typeof S.FlipCard>[0]>>;
export type VacantSlotContract = Assert<PegOnly<Parameters<typeof S.VacantSlot>[0]>>;
export type TagContract = Assert<PegOnly<Parameters<typeof S.Tag>[0]>>;
export type QrTagContract = Assert<NoOverrides<Parameters<typeof S.QrTag>[0]>>;
export type TokenTagContract = Assert<NoOverrides<Parameters<typeof S.TokenTag>[0]>>;
export type PaperLabelContract = Assert<PegOnly<Parameters<typeof S.PaperLabel>[0]>>;
export type ShiftSignContract = Assert<PegOnly<Parameters<typeof S.ShiftSign>[0]>>;
export type ClipboardContract = Assert<PegOnly<Parameters<typeof S.Clipboard>[0]>>;
export type ChecklistRowContract = Assert<NoOverrides<Parameters<typeof S.ChecklistRow>[0]>>;
export type ConsoleContract = Assert<NoOverrides<Parameters<typeof S.Console>[0]>>;
export type ConsoleScreenContract = Assert<NoOverrides<Parameters<typeof S.ConsoleScreen>[0]>>;
export type ScreenReadoutContract = Assert<NoOverrides<Parameters<typeof S.ScreenReadout>[0]>>;
export type ConsoleKeyContract = Assert<NoOverrides<Parameters<typeof S.ConsoleKey>[0]>>;
export type SoulSlotContract = Assert<NoOverrides<Parameters<typeof S.SoulSlot>[0]>>;
export type LeverContract = Assert<NoOverrides<Parameters<typeof S.Lever>[0]>>;
export type FigureContract = Assert<NoOverrides<Parameters<typeof S.Figure>[0]>>;
export type SeatContract = Assert<NoOverrides<Parameters<typeof S.Seat>[0]>>;
export type WorkRowContract = Assert<NoOverrides<Parameters<typeof S.WorkRow>[0]>>;
export type AddRowContract = Assert<NoOverrides<Parameters<typeof S.AddRow>[0]>>;
export type ReactionContract = Assert<NoOverrides<Parameters<typeof S.Reaction>[0]>>;
export type MascotContract = Assert<NoOverrides<Parameters<typeof S.Mascot>[0]>>;
export type PullCordContract = Assert<NoOverrides<Parameters<typeof S.PullCord>[0]>>;
export type RigFigureContract = Assert<NoOverrides<Parameters<typeof S.RigFigure>[0]>>;
export type StickerContract = Assert<NoOverrides<Parameters<typeof S.Sticker>[0]>>;
export type HarnessRowContract = Assert<NoOverrides<Parameters<typeof S.HarnessRow>[0]>>;
declare module '../src/lib/i18n' {
  interface MessageCatalog {
    members: { name: string };
  }
}
registerMessages('members', { en: { name: 'Name' }, zh: { name: '名字' } });
t('members.name');
// @ts-expect-error Unknown message keys must fail.
t('members.missing');
// @ts-expect-error Missing translation must fail.
registerMessages('members', { en: { name: 'Name' }, zh: {} });
// @ts-expect-error Native appearance overrides must fail.
export const badButton = <D.Button style={{ display: 'none' }}>X</D.Button>;
export const badBoard = (
  // @ts-expect-error Native appearance overrides must fail.
  <S.Board layout="rails" className="x">
    X
  </S.Board>
);
// @ts-expect-error Hang only takes a peg position.
export const badHang = <S.Hang style={{ opacity: 0 }}>X</S.Hang>;
// @ts-expect-error Tag only takes a peg position.
export const badTag = <S.Tag style={{ opacity: 0 }}>X</S.Tag>;
// @ts-expect-error Unsafe HTML cannot pass through UIProps.
export const unsafe: UIProps = { dangerouslySetInnerHTML: { __html: 'x' } };
export const triggerRefs = (
  <>
    <D.Button ref={createRef<HTMLButtonElement>()}>Button</D.Button>
    <D.IconButton ref={createRef<HTMLButtonElement>()} icon="plus" label="Add" onClick={() => {}} />
    <D.LinkButton ref={createRef<HTMLButtonElement>()} onClick={() => {}}>
      Link
    </D.LinkButton>
    <D.Avatar ref={createRef<HTMLButtonElement>()} size={40} onEdit={() => {}} />
    <S.ConsoleScreen ref={createRef<HTMLButtonElement>()} variant="readout" onClick={() => {}}>
      Screen
    </S.ConsoleScreen>
    <S.ConsoleKey ref={createRef<HTMLButtonElement>()} label="Key" onClick={() => {}} />
    <S.RigFigure ref={createRef<HTMLButtonElement>()} id="01" onClick={() => {}} />
    <S.Hang ref={createRef<HTMLDivElement>()} style={{ left: 12, top: 24 }}>
      Hang
    </S.Hang>
    <D.LinkButton disclosure>Trigger</D.LinkButton>
    <S.Sticker
      ref={createRef<HTMLButtonElement>()}
      reaction={null}
      label="Sticker"
      onClick={() => {}}
    />
  </>
);
export const pegRefs = (
  <>
    <S.Tag ref={createRef<HTMLElement>()} style={{ left: 12, top: 24 }}>
      Tag
    </S.Tag>
    <S.PaperLabel
      ref={createRef<HTMLDivElement>()}
      style={{ left: 12, top: 24 }}
      value="Team"
      onCommit={() => {}}
      label="Team"
    />
    <S.Clipboard
      ref={createRef<HTMLElement>()}
      style={{ left: 12, top: 24 }}
      title="Clipboard"
      actions={null}
      footer={null}
    >
      Clipboard
    </S.Clipboard>
    <S.ShiftSign
      ref={createRef<HTMLDivElement>()}
      style={{ left: 12, top: 24 }}
      on
      onToggle={() => {}}
      onLabel="On"
      offLabel="Off"
      label="Shift"
    />
    <S.VacantSlot
      ref={createRef<HTMLButtonElement>()}
      style={{ left: 12, top: 24 }}
      label="Add"
      onClick={() => {}}
      shape="clipboard"
    />
  </>
);
const pack: S.FigurePack = {
  frames: { work: 'w.png', sleep: 's.png', carry: 'c.png', stand: 't.png' },
  scale: 0.5,
};
export const builtinFigures: string[] = Object.keys(S.figures);
export const sceneSlots = (
  <>
    <S.Hang onClick={() => {}} faded>
      Hang
    </S.Hang>
    <S.Figure id="pack" pose="stand" pack={pack} />
    <S.RigFigure id="pack" pack={pack} />
    <S.Seat setup="mini" screen="ok" occupant={{ figure: 'pack', pack }} label="Seat" />
    <S.Figure id="default" pose="work" seed="member-1" />
    <S.RigFigure id="default" seed="member-1" />
    <S.Seat
      setup="laptop"
      screen="busy"
      occupant={{ figure: 'default', seed: 'member-1' }}
      label="Seat"
    />
    <D.OptionColumn
      title="Thinking"
      control={<S.Lever levels={['Low']} value={0} onChange={() => {}} />}
    />
  </>
);
export const picked: Promise<File | null> = D.pickFile('image/png');
export const pitch: number = S.holePitch;
export const menuEditor = (
  <D.Menu trigger={<D.Button>More</D.Button>}>
    <D.Menu.Item returnFocus={false} onSelect={() => {}}>
      Rename
    </D.Menu.Item>
  </D.Menu>
);
