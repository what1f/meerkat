# Design system

The shared crate is `crates/desktop-ui`. Business UI lives in
`apps/desktop/src/ui` and should only compose primitives from that crate.
`apps/ui-preview` renders the primitives for visual review; new primitives
are added there too. This document records the target language and the current
gaps against it.

## Language

Quiet and precise. Generous whitespace, a near-neutral canvas, hairline borders,
soft layered shadows. Color is reserved for status, brand icons, avatars, and the
mascot. Craft shows through typography, spacing, and motion, not decoration.

The meerkat from the app logo is the only illustration language. It is
flat-shaded (no outlines, gradients, or 3D) in ochre `#e9b35f`, belly `#f6c875`,
and plum `#523039`.

## Tokens

### Color

`DesktopTheme` (`theme.rs`) defines light and dark neutrals: `window`,
`sidebar`, `pane`, `content`, `elevated`, `raised`, `hover`, `selected`,
`border`, `border_strong`, `text`, `text_secondary`, `text_tertiary`, `focus`,
`error`, `success`. Primary buttons already use `text` on `content`
(near-black pill in light mode), which matches the onboarding mockups.

To add:

| Token | Purpose | Replaces |
|---|---|---|
| `ok` | running, online, passed test | `0x22a447`, `0x238636`, `0x16803a`, `success` used as a dot |
| `busy` | starting, syncing | `0xb7791f` |
| `idle` | stopped, offline | `0x9a9a9a` |
| `err` | failed | `0xdc2626`, `error` used as a dot |
| `*-soft` | tinted backgrounds for the above | `0x22a4470d`, `0x16803a11`, `0xdc262611`, banner `0xff00000a` |
| `board` | pegboard canvas and hole color, shared by the members page and onboarding | — |
| `brand-*` | periwinkle `#7683d6` (also your own lanyard), ochre, plum | `0xfccc73`, `0x4e3038`, `0x3e2328` in the peek mascot |

`DesktopSteps` hard-codes its own greens (`0x86c89a`, `0x22a44714`, `0x238636`).

