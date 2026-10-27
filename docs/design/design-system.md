# Design system

`system.html` is the reference: tokens, every component, and the shipped app
running on sample data (`prototype.html`, rebuilt with
`node docs/design/preview/build.mjs`). The web app lives in `apps/web`; its
layers and component inventory are in
[web-frontend-components.md](../web-frontend-components.md). This page records
the rules behind them.

## Language

The app is a workshop wall. Members, rules, tasks, and settings are real
objects hung on a pegboard: badges on lanyards, a paper tag, a clipboard, a
sign, a warm-white console. You change things where they hang, not in forms.

Everything else stays quiet and precise: a near-neutral canvas, hairlines,
soft layered shadows, generous space. Color is reserved for status, brand
marks, avatars, figures, and the meerkat. The accent (periwinkle) only marks
focus and selection, and the lead's lanyard.

The meerkat is the only illustration language. It is flat-shaded (no
outlines, gradients, or 3D) in ochre `#e9b35f`, belly `#f6c875`, and plum
`#523039`.

## Principles

1. Don't explain what is obvious. Position, avatars, icons, and status dots
   come before text.
2. Finish in place. Click where a thing is to edit it there. Settings save on
   their own; member changes sync to Feishu, so they show **保存** once
   something changed. New things appear where they belong, with their progress.
3. One need, one component. Two to four fixed options use Segmented; growing
   lists use Select. An empty spot waiting for a badge is a blank sleeve; any
   other empty spot is a dashed outline.
4. Color is for life: avatars, figures, and status dots.
5. Motion only explains a change: 120 to 320 ms, a little overshoot on
   entry only. Only ongoing work loops.
6. Speak the user's language: 成员, 团队, 飞书, 工区.

## Surfaces

| Surface | Used by | Rules |
|---|---|---|
| Pegboard | 成员, 团队, a member's console, onboarding | Holes on a 24 px pitch; every object hangs from a peg on a hole. The board runs from the window's top edge to the bottom; the traffic lights sit on it. |
| Plain canvas | 工区, 设置 | No holes. 工区 is rows of desks split by hairlines; 设置 is one `Group` in a 560 px column. |
| Dialog | 添加工区, confirmations | Only for focused creation or irreversible actions. Anything undoable uses a toast with **撤销**. |
| Popover | menus, pickers | Elevated surface with `shadow-2`, 10 px radius. |

Scene objects (`apps/web/src/scene`) carry their own materials: card, sleeve,
paper, wood, rig, screen. Their paint lives in `tokens.css` as composite
tokens (`--wood`, `--paper-face`, `--rig-shadow`, …); add a token rather than
a literal. Each object keeps its look in dark mode by reading theme colors.

## Tokens

`apps/web/src/design/tokens.css` is the only place literal values live;
stylelint rejects literals anywhere else.

### Color

Neutrals: `canvas`, `surface`, `elevated`, `well`, `field`, `hover`,
`active`, `line`, `line-strong`, `edge`, `ink` to `ink-4`. Status: `ok`,
`busy`, `idle`, `err` (with `err-soft`). Accent: `accent`, `accent-soft`.
Board and objects: `board`, `hole`, `peg`, `cord`, `card`, `sleeve-*`,
`wood-*`, `paper`, `rig-*`, `wall-*`. Every themed color has a light and a
dark value.

### Typography

System fonts only; nothing is bundled.

- Body and controls: the system UI font (SF Pro, PingFang SC).
- Display: SF Pro Rounded (`ui-rounded`) for member names, titles, the team
  label, and the team switcher. Chinese stays on PingFang.
- Mono: SF Mono for literal values: commands, paths, parameters, environment
  variables, model IDs, the badge foot, and the model on the console screen.
  A model's display name is not mono; where a picker shows both, the name is
  regular and the ID mono.

| Token | Size / weight | Use |
|---|---|---|
| `t-display` | 28 / 700 rounded | onboarding titles |
| `t-heading` | 22 / 700 rounded | badge name on the stage, page titles |
| `t-title` | 15 / 600 | dialog titles, row titles |
| `t-body` | 13 / 400–500 | body |
| `t-small` | 12 / 400 | secondary text, labels |
| `t-caption` | 11 / 400 | captions, hints |

Scene objects have their own sizes (`t-label-name`, `t-model`, …). A badge
reads name, role, bio. Latin roles are uppercase with wide tracking; Chinese
roles are not tracked. Labels on tags and plates are tracked like engraving.
Digits that update in place use tabular numbers.

### Size and space

- Spacing and sizes are multiples of 4: 4, 8, 12, 16, 24, 32, 48.
- Heights: `h-sm` 28 (inline actions, quiet select, segmented), `h-md` 32
  (buttons, inputs), `h-lg` 40 (onboarding buttons, main dialog input).
  Settings rows are 40; list rows with an icon are 56.
- Avatars: 20, 26, 32, 40, 44, 60, 76, 88. A badge's avatar scales with the
  badge.
- Widths: 320 confirm, 400 dialog, 460 wide dialog, 560 single column.

### Radius

Inner radius = outer radius − padding. Buttons and tags are pills.

| Token | Value | Use |
|---|---|---|
| `r-xs` | 4 | skeletons |
| `r-sm` | 6 | menu items, tags, segment thumbs, quiet select |
| `r-md` | 8 | inputs, icon buttons, segmented |
| `r-lg` | 12 | cards, `Group`, document fields |
| `r-xl` | 16 | dialogs |
| full | — | buttons, avatars, dots, toasts |

### Elevation and motion

