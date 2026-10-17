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

| 层 | 可以 import |
|---|---|
| `design/` | `design/` 内部、`react`、`@base-ui/react`、`qrcode`、`lucide-react` |
| `scene/` | `design/`、`scene/` 内部 |
| `shell/` | `design/`、`scene/`、`lib/` |
| `features/<x>/` | `design/`、`scene/`、`lib/`、自己目录内的文件 |
| `app/` | 全部 |
| `lib/` | `lib/` 内部、`api/generated` |
| `dev/` | `design/`、`scene/`、`shell/` |

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
      Switch.tsx  Segmented.tsx  Select.tsx  Combo.tsx  ChoiceGroup.tsx
      Group.tsx  Reveal.tsx
      StatusDot.tsx  Avatar.tsx  AvatarStack.tsx
      Menu.tsx  Popover.tsx  GridPicker.tsx  Tooltip.tsx
      Toast.tsx  Dialog.tsx  ConfirmDialog.tsx
      Command.tsx  Dashes.tsx  Skeleton.tsx  Spinner.tsx
      EmptyState.tsx  Callout.tsx  Message.tsx  QrCode.tsx
      *.module.css         一个组件对应一个样式文件，同名
      index.ts             唯一出口
    scene/
      Board.tsx  Hang.tsx  Badge.tsx  BadgeVerso.tsx  FlipCard.tsx  VacantSlot.tsx
      Tag.tsx  QrTag.tsx  TokenTag.tsx  PaperLabel.tsx  ShiftSign.tsx
      Clipboard.tsx  ChecklistRow.tsx
      Console.tsx  ConsoleScreen.tsx  ScreenReadout.tsx  ConsoleKey.tsx  SoulSlot.tsx  Lever.tsx
      Figure.tsx  Seat.tsx  WorkRow.tsx  AddRow.tsx
      Reaction.tsx  Mascot.tsx  PullCord.tsx
      figures.ts           形象帧的尺寸表
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

- `apps/desktop/assets` 下的 reactions、icons、mascot、fonts
- `docs/design` 下的 avatars、figures、office

`system.html` 改为引用这里的素材这件事，由设计会话负责。

## 写法约束（lint 强制）

| 约束 | 工具 |
|---|---|
| 禁止使用 `dangerouslySetInnerHTML` | ESLint `react/no-danger` |
| `features/` 和 `shell/` 中禁止直接写 `button`、`input`、`textarea`、`select`、`dialog`、`img`、`svg` 元素 | ESLint `no-restricted-syntax`（JSXOpeningElement） |
| 层之间的 import 方向必须符合上一节的表 | ESLint `import-x/no-restricted-paths` |
| 单个文件不超过 200 行，JSX 嵌套不超过 6 层 | ESLint `max-lines`、`react/jsx-max-depth` |
| `features/` 和 `shell/` 中的 JSX 文本不能是字面量，必须用 `t()` | ESLint `i18next/no-literal-string`（只检查 JSX 文本） |
| 颜色、圆角、阴影、字号只能写 `var(--…)`；只有 `tokens.css` 可以写字面值 | stylelint `scale-unlimited/declaration-strict-value` |
| `design/` 和 `scene/` 组件的 props 不接受 `className` 和 `style` | 用 TS 类型强制：props 基于 `Omit<…, 'className' \| 'style'>` |
| 原生属性透传只允许 `data-testid`、`aria-*`、事件回调 | 由各组件的 props 类型声明 |

其他约定：

- 需要新外观时，给组件加 variant，不在调用处覆盖样式。
- `features/` 的 CSS Module 只写布局：`display`、`gap`、`grid`、`margin`、`padding`、`width`。
- 定时器、轮询和 WS 只出现在 `lib/` 的 hook 和 `scene/drag/` 中。features 不直接写 `useEffect`
  订阅外部资源；如果必须写，在注释里说明理由。
- 动效只用 CSS transition、keyframes 和 `@starting-style`。少数 FLIP 场景用 `useFlip` 里的 Web Animations API。