Avatar backgrounds use the muted palette in
[onboarding.md](onboarding.md#avatars); they are image content, not tokens.

### Typography

System UI font (`.SystemUIFont`) and JetBrains Mono. Sizes in use today: 10, 11,
12, 13, 14, 16, 18, 20, 21, 22, 24. Target scale:

| Role | Size | Weight | Current strays |
|---|---|---|---|
| Display (onboarding titles) | 28 | Semibold, `Songti SC` serif | — |
| Page title | 24 | Semibold | 21, 22 |
| Dialog title | 20 | Semibold | — |
| Subhead | 16 | Medium | — |
| Body | 14 | Regular | — |
| Secondary, labels | 13 | Regular | — |
| Caption, hints | 12 | Regular | — |
| Micro (badges, handles) | 11 | Medium | 10 |

The serif face is used only for onboarding display titles. The 18 px emoji in
the reaction picker is content, not type.

### Spacing

Gaps in use: 4, 6, 7, 8, 9, 10, 11, 12, 14, 16, 18, 24. Target scale:
4, 8, 12, 16, 24, 32, 48. Odd values (7, 9, 11) are optical tweaks; each is
reviewed visually when migrated, not replaced blindly.

### Radius

Tokens: `RADIUS_CONTROL` 10, `RADIUS_ROW` 10, `RADIUS_CARD` 20,
`RADIUS_MODAL` 25. Literals in use: 6, 8, 11, 12, 14, 15, 16, 26, 28. Target:

| Token | Value | Use |
|---|---|---|
| `RADIUS_BADGE` | 6 | small badges |
| `RADIUS_CONTROL` | 10 | buttons, inputs, rows |
| `RADIUS_TILE` | 14 | inset tiles (banners) |
| `RADIUS_CARD` | 20 | cards, settings groups |
| `RADIUS_MODAL` | 25 | dialogs |
| full | — | pills, avatars, dots |

### Elevation

`desktop_card` uses `shadow_lg`; settings groups have no shadow. Onboarding
tiles use a soft two-layer shadow (ambient plus contact). Add one
`elevation_tile` helper instead of per-call shadows.

### Size

`CONTROL_HEIGHT` 40, `BUTTON_HEIGHT` 32. Dialog widths in use: 440, 680, 720.
Standardize on small 440, medium 560, large 720.

## Components

| Component | Status |
|---|---|
| `DesktopButton` (primary, outline, ghost, icon, loading) | Keep. Add `LinkButton`, a quiet text link for **稍后设置**, **切换账号**, **高级设置**. |
| `DesktopInput`, `DesktopTextarea`, `DesktopSelect`, `DesktopCheckbox` | Keep. |
| `DesktopFormField` | Keep. |
| `DesktopBanner` | Keep. Move the error tint to `err-soft`. |
| `DesktopDialog`, header, footer | Keep. |
| `DesktopSteps` | Replace with hairline progress dashes. Used by the workspace dialog, the Agent dialog, and the old onboarding wizard, all of which the new flow replaces. |
| `DesktopChip`, `desktop_choice` | Keep. |
| `DesktopEmptyState`, `DesktopCenteredMessage` | Keep. |
| `desktop_settings_*` | Keep. |
| `DesktopAvatar` | Keep. Add built-in avatar sources. |
| `desktop_card` | Unused. Remove or adopt for onboarding tiles. |
| `DesktopAvatarGroup` | Only `apps/ui-preview` uses it. |
| Peek mascot (`apps/desktop/src/ui/components.rs`) | Move into the crate as `Mascot`; the only pose is peek from an edge. |

New primitives for onboarding:

- `OnboardingFlow`: pegboard canvas (`Board` with holes that fade out on the
  desk scenes), progress `Dashes`, centered column, fixed footer.
- `StatusDot`: the only way to render status: `ok`, `busy`, `idle`, `err`.
- `QrTag`: a QR hung on the pegboard like a tag: waiting, scanned (dimmed with
  the scanner's avatar), expired (refresh on the code). After sign-in it flips
  into your badge.
- `HarnessRow`: AI mark, name, test light (running, passed, failed), selected
  row, the reply under it, or an install action when nothing is found.
- `AvatarPicker`: **上传** followed by the built-in set.

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
sleep in front of dark screens. When the team is off duty (下班), nobody
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
- Never expose internal concepts: Agent, App, Harness, workspace ID, scopes.
  User-facing terms are 成员, 团队, 飞书, AI.
- Status is a dot or icon, never 「在线」 or 「离线」 text.
- No `·` joins. No counts unless the number itself is what the user needs.
- Every string goes through `language.text(zh, en)`.

Current strings that break these rules:

| Location | String | Fix |
|---|---|---|
| `dialogs.rs:1109` | 编辑 Agent 资料 | 编辑成员 |
| `dialogs.rs:1111` | 添加 Agent | 添加成员 |
| `dialogs.rs:1127` | 创建 Agent | 创建 |
| `dialogs.rs:1383` | 启用 Agent | removed with the old wizard |
| `dialogs.rs:816` | 已按“工作区_Agent ID_bot”生成；可修改。 | remove the hint |
| `pages.rs:529` | 已有 Agent 后不能更换 IM。 | show the IM as fixed without explanation |

## Dark mode

Every token has a dark value. Onboarding mockups are light only; the dark
canvas is a warm near-black with the color wash at lower opacity, and the
mascot keeps its colors.

## Migration order

1. Add status, brand, and radius tokens. Replace raw colors in `pages.rs`,
   `components.rs`, `controls.rs`.
2. Fix the copy violations above.
3. Snap type sizes to the scale; review spacing and radius strays visually.
4. Add `StatusDot`, `LinkButton`, `OnboardingFlow`, `QrTag`,
   `HarnessRow`, `AvatarPicker`, and `Mascot`.
5. Remove `DesktopSteps` and unused helpers with the old wizard.