`shadow-1` content surfaces, `shadow-2` popovers and hover, `shadow-3`
dialogs. Hung objects use `hang-shadow`. Durations: `d-fast` 120 (hover,
press), `d-base` 200 (segments, menus, fades), `d-slow` 320 (panels, pages,
height). `ease` for everything, `ease-pop` for a slight overshoot on entry.

## Components

Each need has one component; its variants are the ones `system.html` shows.

- Button: primary (one per screen at most), default, quiet, danger (only to
  confirm a destructive action); sizes 28, 32, 40. `LinkButton` is the quiet
  text link for **稍后设置**, **高级设置**.
- Input is a filled field with an accent ring on focus. Inline-edited text
  looks like text until it is edited.
- Segmented and Select are the only choice controls. `Group` is the only
  container: label left, control right, hairlines between rows.
- `StatusDot` is the only status: running, busy (breathing), stopped, needs
  attention. It sits at the bottom-right of an avatar or icon. Paused avatars
  turn grey.
- Command copies in one click. Arguments longer than 32 characters show their
  first and last four characters; copying takes the full text.
- Loading uses skeletons. Spinners only mark a running action inside a
  control or row.
- Dialogs open with focus on the dialog itself, so no control wears a focus
  ring until the user tabs.

## Navigation

There is no sidebar. A floating glass dock sits at the bottom center: the team
switcher (opens upward), a separator, then icon buttons for 成员, 团队, and
工区, each with a tooltip. The team menu lists teams, **新建团队**, and
**设置**; ⌘, also opens settings. Content leaves enough bottom clearance that
the dock never covers the last row. A cord hangs at the top right; pulling it
toggles light and dark.

## Board objects

- **Badge.** A member is a white badge on a black lanyard: avatar, name,
  role, bio, and the AI at the foot. It sways when the pointer passes and can
  be dragged to reorder. Opening it lays the member's console over the board,
  with the badge hanging on the right as a live preview.
- **Your badge.** Every Feishu account that created a member for the team
  joins it, one badge each, on a periwinkle lanyard. Name and avatar sync from
  Feishu; the role can be edited. Clicking flips the badge: the back has the
  Feishu mark and two rows, **同步资料** and **编辑岗位**. Clicking elsewhere
  flips it back. It cannot be deleted.
- **Empty board.** The add-member slot is a blank plastic sleeve on a lanyard
  with a plus and **添加成员**. With no members it is the only thing on the
  board, centered; there is no separate empty-state message.
- **Team wall.** The team name is a paper label taped above; click to rename.
  Rules are written on a hanging paper tag. Tasks are a real checklist on a
  wooden clipboard; when disabled it becomes a dashed slot. A
  **上班/下班** sign flips on click. Off duty nobody replies. Each object can
  be dragged to another hole.
- **Flipping.** Flip cards and the shift sign hide the face turned away by
  switching visibility when edge-on, as well as with `backface-visibility`,
  since some WebKit renderers ignore the latter.

## Figures

A figure is the full-body meerkat that represents a member in the 工区
office. It is independent of the avatar: the avatar is the
Feishu App icon and the badge photo; the figure is how they appear at work.
Any member can pick any figure, even with an uploaded photo avatar.

A figure is one complete sprite set. There is no dress-up or layering; picking
a figure swaps every frame at once. Screens and the 「z」 bubble are drawn by
the app, not by the figure.

### Frames

The office is seen from behind the members, over their shoulders. All frames
share one scale, transparent PNG. `work` and `sleep` share one canvas with the
chair at the same place, so the app can swap them without moving anything;
`carry` and `stand` are trimmed.

| Frame | Pose | Used when |
|---|---|---|
| `work` | Seated on the chair, seen from behind, paws on the desk | Running, replying, needs attention |
| `sleep` | Same chair, head down on folded arms | Paused, or the workspace is offline |
| `carry` | Dangling from the scruff, nothing above the head | Being dragged to another workspace |
| `stand` | Standing, facing the viewer | Picker thumbnail |

The chair is drawn into `work` and `sleep`; the desk and computers are not.
The figure sits centered in front of a desk from `apps/web/src/assets/office/`, without a tail or
feet showing. Each desk comes with its computers, and its screens are
transparent holes; the app draws the status behind them. There are two
setups: a MacBook on a stand next to a display, and a display with a Mac mini.
A member gets the setup that matches the workspace's machine kind when it can,
otherwise a fixed pick per member; it never depends on the machine's name. The
main screen shows the status: typing when replying, a red 「!」 when the
member needs attention, dark when paused. Members of an offline workspace
sleep in front of dark screens, even one who needs attention. When the team is off duty (下班), nobody
replies: every member has gone home, so each desk shows the empty chair from
`apps/web/src/assets/office/` and all screens are off. A workspace without members has no desk.

### Pack format

A figure pack is a `.zip` with a manifest and one PNG per frame:

```
manifest.json
work.png  sleep.png  carry.png  stand.png
```

```json
{ "format": 1, "name": "Beret", "author": "…", "scale": 2 }
```

`scale` is the pixel density of the images (2 means drawn at twice the
display size). Import rejects packs with missing frames. The built-in figures
in `apps/web/src/assets/figures/` use the same layout. The desk setups are
shared office art in `apps/web/src/assets/office/`.

## Copy

- Say only what the user cannot understand the screen without. Prefer
  position, grouping, thumbnails, and icons over text.
- Never expose internal concepts: Agent, App, Bot, Harness, IM, workspace ID,
  scopes. User-facing terms are 成员, 团队, 飞书, 工区, AI.
- Status is a dot or icon, never 「在线」 or 「离线」 text.
- No `·` joins. No counts unless the number itself is what the user needs.
- Every string goes through i18n with both zh and en.

## Dark mode

Every token has a dark value. The board is a warm near-black, objects keep
their materials at lower brightness, and the meerkat keeps its colors.