## design/ 组件

除非特别说明，所有组件都透传 `data-testid` 和 `aria-*`。`Status` 统一为 `'ok' | 'busy' | 'idle' | 'err'`。

| 组件 | Props | 说明 |
|---|---|---|
| `Icon` | `name: IconName; size?: 12 \| 14 \| 16 \| 18 \| 20 \| 22` | 线条图标，颜色跟随 `currentColor` |
| `BrandMark` | `brand: 'feishu' \| 'telegram' \| 'codex' \| 'claude-code' \| 'pi'; size?: number` | 品牌图标。单色图标跟随墨色 |
| `Button` | `variant?: 'default' \| 'primary' \| 'quiet' \| 'danger'; size?: 'sm' \| 'md' \| 'lg'; loading?; leading?: ReactNode; children` 加上原生 button 事件 | 胶囊形。一屏最多一个 primary |
| `IconButton` | `icon: IconName; label: string; size?: 'sm' \| 'md'; onClick` | `label` 同时用作 tooltip 和 aria-label |
| `LinkButton` | `children; disclosure?: boolean; open?: boolean; onClick` | 文字链接；带 disclosure 时显示 chevron，展开后旋转 |
| `TextInput` | `value; onChange; size?: 'md' \| 'lg'; align?: 'start' \| 'center'; mono?; invalid?; placeholder` | 无边框，聚焦时显示品牌紫光圈 |
| `DocArea` | `value; onChange; placeholder; minRows?` | 文档式多行编辑，高度随内容增长 |
| `InlineText` | `value; onCommit(v); placeholder; multiline?; textStyle: 'display' \| 'heading' \| 'title' \| 'body' \| 'small'; autoFocus?` | 平时就是一段文字，点击后原地编辑。Enter 或失焦时提交，Esc 撤销 |
| `Switch` | `checked; onChange; label` | 只用在设置行 |
| `Segmented<T>` | `options: { value: T; label: string; icon?: ReactNode }[]; value; onChange; size?: 'sm' \| 'md'` | 固定的 2 到 4 项 |
| `Select<T>` | `options: { value: T; label: string; leading?: ReactNode; disabled? }[]; value; onChange; variant?: 'quiet' \| 'boxed'` | 基于 Base UI Select；quiet 平时就是文字 |
| `Combo` | `value; suggestions: string[]; onCommit(v); mono?; placeholder` | 输入即过滤，Enter 采用当前输入 |
| `ChoiceGroup<T>` | `options: { value: T; label: string; icon: ReactNode }[]; value; onChange` | 需要靠品牌图标认出来的一次性选择 |
| `Group` / `Group.Row` | `Group: children`；`Row: label; children; align?: 'center' \| 'top'` | 唯一的容器。行高 40，行与行之间是细线 |
| `Reveal` | `open; children` | 带高度动画的折叠区域 |
| `StatusDot` | `status: Status; tip?: string` | 表达状态的唯一方式；busy 状态呼吸闪烁 |
| `Avatar` | `src?; size: 20 \| 26 \| 32 \| 40 \| 44 \| 60 \| 76 \| 88; status?: Status; dimmed?; creating?; selected?; onEdit?: () => void; editLabel?` | 状态点挂在右下角。`dimmed` 表示暂停（变灰）；`onEdit` 存在时悬停显示铅笔 |
| `AvatarStack` | `srcs: string[]; size: 20 \| 26` | 头像叠放 |
| `Menu` | `trigger: ReactElement; align?: 'start' \| 'end'; children` | 基于 Base UI Menu |
| `Menu.Item` | `leading?: ReactNode; checked?; danger?; onSelect; children` | 菜单项高 30，选中时显示勾 |
| `Menu.Separator` | — | |
| `Popover` | `trigger: ReactElement; align?; open?; onOpenChange?; children` | 基于 Base UI Popover，供网格选择器和多列选择器使用 |
| `GridPicker<T>` | `cells: { value: T; label?: string; content: ReactNode }[]; value?: T; onPick(v); leading?: ReactNode; columns: number` | 头像、表情、形象选择器共用。`leading` 放上传、导入或“不回复”格 |
| `Tooltip` | `label; children` | 对应原型里所有 `data-tip` |
| `Toast` | `ToastProvider`；`useToast()(opts: { leading?: ReactNode; message: string; action?: { label; onAction } })` | 3.6 秒后自动消失；有 action 时用于撤销或打开 |
| `Dialog` | `open; onClose; title; size?: 'sm' \| 'md'; children; footer: ReactNode` | 基于原生 `<dialog>`，宽度 400 或 560 |
| `ConfirmDialog` | `open; onClose; leading?: ReactNode; title; body; confirmLabel; cancelLabel; onConfirm; tone?: 'danger'` | 只用于不可撤销的操作 |
| `Command` | `text; copyLabel; copiedLabel` | 等宽命令，带一键复制；复制后在原位反馈 |
| `Dashes` | `total; current` | 引导页顶部的进度细线 |
| `Skeleton` | `shape: 'line' \| 'circle'; width; height?` | 加载用骨架，不用转圈 |
| `Spinner` | `size?: 'sm' \| 'md'` | 只用于行内的“正在进行” |
| `EmptyState` | `title; action?: ReactNode` | 带探头的狐獴 |
| `Callout` | `tone: 'quiet' \| 'attention'; leading?: ReactNode; children; action?: ReactNode` | 成员暂停或需要处理时的提示条 |
| `Message` | `from: 'me' \| 'them'; avatar?: string; typing?; delay?: number; children` | 对话气泡。引导页使用，以后的 IM 也复用 |
| `QrCode` | `value: string; size: number` | 用 `qrcode` 库生成 SVG |

