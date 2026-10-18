# Web 前端组件与目录规格

状态：草案。总体方案见 [web-frontend-plan.md](web-frontend-plan.md)。

这份文档在派发任务之前，先把以下内容定死：组件清单、props 接口、目录结构、依赖方向，以及由 lint 强制执行的写法。
所有实现任务只能新增本文列出的文件和组件。确实需要新组件时，先改本文，再写代码。

`docs/design/system.html` 只用来参考视觉和交互：颜色、尺寸、动效、状态长什么样。
它的 DOM 结构、字符串拼接和全局事件处理都**不能**照搬。
原型里的一个函数通常要拆成好几个组件，对应关系见文末的[原型对照](#原型对照)。

## 分层

```text
app/       入口、Provider、页面切换
  ↓
features/  业务：取数据、管理状态、组合下层组件。每个 feature 一个目录，feature 之间互不 import
  ↓
shell/     窗口外壳：侧栏、顶栏、toast 容器
  ↓
scene/     Lankide 的“实物”：洞洞板、工卡、吊牌、操控台、工位。只接 props，不取数据
  ↓
design/    通用控件：按钮、输入框、菜单、弹窗……与业务和场景都无关
lib/       api、事件、query、i18n、主题、存储。可以被 app、features、shell 使用
```

| 层              | 可以 import                                                         |
| --------------- | ------------------------------------------------------------------- |
| `design/`       | `design/` 内部、`react`、`@base-ui/react`、`qrcode`、`lucide-react` |
| `scene/`        | `design/`、`scene/` 内部                                            |
| `shell/`        | `design/`、`scene/`、`lib/`                                         |
| `features/<x>/` | `design/`、`scene/`、`lib/`、自己目录内的文件                       |
| `app/`          | 全部                                                                |
| `lib/`          | `lib/` 内部、`api/generated`                                        |
| `dev/`          | `design/`、`scene/`、`shell/`                                       |

`design/` 和 `scene/` 不 import i18n。所有用户可见的文字都由调用方通过 props 传入，这样组件不依赖任何业务文案。

## 目录结构

```text
apps/web/
  package.json  pnpm-lock.yaml  vite.config.ts  tsconfig.json
  eslint.config.js  stylelint.config.js  playwright.config.ts  index.html
  src/
    main.tsx
    app/
      App.tsx              Provider 组合：QueryClient、i18n、主题、Toast
      routes.ts            Page 类型和 hash 的解析与生成
    design/
      tokens.css           唯一的 token 来源（从 system.html 的 :root 和 [data-theme] 中整理出来）
      global.css           reset、字体、全局排版
      icons.tsx            IconName 到 lucide-react 组件的映射
      Icon.tsx  BrandMark.tsx
      Button.tsx  IconButton.tsx  LinkButton.tsx
      TextInput.tsx  DocArea.tsx  InlineText.tsx
      Segmented.tsx  Select.tsx  SearchField.tsx  OptionColumn.tsx  ThemeThumb.tsx
      Group.tsx  Reveal.tsx
      StatusDot.tsx  Avatar.tsx
      Menu.tsx  Popover.tsx  GridPicker.tsx  Tooltip.tsx
      Toast.tsx  Dialog.tsx  ConfirmDialog.tsx
      Command.tsx  Dashes.tsx  Skeleton.tsx  Spinner.tsx
      pickFile.ts          调起系统文件选择，features 不自己创建 input
      EmptyState.tsx  Callout.tsx  TrayField.tsx  Message.tsx  QrCode.tsx
      *.module.css         一个组件对应一个样式文件，同名
      index.ts             唯一出口
    scene/
      Board.tsx  Hang.tsx  Badge.tsx  VacantSlot.tsx
      Tag.tsx  QrTag.tsx  TokenTag.tsx  PaperLabel.tsx  ShiftSign.tsx
      Clipboard.tsx  ChecklistRow.tsx
      Console.tsx  ConsoleScreen.tsx  ScreenReadout.tsx  ConsoleKey.tsx  SoulSlot.tsx  Lever.tsx
      RigFigure.tsx  Sticker.tsx  HarnessRow.tsx
      Figure.tsx  Seat.tsx  WorkRow.tsx  AddRow.tsx
      Reaction.tsx  Mascot.tsx  PullCord.tsx
      figures.ts           内置形象帧的尺寸表；`figures`、`FigurePack`、`Pose` 从 scene 的 index 导出
      desks.ts             桌面素材的屏幕区域表
      drag/
        useHoleDrag.ts     自由挂板：吸附到 24px 孔位，带碰撞回退和摆动
        useSortable.ts     工卡排序：其他卡片让位，FLIP 动画
        useCarry.ts        拎起形象，拖到另一排工区
        useFlip.ts         从一个矩形飞到另一个矩形
      *.module.css
      index.ts
    shell/
      AppShell.tsx  Sidebar.tsx  SidebarGrip.tsx  NavItem.tsx  TeamSwitcher.tsx  TopBar.tsx
      useSidebarLayout.ts  宽度、图标模式、隐藏状态，存在 localStorage
      *.module.css
      index.ts
    features/
      members/  team/  office/  settings/  onboarding/
        index.ts           只导出页面组件
        messages.ts        这个 feature 的中英文案
        *.tsx  *.module.css
    lib/
      api/
        client.ts          fetch 封装和错误类型
        generated/         ts-rs 生成，不手写
      events.ts            事件 WS 的连接、重连和分发
      queries/
        keys.ts            query key，以及事件到 key 的映射
        workspaces.ts  agents.ts  machines.ts  catalogs.ts  onboarding.ts
      usePoll.ts
      i18n/
        index.ts           t()、语言状态、汇总各 feature 的 messages
        common.ts          跨 feature 的通用文案（取消、删除、保存……）
      theme.ts             system / light / dark，存在 localStorage
      desktopShell.ts      用 User-Agent 判断是否在 Tauri 外壳内
      storage.ts           带类型的 localStorage
      assets.ts            头像、形象、表情、品牌图标的 URL 解析
    assets/
      avatars/  figures/  office/  reactions/  brand/  fonts/  mascot/
    dev/
      DesignPreview.tsx    只在 dev 构建中存在的 #/design 预览页
      sections/*.tsx       每组组件一个文件，列出全部状态
  e2e/
    rig.ts                 拉起 system rig
    *.spec.ts
```

素材统一放进 `apps/web/src/assets/`，作为唯一来源，包括：

- Web 使用的 reactions、brand、mascot、fonts（旧桌面副本已删除）
- `docs/design` 下的 avatars、figures、office

`system.html` 改为引用这里的素材这件事，由设计会话负责。

## 写法约束（lint 强制）

| 约束                                                                                                      | 工具                                                         |
| --------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------ |
| 禁止使用 `dangerouslySetInnerHTML`                                                                        | ESLint `react/no-danger`                                     |
| `features/` 和 `shell/` 中禁止直接写 `button`、`input`、`textarea`、`select`、`dialog`、`img`、`svg` 元素 | ESLint `no-restricted-syntax`（JSXOpeningElement）           |
| 层之间的 import 方向必须符合上一节的表                                                                    | ESLint `import-x/no-restricted-paths`                        |
| TS/TSX 单个文件不超过 200 行（`tokens.css` 是唯一的字面值来源，不受此限制），JSX 嵌套不超过 6 层          | ESLint `max-lines`（只检查 TS/TSX）、`react/jsx-max-depth`   |
| `features/` 和 `shell/` 中的 JSX 文本不能是字面量，必须用 `t()`                                           | ESLint `i18next/no-literal-string`（只检查 JSX 文本）        |
| 颜色、圆角、阴影、字号只能写 `var(--…)`；只有 `tokens.css` 可以写字面值                                   | stylelint `scale-unlimited/declaration-strict-value`         |
| `design/` 和 `scene/` 组件的 props 不接受 `className` 和 `style`                                          | 用 TS 类型强制：props 基于 `Omit<…, 'className' \| 'style'>` |
| 原生属性透传只允许 `data-testid`、`aria-*`、事件回调                                                      | 由各组件的 props 类型声明                                    |
| 可以作为弹层触发器的组件，要接受 `ref`，并把其余 props 透传给根 DOM 元素                                  | 由 props 类型声明（见下文）                                  |

其他约定：

- 需要新外观时，给组件加 variant，不在调用处覆盖样式。
- `features/` 的 CSS Module 只写布局：`display`、`gap`、`grid`、`margin`、`padding`、`width`。
- 布局数值也用 token：间距 `--sp-sm` 14、`--sp-md` 16、`--sp-lg` 24、`--sp-xl` 32、`--sp-2xl` 48；设置类页面的内容列宽 `--column` 560；墙孔间距 `--hole-pitch` 24（TS 里用 scene 导出的 `holePitch`）。不要拿圆角、高度 token 充当间距。
- 定时器、轮询和 WS 只出现在 `lib/` 的 hook 和 `scene/drag/` 中。features 不直接写 `useEffect`
  订阅外部资源；如果必须写，在注释里说明理由。
- 动效只用 CSS transition、keyframes 和 `@starting-style`。少数 FLIP 场景用 `useFlip` 里的 Web Animations API。
- **弹层触发器**：`Menu` 和 `Popover` 的 `trigger` 通过 Base UI 的 `render` 接入。Base UI 会把 `ref`、`aria-expanded`、`data-popup-open` 和事件合并到触发元素上，所以可作为触发器的组件必须接受 React 19 的 `ref` prop，并把剩余的 props 透传到根元素（仍然禁止 `className` 和 `style`）。用作触发器时不需要传 `onClick`，所以 `IconButton` 和 `LinkButton` 的 `onClick` 是可选的。这类组件包括：`Button`、`IconButton`、`LinkButton`、`Avatar`（有 `onEdit` 时）、`ConsoleScreen`、`ConsoleKey`、`RigFigure`、`Sticker`。`WorkRow` 的 `menu` 槽位和 `Clipboard` 的 `actions` 槽位放入的就是这类组件。

## design/ 组件

除非特别说明，所有组件都透传 `data-testid` 和 `aria-*`。`Status` 统一为 `'ok' | 'busy' | 'idle' | 'err'`。

| 组件                                 | Props                                                                                                                                                                                              | 说明                                                                                                                                                                                                                                                                                                                                               |
| ------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Icon`                               | `name: IconName; size?: 10 \| 12 \| 14 \| 15 \| 16 \| 18 \| 20 \| 22`                                                                                                                              | 线条图标，颜色跟随 `currentColor`                                                                                                                                                                                                                                                                                                                  |
| `BrandMark`                          | `brand: 'feishu' \| 'telegram' \| 'codex' \| 'claude_code' \| 'pi'; size?: number`                                                                                                                 | 品牌图标。单色图标跟随墨色                                                                                                                                                                                                                                                                                                                         |
| `Button`                             | `variant?: 'default' \| 'primary' \| 'quiet' \| 'danger'; size?: 'sm' \| 'md' \| 'lg'; loading?; leading?: ReactNode; children` 加上原生 button 事件                                               | 胶囊形。一屏最多一个 primary                                                                                                                                                                                                                                                                                                                       |
| `IconButton`                         | `icon: IconName; label: string; size?: 'sm' \| 'md'; onClick?`                                                                                                                                     | `label` 同时用作 tooltip 和 aria-label                                                                                                                                                                                                                                                                                                             |
| `LinkButton`                         | `children; disclosure?: boolean; open?: boolean; onClick?`                                                                                                                                         | 文字链接；带 disclosure 时显示 chevron，展开后旋转                                                                                                                                                                                                                                                                                                 |
| `TextInput`                          | `value; onChange; size?: 'sm' \| 'md' \| 'lg'; align?: 'start' \| 'center'; mono?; invalid?; placeholder`                                                                                          | 填充框，没有描边，聚焦时显示品牌紫光圈。`sm` 用于等宽的小字，例如网络地址、环境变量                                                                                                                                                                                                                                                                |
| `DocArea`                            | `value; onChange; placeholder; minRows?`                                                                                                                                                           | 文档式多行编辑，高度随内容增长                                                                                                                                                                                                                                                                                                                     |
| `InlineText`                         | `mode?: 'commit' \| 'live'; value; onCommit?(v); onChange?(v); placeholder; multiline?; textStyle: 'display' \| 'heading' \| 'title' \| 'body' \| 'small'; autoFocus?`                             | `commit`（默认）：平时就是一段文字，点击后原地编辑，Enter 或失焦时提交，Esc 撤销。用于团队名、工区重命名、清单标题。编辑时文字下方有一条强调色底线；单行时不论是否在编辑都只和文字一样宽，不会把同一行的其他元素挤走。`live`：一直处于可编辑状态，每次输入都调用 `onChange`，不支持 Esc 撤销（撤销通过返回键的「放弃更改」）。用于成员工卡上的资料 |
| `Segmented<T>`                       | `options: { value: T; label: string; icon?: ReactNode }[]; value; onChange; size?: 'sm' \| 'md'`                                                                                                   | 固定的 2 到 4 项                                                                                                                                                                                                                                                                                                                                   |
| `Select<T>`                          | `options: { value: T; label: string; leading?: ReactNode; disabled? }[]; value; onChange; variant?: 'quiet' \| 'boxed'`                                                                            | 基于 Base UI Select；quiet 平时就是文字                                                                                                                                                                                                                                                                                                            |
| `SearchField`                        | `value; onChange; placeholder; autoFocus?; onEnter?(v)`                                                                                                                                            | 放大镜加输入框。在 `BrainPicker` 中按 Enter 可以直接采用输入的自定义模型                                                                                                                                                                                                                                                                           |
| `OptionColumn` / `OptionColumn.Item` | `OptionColumn: title: string; children?; control?: ReactNode`；`Item: selected?; dimmed?; mono?; leading?: ReactNode; hint?: string; onSelect; children`                                           | 多列选择器中的一列。`children` 只放 `Item`，组成 listbox；不是选项的控件（例如思考列的 `Lever`）放 `control`，在 listbox 之外。`dimmed` 表示离线；`hint` 只给同名选项加一个区分来源的淡色后缀，不重名的不加。每列最高 236，超出在列内滚动                                                                                                          |
| `ThemeThumb`                         | `mode: 'light' \| 'dark' \| 'system'; selected; label; onSelect`                                                                                                                                   | 外观缩略图，画的是一个小窗口                                                                                                                                                                                                                                                                                                                       |
| `Group` / `Group.Row`                | `Group: children`；`Row: label; children; align?: 'center' \| 'top'`                                                                                                                               | 唯一的容器。行高 40，行与行之间是细线                                                                                                                                                                                                                                                                                                              |
| `Reveal`                             | `open; children`                                                                                                                                                                                   | 带高度动画的折叠区域                                                                                                                                                                                                                                                                                                                               |
| `StatusDot`                          | `status: Status; tip?: string`                                                                                                                                                                     | 表达状态的唯一方式；busy 状态呼吸闪烁                                                                                                                                                                                                                                                                                                              |
| `Avatar`                             | `src?; fallback?: string; size: 20 \| 22 \| 26 \| 32 \| 40 \| 44 \| 60 \| 76 \| 88 \| 'fill'; status?: Status; statusTip?: string; dimmed?; creating?; selected?; onEdit?: () => void; editLabel?` | 状态点挂在右下角，`statusTip` 在悬停状态点时显示（例如暂停原因）。图片按 `src` 生命周期处理：加载失败时改用 `fallback`（没有则留空圆），`src` 一变就重新尝试。`fill` 表示撑满父元素，只供 scene 内部使用。`dimmed` 表示暂停（变灰）；`onEdit` 存在时悬停显示铅笔                                                                                   |
| `Menu`                               | `trigger: ReactElement; align?: 'start' \| 'end'; children`                                                                                                                                        | 基于 Base UI Menu。`trigger` 直接传触发按钮本身，不需要占位的 `onClick`，例如 `<LinkButton disclosure>`                                                                                                                                                                                                                                            |
| `Menu.Item`                          | `leading?: ReactNode; checked?; danger?; returnFocus?: boolean; onSelect; children`                                                                                                                | 菜单项高 30，选中时显示勾。菜单关闭后焦点默认回到触发按钮；打开原地编辑器（例如重命名）的项传 `returnFocus={false}`，焦点留给编辑器                                                                                                                                                                                                                |
| `Menu.Separator`                     | —                                                                                                                                                                                                  |                                                                                                                                                                                                                                                                                                                                                    |
| `Popover`                            | `trigger: ReactElement; align?; open?; onOpenChange?; children`                                                                                                                                    | 基于 Base UI Popover，供网格选择器和多列选择器使用。`Popover`、`Menu`、`Select` 的浮层都在 `--z-popup` 层，高于场景（操控台、形象、拖动中的物件）；`Tooltip` 在 `--z-tooltip`，`Toast` 在 `--z-toast`。浮层最高不超过窗口可用高度，内容多时在浮层内滚动                                                                                            |
| `GridPicker<T>`                      | `cells: { value: T; label?: string; content: ReactNode }[]; value?: T; onPick(v); leading?: ReactNode; trailing?: ReactNode; columns: number`                                                      | 头像、表情、形象选择器共用。`leading` 放上传格和“不回复”格，`trailing` 放形象的导入格                                                                                                                                                                                                                                                              |
| `Tooltip`                            | `label; children`                                                                                                                                                                                  | 对应原型里所有 `data-tip`                                                                                                                                                                                                                                                                                                                          |
| `pickFile`                           | `pickFile(accept: string): Promise<File \| null>`                                                                                                                                                  | 调起系统文件选择，取消时得到 `null`，页面里不留元素。上传头像等入口用 `IconButton` 加它                                                                                                                                                                                                                                                            |
| `Toast`                              | `ToastProvider`；`useToast()(opts: { leading?: ReactNode; message: string; action?: { label; onAction } })`                                                                                        | 3.6 秒后自动消失；有 action 时用于撤销或打开                                                                                                                                                                                                                                                                                                       |
| `Dialog`                             | `open; onClose; title; size?: 'sm' \| 'md'; children; footer: ReactNode`                                                                                                                           | 基于原生 `<dialog>`，宽度 400 或 460。`ConfirmDialog` 的宽度是 320，由它自己处理                                                                                                                                                                                                                                                                   |
| `ConfirmDialog`                      | `open; onClose; leading?: ReactNode; title; body; confirmLabel; cancelLabel; onConfirm; tone?: 'danger'`                                                                                           | 只用于不可撤销的操作                                                                                                                                                                                                                                                                                                                               |
| `Command`                            | `text; copyLabel; copiedLabel`                                                                                                                                                                     | 等宽命令，带一键复制；复制后在原位反馈。只在空格处折行，参数不会被拆开（超过一行的单个参数才在内部折行）；复制的始终是原来的单行 `text`                                                                                                                                                                                                            |
| `Dashes`                             | `total; current`                                                                                                                                                                                   | 引导页顶部的进度细线                                                                                                                                                                                                                                                                                                                               |
| `StepHeading`                        | `children; sub?: ReactNode`                                                                                                                                                                        | 引导每一步的标题（衬线 32px）和下面的一行小字，渲染成 `h1` 加 `p`，直接放在步骤的网格里                                                                                                                                                                                                                                                            |
| `Skeleton`                           | `shape: 'line' \| 'circle'; width; height?`                                                                                                                                                        | 加载用骨架，不用转圈                                                                                                                                                                                                                                                                                                                               |
| `Spinner`                            | `size?: 'sm' \| 'md'`                                                                                                                                                                              | 只用于行内的“正在进行”                                                                                                                                                                                                                                                                                                                             |
| `EmptyState`                         | `title; action?: ReactNode`                                                                                                                                                                        | 带探头的狐獴                                                                                                                                                                                                                                                                                                                                       |
| `Callout`                            | `tone: 'quiet' \| 'attention'; leading?: ReactNode; children; action?: ReactNode`                                                                                                                  | 成员暂停或需要处理时的提示条                                                                                                                                                                                                                                                                                                                       |
| `TrayField`                          | `label; children`                                                                                                                                                                                  | 直接铺在纸托盘上的表单项：小号加粗标签在上，通栏输入在下，输入框为白底。成员「高级」抽屉使用                                                                                                                                                                                                                                                       |
| `Message`                            | `from: 'me' \| 'them'; variant?: 'bubble' \| 'done'; avatar?: string; typing?; delay?: number; animate?: boolean; children`                                                                        | 对话气泡，引导页使用，以后的 IM 也复用。`done` 是带绿色勾的完成小卡片。`animate` 默认为 true；为 false 时不播放入场动画，用于重绘时已经出现过的消息                                                                                                                                                                                                |
| `QrCode`                             | `value: string; size: number`                                                                                                                                                                      | 用 `qrcode` 库生成 SVG                                                                                                                                                                                                                                                                                                                             |

`IconName` 的取值来自原型 `ICONS` 中的全部键：users chat monitor settings sidebar search updown
chev chevl smile refresh plus x check more pause play trash pencil out copy shuffle upload up ban laptop box msg zap，
以及实现过程中需要、并已登记到本文的新图标。

## scene/ 组件

scene 是 Lankide 的视觉语言：洞洞板、挂钩、工卡、吊牌、写字板、操控台、工位。
它们只负责“长什么样、怎么动”，不认识 Agent、Workspace 等业务概念。

| 组件            | Props                                                                                                                                                                                                                                                                                          | 说明                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| --------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Board`         | `layout: 'rails' \| 'free' \| 'open'; holes?: 'on' \| 'fade'; lead?: ReactNode; children`                                                                                                                                                                                                      | 洞洞板画布。孔网格自动对齐到第一个挂钉；`free` 模式下由子项自行定位，孔坐标从 `lead` 之下算起；`open` 不排布子项，用于成员详情。板面铺在透明顶栏之下（`AppShell` 的 `board`），`rails`、`free` 顶部留出顶栏和 4px，`free` 左右各 32px                                                                                                                                                                                                                                                                                                       |
| `Hang`          | `ref?: Ref<HTMLDivElement>; style?: { left?: number; top?: number }; lanyard?: 'member' \| 'lead'; motion?: 'enter' \| 'leave' \| 'turn-in'; swingOnHover?; swingKey?: number; faded?; muted?; onClick?; children`                                                                                     | 挂钉、挂绳、夹子，以挂钉为支点。传 `onClick` 时整张是可打开的卡片：`role="button"`、可聚焦，Enter 和 Space 也能打开。`faded` 让它退到背景（透明度 0.35），用于阅读性格时的 stage；`Badge` 的 `dimmed` 是暂停状态，两者不同。`muted` 让挂绳和夹子褪成灰色，用于授权失效的工卡，配合 `onClick` 时悬停会微微上抬。`swingKey` 的值每变化一次就晃一下，用于保存、换头像、落位之后。`turn-in` 表示原地翻入，例如二维码吊牌翻成工卡。FLIP 期间的层级由 `useFlip` 通过 data 属性处理，不放进 props。`ref` 和 `style` 用来直接接 `useHoleDrag` 和 `useSortable` 返回的 handle，不需要再包一层 div；`style` 只允许挂钉位置 `left`、`top` |
| `Badge`         | `ref?: Ref<HTMLElement>; style?: { left?: number; top?: number }; avatar: { src?; fallback?: string; status?: Status; statusTip?; creating?; onEdit?; editLabel? }; name: ReactNode; role?: ReactNode; bio?: ReactNode; foot?: ReactNode; size: 'card' \| 'stage' \| 'intro'; dimmed?; ghost?` | 白色工卡。头像由 Badge 内部渲染，并按工卡比例确定尺寸；`avatar.fallback` 与 `Avatar` 的失败回退一致，`src` 加载失败时改用它。`name`、`role`、`bio` 可以传纯文字，也可以传 `InlineText`。`intro` 是引导页的尺寸。`ghost` 是骨架状态的空白卡。`ref` 和 `style` 的用途同 `Hang`，用于不挂在挂绳上直接拖动的工卡                                                                                                                                                                                                                                |
| `VacantSlot`    | `ref?: Ref<HTMLButtonElement>; style?: { left?: number; top?: number }; label; onClick; shape: 'badge' \| 'clipboard'`                                                                                                                                                                         | 虚线空位，例如添加成员、任务清单停用后的位置。两种形状都带挂钉：`badge` 和 rails 里的工卡对齐，`clipboard` 和 `Clipboard` 对齐。`ref`、`style` 的用途同 `Hang`                                                                                                                                                                                                                                                                                                                                                                              |
| `Tag`           | `ref?: Ref<HTMLElement>; style?: { left?: number; top?: number }; children; tone?: 'paper'`                                                                                                                                                                                                    | 挂在绳上的吊牌（带孔眼），规约、二维码、Token 都用它。`ref`、`style` 的用途同 `Hang`                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `QrTag`         | `state: 'loading' \| 'wait' \| 'scanned' \| 'expired' \| 'error'; errorLabel?: string; value: string; brand: 'feishu' \| 'telegram'; scannerAvatar?: string; onRefresh; refreshLabel; onClose?; closeLabel?`                                                                                                                                          | 二维码吊牌：扫码后变暗，并显示扫码人的头像；过期时刷新按钮就在码上；`loading` 时显示骨架；`error` 时用本地化 `errorLabel` 和重新开始按钮替换二维码。登录成功后翻成工卡，由 `Hang motion="turn-in"` 完成。传 `onClose` 时孔眼旁出现关闭按钮                                                                                                                                                                                                                                                                                                                                                                                               |
| `TokenTag`      | `value; onChange; label; invalid?`                                                                                                                                                                                                                                                             | Telegram 的 Token 吊牌，填写线样式的输入。`invalid` 时填写线变红，不加文字                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `PaperLabel`    | `ref?: Ref<HTMLDivElement>; style?: { left?: number; top?: number }; value; onCommit; label`                                                                                                                                                                                                   | 团队名纸条，可以原地编辑。团队页把它放进 `Board` 的 `lead` 槽：它在文档流里居中，不再靠 `ref`、`style` 定位，也不被 `useHoleDrag` 接管                                                                                                                                                                                                                                                                                                                                                                                                      |
| `ShiftSign`     | `ref?: Ref<HTMLDivElement>; style?: { left?: number; top?: number }; on; onToggle; onLabel; offLabel; label`                                                                                                                                                                                   | 上班/下班翻牌。`ref`、`style` 的用途同 `Hang`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `Clipboard`     | `ref?: Ref<HTMLElement>; style?: { left?: number; top?: number }; title: ReactNode; actions: ReactNode; children; footer: ReactNode`                                                                                                                                                           | 木质写字板和纸，用于任务清单。`ref`、`style` 的用途同 `Hang`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| `ChecklistRow`  | `done; text; avatar?: string; onToggle?` 以及 `ChecklistRow.New: placeholder; onAdd(text)`                                                                                                                                                                                                      | 清单里的一行。不传 `onToggle` 时只读；可编辑时点击后立即显示，`done` 下一次变化后以 `done` 为准                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `Bench`         | `children`                                                                                                                                                                                                                                                                                     | 成员详情的台面：操控台在左、工卡挂在右边；窗口放不下时工卡换到操控台下方，整体居中                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `Console`       | `figure?: ReactNode; sticker?: ReactNode; notice?: ReactNode; screen: ReactNode; body?: ReactNode; deck: ReactNode; drawer?: { label; content: ReactNode }; reading?`                                                                                                                          | 暖白色操控台。`reading` 表示性格卡被抽出编辑                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| `ConsoleScreen` | `onClick?; variant: 'readout' \| 'list'; children`                                                                                                                                                                                                                                             | 黑色屏幕。`readout` 用于成员；`list` 用于引导页的 AI 列表                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `ScreenReadout` | `ai: { mark: ReactNode; label }; model: string; place: { label; status: Status }; level: { index; total; label }`                                                                                                                                                                              | 屏幕内容：AI、模型、工区和思考强度刻度                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `ConsoleKey`    | `icon?: IconName; label; lit?; go?; disabled?; onClick; children?`                                                                                                                                                                                                                             | 实体键。`go` 是带 LED 的主键（添加、保存）。没有贴纸时，deck 上的笑脸键也是 ConsoleKey                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `RigFigure`     | `id: string; pack?: FigurePack; seed?: string; onClick?; label?`                                                                                                                                                                                                                               | 操控台顶上的形象。可以点击，悬停时上浮，更换时有 pop 动画；不传 `onClick` 时是静态的（引导页）。自身不定位，位置由所在的槽位决定（`Console` 的 `figure`）                                                                                                                                                                                                                                                                                                                                                                                   |
| `Sticker`       | `reaction: string \| null; onClick; label`                                                                                                                                                                                                                                                     | 操控台上的表情贴纸，更换时有 pop 动画                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `HarnessRow`    | `mark: ReactNode; label; test?: 'run' \| 'ok' \| 'fail'; reply?: string; selected?; onSelect?; action?: { label; onClick }`                                                                                                                                                                    | `ConsoleScreen variant="list"` 中的一行：品牌图标、名字、右侧测试灯。选中后在行下方展开 `reply`。`action` 用于没有安装时的「安装」                                                                                                                                                                                                                                                                                                                                                                                                          |
| `SoulSlot`      | `value; onChange; label; placeholder; onFocusChange(reading)`                                                                                                                                                                                                                                  | 插槽里的性格卡                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| `Lever`         | `levels: string[]; value: number; onChange`                                                                                                                                                                                                                                                    | 竖向档位拉杆，档位数量由模型决定                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `Figure`        | `id: string; pose: 'work' \| 'sleep' \| 'carry' \| 'stand'; scale?: number; pack?: { frames: Partial<Record<Pose, string>>; scale: number }; seed?: string`                                                                                                                                    | 形象帧。内置形象只传 `id`，尺寸查 `figures.ts`；导入的形象包传 `pack`（每个姿势一个帧 URL），尺寸是图片原始像素乘以 `pack.scale` 和 `scale`，加载完成前不显示。`id` 是 `default`、不是内置形象，或形象包缺这个姿势（例如没有坐姿）时，按 `seed`（传成员 id）稳定地选一个内置形象顶替，features 不用自己挑                                                                                                                                                                                                                                   |
| `Seat`          | `setup: 'laptop' \| 'mini'; screen: 'dark' \| 'asleep' \| 'alert' \| 'busy' \| 'ok'; occupant: { figure: string; pack?: FigurePack; seed?: string } \| 'empty-chair'; arriving?; size?: 'seat' \| 'hero'; label; onClick?; carry?: CarryHandle`                                                | 一张工位：桌子、屏幕、形象或空椅子，睡着时显示 zz。形象的姿势由 `screen` 推出（`asleep` 和 `dark` 对应 sleep，其他对应 work），不另设 `pose` 参数                                                                                                                                                                                                                                                                                                                                                                                           |
| `WorkRow`       | `title: ReactNode; status: Status; menu?: ReactNode; arriving?; children` 加上 `useCarry().rowProps(rowKey)`                                                                                                                                                                                   | 一个工区一排，标题是名字加状态点。`arriving` 表示新工区加入时的高亮。拖拽悬停的样式只认 `rowProps` 设置的 `data-over`                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `AddRow`        | `label; onClick`                                                                                                                                                                                                                                                                               | 最后一行的添加入口                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `Reaction`      | `id: string; size?: number`                                                                                                                                                                                                                                                                    | 表情图片                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `Mascot`        | `pose: 'peek'`                                                                                                                                                                                                                                                                                 | 狐獴插画                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `PullCord`      | `onToggle; label`                                                                                                                                                                                                                                                                              | 右上角的拉线开关灯。无障碍语义是普通按钮，`label` 是按钮名称                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |

拖拽相关的 hook 都在 `scene/drag/`，从 scene 的 index 导出：

| Hook          | 签名                                                                                                                        | 用途                                                                                                                                                                                                                                                                       |
| ------------- | --------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `useHoleDrag` | `(items: { key; kind; pos }[], onMove(key, pos))` 返回每个子项的 `{ ref, style, dragging }`；同文件导出孔距常量 `holePitch` | 团队墙上吊牌、清单、翻牌的自由挂放。`ref`、`style` 直接给 `Hang`、`Badge`、`Tag`、`Clipboard`、`ShiftSign`、`VacantSlot`；拖动中的元素由 hook 标上 `data-dragging`，不需要外包一层。物件内部的按钮、输入框不触发拖动，物件本身是按钮（`VacantSlot`）时可以拖，短按仍是点击 |
| `useSortable` | `(ids: string[], onReorder(ids))` 返回 `{ itemProps(id) }`                                                                  | 成员板上的工卡排序                                                                                                                                                                                                                                                         |
| `useCarry`    | `(onDrop(id, rowKey))` 返回 `{ seatHandle(id), rowProps(rowKey) }`                                                          | 拎起成员，换到另一个工区                                                                                                                                                                                                                                                   |
| `useFlip`     | `() => (from: DOMRect, el: HTMLElement) => void`                                                                            | 工卡从板上飞到详情页，或从详情页飞回板上                                                                                                                                                                                                                                   |

## shell/

| 组件           | Props                                                                     | 说明                                                                                                                                                                              |
| -------------- | ------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `AppShell`     | `sidebar; topBar; children; board?: 'wall' \| 'open'`                     | 窗口布局：在外壳内时为红绿灯留出安全区，并设置拖拽区；toast 容器；拉线。侧栏开关按钮也由 AppShell 内部渲染（展开时在红绿灯右侧，图标栏模式时移到栏内），状态读 `useSidebarLayout` |
| `Sidebar`      | `header: ReactNode; nav: ReactNode; footer: ReactNode`                    | 可以拖动边缘调宽（160–480）、折叠成图标栏（宽 84，token `--rail`）、完全隐藏；图标栏模式下隐藏 `header`；状态来自 `useSidebarLayout`                                              |
| `SidebarGrip`  | —                                                                         | 拖拽调宽；双击切换图标模式                                                                                                                                                        |
| `NavItem`      | `icon: IconName; label; active; onClick; testId`                          | 侧栏专用的行按钮（原生 button 加自己的样式，不借用 `Button`）。当前页是浅色填充。只有图标模式下才包 `Tooltip`，名字显示在图标右侧                                                 |
| `TeamSwitcher` | `teams: { id; name }[]; current; onSelect; onCreate; createLabel`         | 侧栏顶部的团队按钮，读作侧栏标题。点开是 `Menu`：各团队（当前团队带勾）、分隔线、「新建团队」（`createLabel`）。选中当前团队时不触发 `onSelect`                                   |
| `TopBar`       | `title?: string; back?: { label; onClick } \| { label; menu: ReactNode }` | 传 `menu` 时，点返回会在返回键下方弹出菜单（例如有未保存修改时的「保存 / 放弃更改」），返回键本身就是 `Menu` 的触发器，带下拉 chevron；普通返回在文字前显示左箭头                 |

## features/

每个 feature 只导出页面组件。下表列出它包含的组件、依赖的数据 hook，以及 E2E 使用的 testid。

### members（成员）

| 文件                                                         | 内容                                                                                                          |
| ------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------- |
| `MembersPage.tsx`                                            | 成员板：用 `Board` 的 rails 布局，加 `useSortable`。所有人类工卡排在 Agent 工卡前，最后是添加用的空位                  |
| `MemberCard.tsx`                                             | 板上的一张工卡：`Hang` 加 `Badge`，状态点和工卡底部的模型显示                                                 |
| `HumanCard.tsx` | 人类成员的小工卡与详情工卡。统一读取人类名册，姓名头像只读，岗位输入支持 Enter/失焦保存和 Esc 撤销，无卡背、介绍、状态点或模型脚注。 |
| `MemberBench.tsx`                                            | 打开成员后的页面：左边是 `Console`，右边挂着大工卡，直接在卡上编辑                                            |
| `MemberConsole.tsx`                                          | 组合操控台：形象、表情贴纸、屏幕、性格卡、按键、高级抽屉                                                      |
| `BrainPicker.tsx`                                            | 屏幕点开后的四列选择器：工区、AI、模型、思考。由 `Popover`、`SearchField`、`OptionColumn` 和 `Lever` 组合而成 |
| `AdvancedDrawer.tsx`                                         | 环境变量、启动参数（输入框保留原始文本，提交时再拆分）、命令行路径、测试                                      |
| `AvatarPicker.tsx`、`ReactionPicker.tsx`、`FigurePicker.tsx` | 都基于 `GridPicker` 加 `Popover`；文件选择用 design 的 `pickFile`                                             |
| `DeleteMemberDialog.tsx`                                     | 基于 `ConfirmDialog`                                                                                          |
| `MemberUnread.tsx`                                           | 成员读出来之前顶替工作台：加载中的占位工卡、读取失败（带重试）、成员不存在                                    |
| `useMemberDraft.ts`                                          | 草稿、脏状态、保存、创建，以及创建时的进度                                                                    |
| `draft.ts`                                                   | 草稿类型、空草稿、`emptyUpdate`、`broken`（读取失败的判断）、`oneAtATime`（同一成员的写入逐个进行）           |
| `args.ts`                                                    | 启动参数文本和参数列表互转（支持引号和反斜杠）                                                                |
| `avatars.ts`                                                 | 内置头像列表，按成员 id 选默认头像，转成上传格式                                                              |
| `creation.ts`                                                | 飞书团队创建成员时的进度状态：创建中、等待扫码确认、失败、就绪                                                |
| `messages.ts`                                                |                                                                                                               |

数据：`useAgents`、`useAgent`、`useAgentAvatar`、`useMachines`、`useHarnessCatalog`、`useModelCatalog`、
`useReactionCatalog`、`useFigurePacks`、`useImportFigurePack`、`useWorkspace`、
`useHumanMembers`、`useUpdateHumanPosition`、`useCreateAgent`、`useUpdateAgent`、`useDeleteAgent`、
`useResumeAgent`、`useStartAgentOnboarding`、`useAgentOnboardingProgress`（飞书团队创建成员）、
`lib/assets.ts` 的 `resourceUrl`、`packFrames`（形象包帧）和 `figureId`（`builtin:<id>` 转裸 id）。

testid：`member-card-{id}`、`human-card-{person_id}`、`human-position`、`add-member`、`member-name`、`member-save`、`member-create`、
`member-create-qr`（`data-state` 为 `wait` 或 `scanned`）、`member-pause`、`member-delete`、`member-test`、
`confirm-delete-member`、`brain-picker`、`reaction-picker`、`avatar-picker`、`figure-picker`。

### team（团队）

| 文件              | 内容                                                                             |
| ----------------- | -------------------------------------------------------------------------------- |
| `TeamPage.tsx`    | 用 `Board` 的 free 布局，加 `useHoleDrag`；摆放位置存在 localStorage，按团队区分 |
| `RulesTag.tsx`    | 用 `Tag` 加 `DocArea` 显示规约，失焦时自动保存                                   |
| `Tasklist.tsx`    | 用 `Clipboard` 加 `ChecklistRow`；停用时显示为 `VacantSlot`                      |
| `ShiftToggle.tsx` | 用 `ShiftSign` 切换上班/下班，对应 `set_workspace_state`                         |
| `messages.ts`     |                                                                                  |

拖动的 handle（`ref`、`style`）直接传给 `Tag`、`Clipboard`、`VacantSlot`、`ShiftSign`，没有外包元素。
`PaperLabel` 放在 `Board` 的 `lead` 槽，位于文档流中，随窗口和侧栏宽度居中，不需要外包 `left: 50%` 的元素。`items` 里的孔坐标从 `lead` 之下算起；列数由 `items` 内一个单独的不可见元素测量其父容器宽度得到，不借用名牌的 `ref`。

数据：`useWorkspace`、`useUpdateWorkspace`、`useSetWorkspaceState`、`useWorkspaceTasks`、
`useCreateWorkspaceTask`、`useSetWorkspaceTaskCompleted`、`useWorkspaceImage`。

testid：`team-name`、`team-rules`、`team-shift`、`tasklist`、`task-{id}`。

### office（工区）

| 文件                      | 内容                                                                                                        |
| ------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `OfficePage.tsx`          | 每台 Machine 一排 `WorkRow`，加 `useCarry`；最后是 `AddRow`                                                 |
| `MemberSeat.tsx`          | 把成员状态映射成 `Seat` 的 screen；形象按成员的 figure 取（形象包取帧 URL，`default` 按成员 id 选内置形象） |
| `MachineMenu.tsx`         | 重命名（标题原地变成 `InlineText`，菜单项 `returnFocus={false}`）和移除                                     |
| `AddMachineDialog.tsx`    | `Command`、等待连接的状态行，以及可展开的网络地址；导出 `usePairing`                                        |
| `RemoveMachineDialog.tsx` |                                                                                                             |
| `LoadState.tsx`           | 工区读出来之前的占位，读取失败时显示重试                                                                    |
| `messages.ts`             |                                                                                                             |

数据：`useMachines`、`useAgents`、`useWorkspace`、`useFigurePacks`、`useUpdateAgent`、`useRenameMachine`、
`useRevokeMachine`、`usePairingCode`、`useEnableRemoteAccess`、`useSetRemoteAccess`。

testid：`machine-row-{id}`、`machine-menu-{id}`、`add-machine`、`pair-command`、`pair-state`、
`remote-address`、`confirm-remove-machine`。

### settings（设置）

| 文件               | 内容                                                   |
| ------------------ | ------------------------------------------------------ |
| `SettingsPage.tsx` | 一个 `Group`：语言用 `Select`，外观用三张 `ThemeThumb` |
| `messages.ts`      |                                                        |

数据：`useLanguage`、`useTheme`。

testid：`settings-language`、`settings-theme-{system|light|dark}`。

### onboarding（引导）

| 文件                                                                                             | 内容                                                                        |
| ------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------- |
| `OnboardingFlow.tsx`                                                                             | 舞台、`Dashes`、固定底栏（左侧“稍后设置”，右侧主按钮）、步骤切换            |
| `useOnboarding.ts`                                                                               | 步骤状态机和跳过规则                                                        |
| `WelcomeStep.tsx`、`ConnectStep.tsx`、`ChooseAiStep.tsx`、`FirstMemberStep.tsx`、`ReadyStep.tsx` | 每个步骤一个文件，只组合 scene 组件和 design 组件                           |
| `ChatPreview.tsx`                                                                                | 用 `Message` 组成一段飞书对话                                               |
| `useAiPick.ts`                                                                                   | 进入选 AI 这一步后，测试最近选的（或第一个可用的）AI，并给出每行的测试结果  |
| `messages.ts`                                                                                    |                                                                             |

数据：`useCreateWorkspace`、`useFeishuConsoleLogin`、`useStartFeishuConsoleLogin`、`useMachines`、
`useHarnessTrials`、`useCreateAgent`、`useStartAgentOnboarding`、`useAgentOnboardingProgress`、
`useAgentFirstReply`、`useAgent`、`useHumanMembers`。

testid：`onboarding`（`data-step` 为当前步骤 0–4）、`onboarding-next`、`onboarding-later`、
`onboarding-provider`、`onboarding-qr`（`data-state` 为 `loading`、`wait`、`scanned`、`expired`；扫码完成后二维码移除）、
`onboarding-token`、`harness-{id}`（`data-test` 为 `run`、`ok`、`fail`）、
`onboarding-member-name`、`onboarding-confirm`、`onboarding-chat`、`onboarding-open`、`onboarding-retry`。

### 外壳（app）的 testid

`nav-{members|team|office}` 和 `nav-settings`（侧栏入口）、`team-switcher`（团队按钮）。

### 跨 feature 的共用

成员状态到 `Status` 的映射，以及到工位屏幕状态的映射，members 和 office 都要用。
这段逻辑放在 `lib/queries/agents.ts` 的 `agentStatus(agent, machine, workspace)` 中，两个 feature 不直接互相引用。

## 原型对照

| 原型                                              | 拆成                                                                                                                               |
| ------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| `renderSide`、`nav`、侧栏拖拽 IIFE                | `Sidebar`、`SidebarGrip`、`NavItem`、`TeamSwitcher`、`useSidebarLayout`                                                            |
| `renderBar`                                       | `TopBar`                                                                                                                           |
| `badge()`                                         | `Hang`、`Badge`、`Avatar`、`StatusDot`，以及 feature 里的 `MemberCard` 和 `HumanCard`                                               |
| `pageMembers`                                     | `MembersPage`、`VacantSlot`、`useSortable`                                                                                         |
| `pageMember`                                      | `MemberBench`、`MemberConsole`、`Console`、`ConsoleScreen`、`ScreenReadout`、`ConsoleKey`、`SoulSlot`、`Callout`、`AdvancedDrawer` |
| `brainMenu`、`leverHTML`、`bindLever`             | `BrainPicker`、`Popover`、`SearchField`、`OptionColumn`、`Lever`                                                                   |
| `reactionMenu`、`avatarMenu`、`figureMenu`        | `GridPicker`，以及 `ReactionPicker`、`AvatarPicker`、`FigurePicker`                                                                |
| `comboMenu`、`commitModel`                        | 已废弃；自定义模型用 `BrainPicker` 中 `SearchField` 的 Enter                                                                       |
| `pageTeam`、`checklist`、`taskRow`                | `TeamPage`、`PaperLabel`、`Tag`、`RulesTag`、`Clipboard`、`ChecklistRow`、`ShiftSign`、`useHoleDrag`                               |
| `pageMachines`、`seat`、`screenOf`                | `OfficePage`、`WorkRow`、`Seat`、`Figure`、`MemberSeat`、`useCarry`                                                                |
| `pageSettings`                                    | `SettingsPage`、`Group`、`Select`、`ThemeThumb`                                                                                    |
| `openMenu`、`mi`                                  | `Menu`、`Menu.Item`                                                                                                                |
| `openDialog`、`confirmDelete`、`addMachineDialog` | `Dialog`、`ConfirmDialog`、`DeleteMemberDialog`、`AddMachineDialog`、`Command`                                                     |
| `toast`                                           | `ToastProvider`、`useToast`                                                                                                        |
| `flyFrom`、`enterMember`、`leaveMember`           | `useFlip`                                                                                                                          |
| 拉线 IIFE                                         | `PullCord` 和 `lib/theme.ts`                                                                                                       |
| `qrCode`、`qrTag`                                 | `QrCode`、`QrTag`                                                                                                                  |
| `obWelcome` … `obReady`、`obFoot`、`obRender`     | `OnboardingFlow` 和各个 `*Step`                                                                                                    |
| `msg`、`obChat`                                   | `Message`、`ChatPreview`                                                                                                           |
| `obDesk`                                          | `Seat`                                                                                                                             |

## dev 预览页

`#/design` 按 `design/`、`scene/`、`shell/` 分组，每个组件列出全部 variant 和状态，浅色和深色并排显示。
视觉验收会拿它和 `system.html` 对照。新增组件时，必须在 `dev/sections/` 中同时补上它的预览。
生产构建通过 `import.meta.env.DEV` 去掉整个 `dev/` 目录。

## 墙板顶栏的层级

墙板（`AppShell` 的 `board="wall"|"open"`）上的顶栏是透明的，叠在板面之上。顶栏本身不建立层叠上下文，拖窗层、板面内容、顶栏控件在同一个上下文里按 token 比较，从下到上：

| token      | 值  | 用途                                                                                   |
| ---------- | --- | -------------------------------------------------------------------------------------- |
| `--z-drag` | 1   | 拖窗层：顶栏的 `::before`，Desktop 下命中它即命中带 `data-tauri-drag-region` 的 header |
| `--z-live` | 4   | 伸进顶栏高度内、仍需点击的板面内容，例如 `Console` 的 notice                           |
| `--z-bar`  | 30  | 顶栏内的按钮（返回键、菜单触发器），高于板面内容                                       |

新增会伸进顶栏高度的可交互板面内容时用 `--z-live`，不要自己加数字。弹层仍在 `--z-popup` 以上。

人类成员：`members/HumanCard.tsx` 导出 HumanCard 和 HumanBench，统一使用人类名册接口。小卡与大卡都用品牌挂绳，姓名头像只读，只编辑岗位；没有卡背、介绍、状态或模型脚注。`human-card-<person_id>`、`human-position` 为测试入口。