`IconName` 的取值来自原型 `ICONS` 中的全部键：users chat monitor settings sidebar search updown
chev chevl smile refresh plus x check more pause play trash pencil out copy shuffle upload up ban laptop box msg zap，
以及实现过程中需要、并已登记到本文的新图标。

## scene/ 组件

scene 是 Lankide 的视觉语言：洞洞板、挂钩、工卡、吊牌、写字板、操控台、工位。
它们只负责“长什么样、怎么动”，不认识 Agent、Workspace 等业务概念。

| 组件 | Props | 说明 |
|---|---|---|
| `Board` | `layout: 'rails' \| 'free'; holes?: 'on' \| 'fade'; children` | 洞洞板画布。孔网格自动对齐到第一个挂钉；`free` 模式下由子项自行定位 |
| `Hang` | `lanyard?: 'member' \| 'lead'; motion?: 'enter' \| 'leave'; swingOnHover?; children` | 挂钉、挂绳、夹子，以挂钉为支点 |
| `Badge` | `avatar: ReactNode; name: ReactNode; role?: ReactNode; bio?: ReactNode; foot?: ReactNode; size: 'card' \| 'stage'; dimmed?; ghost?` | 白色工卡。`name`、`role`、`bio` 可以传纯文字，也可以传 `InlineText`；`ghost` 是骨架状态的空白卡 |
| `BadgeVerso` | `mark: ReactNode; actions: { leading?: ReactNode; label; onClick }[]` | 你自己工卡的背面：品牌标志加撕页按钮 |
| `FlipCard` | `flipped; front; back; onFlip` | 翻面容器，在原地翻转 |
| `VacantSlot` | `label; onClick; shape: 'badge' \| 'clipboard'` | 虚线空位，例如添加成员、任务清单停用后的位置 |
| `Tag` | `children; tone?: 'paper'` | 挂在绳上的吊牌（带孔眼），规约、二维码、Token 都用它 |
| `QrTag` | `state: 'loading' \| 'wait' \| 'scanned' \| 'expired'; value: string; brand: 'feishu' \| 'telegram'; scanner?: string; onRefresh; refreshLabel` | 二维码吊牌：扫码后变暗并显示扫码人；过期时刷新按钮就在码上 |
| `TokenTag` | `value; onChange; label` | Telegram 的 Token 吊牌，填写线样式的输入 |
| `PaperLabel` | `value; onCommit; label` | 贴在板子上方的团队名纸条，可以原地编辑 |
| `ShiftSign` | `on; onToggle; onLabel; offLabel; label` | 上班/下班翻牌 |
| `Clipboard` | `title: ReactNode; actions: ReactNode; children; footer: ReactNode` | 木质写字板和纸，用于任务清单 |
| `ChecklistRow` | `done; text; avatar?: string; onToggle` 以及 `ChecklistRow.New: placeholder; onAdd(text)` | 清单里的一行 |
| `Console` | `figure?: ReactNode; sticker?: ReactNode; notice?: ReactNode; screen: ReactNode; body?: ReactNode; deck: ReactNode; drawer?: { label; content: ReactNode }; reading?` | 暖白色操控台。`reading` 表示性格卡被抽出编辑 |
| `ConsoleScreen` | `onClick?; variant: 'readout' \| 'list'; children` | 黑色屏幕。`readout` 用于成员；`list` 用于引导页的 AI 列表 |
| `ScreenReadout` | `ai: { mark: ReactNode; label }; model: string; place: { label; status: Status }; level: { index; total; label }` | 屏幕内容：AI、模型、工区和思考强度刻度 |
| `ConsoleKey` | `icon?: IconName; label; lit?; go?; disabled?; onClick; children?` | 实体键。`go` 是带 LED 的主键（添加、保存） |
| `SoulSlot` | `value; onChange; label; placeholder; onFocusChange(reading)` | 插槽里的性格卡 |
| `Lever` | `levels: string[]; value: number; onChange` | 竖向档位拉杆，档位数量由模型决定 |
| `Figure` | `id: string; pose: 'work' \| 'sleep' \| 'carry' \| 'stand'; scale?: number` | 形象帧，尺寸查 `figures.ts` |
| `Seat` | `setup: 'laptop' \| 'mini'; screen: 'dark' \| 'asleep' \| 'alert' \| 'busy' \| 'ok'; occupant: { figure: string } \| 'empty-chair'; arriving?; label; onClick?; carry?: CarryHandle` | 一张工位：桌子、屏幕、形象或空椅子，睡着时显示 zz |
| `WorkRow` | `title: ReactNode; status: Status; menu?: ReactNode; dropActive?; children` | 一个工区一排，标题是名字加状态点 |
| `AddRow` | `label; onClick` | 最后一行的添加入口 |
| `Reaction` | `id: string; size?: number` | 表情图片 |
| `Mascot` | `pose: 'peek'` | 狐獴插画 |
| `PullCord` | `onToggle; label` | 右上角的拉线开关灯 |

拖拽相关的 hook 都在 `scene/drag/`，从 scene 的 index 导出：

| Hook | 签名 | 用途 |
|---|---|---|
| `useHoleDrag` | `(items: { key; kind; pos }[], onMove(key, pos))` 返回每个子项的 `{ ref, style, dragging }` | 团队墙上吊牌、清单、翻牌的自由挂放 |
| `useSortable` | `(ids: string[], onReorder(ids))` 返回 `{ itemProps(id) }` | 成员板上的工卡排序 |
| `useCarry` | `(onDrop(id, rowKey))` 返回 `{ seatHandle(id), rowProps(rowKey) }` | 拎起成员，换到另一个工区 |
| `useFlip` | `() => (from: DOMRect, el: HTMLElement) => void` | 工卡从板上飞到详情页，或从详情页飞回板上 |

## shell/

| 组件 | Props | 说明 |
|---|---|---|
| `AppShell` | `sidebar; topBar; children; board?: boolean` | 窗口布局：在外壳内时为红绿灯留出安全区，并设置拖拽区；toast 容器；拉线 |
| `Sidebar` | `header: ReactNode; nav: ReactNode; footer: ReactNode` | 可以拖动边缘调宽、折叠成图标栏、完全隐藏；状态来自 `useSidebarLayout` |
| `SidebarGrip` | — | 拖拽调宽；双击切换图标模式 |
| `NavItem` | `icon: IconName; label; active; onClick; testId` | 图标模式下悬停显示名字 |
| `TeamSwitcher` | `teams: { id; name; status: Status }[]; current; onSelect; onCreate; createLabel` | 侧栏顶部的团队切换 |
| `TopBar` | `title?: string; back?: { label; onClick }` | |

## features/

每个 feature 只导出页面组件。下表列出它包含的组件、依赖的数据 hook，以及 E2E 使用的 testid。

### members（成员）

| 文件 | 内容 |
|---|---|
| `MembersPage.tsx` | 成员板：用 `Board` 的 rails 布局，加 `useSortable`。你自己的工卡排第一位，最后是添加用的空位 |
| `MemberCard.tsx` | 板上的一张工卡：`Hang` 加 `Badge`，状态点和工卡底部的模型显示 |
| `LeadCard.tsx` | 你自己的工卡：使用 `FlipCard`，背面是 `BadgeVerso`，提供同步资料和切换账号 |
| `MemberBench.tsx` | 打开成员后的页面：左边是 `Console`，右边挂着大工卡，直接在卡上编辑 |
| `MemberConsole.tsx` | 组合操控台：形象、表情贴纸、屏幕、性格卡、按键、高级抽屉 |
| `BrainPicker.tsx` | 屏幕点开后的四列选择器：工区、AI、模型、思考，带搜索 |
| `AdvancedDrawer.tsx` | 环境变量、启动参数、命令行路径 |
| `AvatarPicker.tsx`、`ReactionPicker.tsx`、`FigurePicker.tsx` | 都基于 `GridPicker` 加 `Popover` |
| `DeleteMemberDialog.tsx` | 基于 `ConfirmDialog` |
| `useMemberDraft.ts` | 草稿、脏状态、保存，以及创建时的进度 |
| `messages.ts` | |

数据：`useAgents`、`useAgent`、`useMachines`、`useHarnessCatalog`、`useModelCatalog`、
`useReactionCatalog`、`useCreateAgent`、`useUpdateAgent`、`useDeleteAgent`、`useResumeAgent`。

testid：`member-card-{id}`、`add-member`、`member-name`、`member-save`、`member-create`、
`member-pause`、`member-delete`、`confirm-delete-member`、`brain-picker`、`reaction-picker`、`avatar-picker`。

### team（团队）

| 文件 | 内容 |
|---|---|
| `TeamPage.tsx` | 用 `Board` 的 free 布局，加 `useHoleDrag`；摆放位置存在 localStorage，按团队区分 |
| `RulesTag.tsx` | 用 `Tag` 加 `DocArea` 显示规约，失焦时自动保存 |
| `Tasklist.tsx` | 用 `Clipboard` 加 `ChecklistRow`；停用时显示为 `VacantSlot` |
| `ShiftToggle.tsx` | 用 `ShiftSign` 切换上班/下班，对应 `set_workspace_state` |
| `messages.ts` | |

testid：`team-name`、`team-rules`、`team-shift`、`tasklist`。

### office（工区）

| 文件 | 内容 |
|---|---|
| `OfficePage.tsx` | 每台 Machine 一排 `WorkRow`，加 `useCarry`；最后是 `AddRow` |
| `MemberSeat.tsx` | 把成员状态映射成 `Seat` 的 screen 和 pose |
| `MachineMenu.tsx` | 重命名（标题原地变成 `InlineText`）和移除 |
| `AddMachineDialog.tsx` | `Command`、等待连接的状态行，以及可展开的网络地址 |
| `RemoveMachineDialog.tsx` | |
| `messages.ts` | |

testid：`machine-row-{id}`、`machine-menu-{id}`、`add-machine`、`pair-command`、`pair-state`、
`remote-address`、`confirm-remove-machine`。

### settings（设置）

| 文件 | 内容 |
|---|---|
| `SettingsPage.tsx` | 一个 `Group`：语言用 `Select`，外观用 `ThemeThumbs` |
| `ThemeThumbs.tsx` | 三张主题缩略图 |
| `messages.ts` | |

### onboarding（引导）

| 文件 | 内容 |
|---|---|
| `OnboardingFlow.tsx` | 舞台、`Dashes`、固定底栏（左侧“稍后设置”，右侧主按钮）、步骤切换 |
| `useOnboarding.ts` | 步骤状态机和跳过规则 |
| `WelcomeStep.tsx`、`ConnectStep.tsx`、`ChooseAiStep.tsx`、`FirstMemberStep.tsx`、`ReadyStep.tsx` | 每个步骤一个文件，只组合 scene 组件和 design 组件 |
| `ChatPreview.tsx` | 用 `Message` 组成一段飞书对话 |
| `messages.ts` | |

### 跨 feature 的共用

成员状态到 `Status` 的映射，以及到工位屏幕状态的映射，members 和 office 都要用。
这段逻辑放在 `lib/queries/agents.ts` 的 `agentStatus(agent, machine, workspace)` 中，两个 feature 不直接互相引用。

## 原型对照

| 原型 | 拆成 |
|---|---|
| `renderSide`、`nav`、侧栏拖拽 IIFE | `Sidebar`、`SidebarGrip`、`NavItem`、`TeamSwitcher`、`useSidebarLayout` |
| `renderBar` | `TopBar` |
| `badge()` | `Hang`、`Badge`、`Avatar`、`StatusDot`，以及 feature 里的 `MemberCard` 和 `LeadCard` |
| `leadBack` | `BadgeVerso` 和 `FlipCard` |
| `pageMembers` | `MembersPage`、`VacantSlot`、`useSortable` |
| `pageMember` | `MemberBench`、`MemberConsole`、`Console`、`ConsoleScreen`、`ScreenReadout`、`ConsoleKey`、`SoulSlot`、`Callout`、`AdvancedDrawer` |
| `brainMenu`、`leverHTML`、`bindLever` | `BrainPicker`、`Popover`、`Lever` |
| `reactionMenu`、`avatarMenu`、`figureMenu` | `GridPicker`，以及 `ReactionPicker`、`AvatarPicker`、`FigurePicker` |
| `comboMenu`、`commitModel` | `Combo` |
| `pageTeam`、`checklist`、`taskRow` | `TeamPage`、`PaperLabel`、`Tag`、`RulesTag`、`Clipboard`、`ChecklistRow`、`ShiftSign`、`useHoleDrag` |
| `pageMachines`、`seat`、`screenOf` | `OfficePage`、`WorkRow`、`Seat`、`Figure`、`MemberSeat`、`useCarry` |
| `pageSettings` | `SettingsPage`、`Group`、`Select`、`ThemeThumbs` |
| `openMenu`、`mi` | `Menu`、`Menu.Item` |
| `openDialog`、`confirmDelete`、`addMachineDialog` | `Dialog`、`ConfirmDialog`、`DeleteMemberDialog`、`AddMachineDialog`、`Command` |
| `toast` | `ToastProvider`、`useToast` |
| `flyFrom`、`enterMember`、`leaveMember` | `useFlip` |
| 拉线 IIFE | `PullCord` 和 `lib/theme.ts` |
| `qrCode`、`qrTag` | `QrCode`、`QrTag` |
| `obWelcome` … `obReady`、`obFoot`、`obRender` | `OnboardingFlow` 和各个 `*Step` |
| `msg`、`obChat` | `Message`、`ChatPreview` |
| `obDesk` | `Seat` |

## dev 预览页

`#/design` 按 `design/`、`scene/`、`shell/` 分组，每个组件列出全部 variant 和状态，浅色和深色并排显示。
视觉验收会拿它和 `system.html` 对照。新增组件时，必须在 `dev/sections/` 中同时补上它的预览。
生产构建通过 `import.meta.env.DEV` 去掉整个 `dev/` 目录。
