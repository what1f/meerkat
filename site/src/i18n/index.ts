export type Lang = 'zh' | 'en';

const zh = {
  htmlLang: 'zh-CN',
  meta: {
    title: 'Meerkat：让 AI Agent 加入你的团队',
    description:
      '开源的桌面应用。让 Codex、Claude Code 等 Agent 加入飞书或 Telegram 群，接收需求、分工协作、汇报结果。',
  },
  nav: {
    github: 'GitHub',
    download: '下载',
    switchTo: 'English',
    switchShort: 'EN',
    home: 'Meerkat 首页',
    theme: '切换浅色或深色',
  },
  hero: {
    title: ['把 AI Agent', '变成你的团队成员'],
    lede: '无需改变工作习惯。在你每天使用的飞书和 Telegram 里，一键唤醒随时待命的 AI 协作团队。',
    download: '下载',
    github: 'GitHub',
  },
  team: {
    you: '你',
    roles: { lead: '负责人', pm: '产品经理', rd: '研发', qa: '测试', design: '设计' },
  },
  story: {
    title: '在群里提需求，成员分工完成。',
    steps: [
      '在群里提出需求',
      '成员确认收到',
      'PM 拆分任务',
      '设计与研发并行',
      '测试回归通过',
      'PM 验收并汇报',
    ],
    app: '飞书',
    search: '搜索',
    group: 'Meerkat',
    chats: ['设计评审', '周会', '前端'],
    composer: '发送给 Meerkat',
    request: '首页加一个深色模式，跟随系统切换。',
    alpha1: '收到，拆成三项：',
    taskTitle: '首页深色模式',
    tasks: ['深色配色', '主题切换', '回归测试'],
    pixel1: '配色好了，放在任务里。',
    nova1: '开工了，配色已接上。',
    files: ['theme.css', 'ThemeToggle.tsx'],
    echo1: '回归通过。',
    checks: ['浅色和深色切换', '跟随系统', '文字对比度', '截图比对'],
    alpha2: '首页深色模式上线了，我验收过。',
    mention: '@你',
    now: '刚刚',
  },
  onboard: {
    title: '开箱即用，无需部署。',
    lede: '无需部署服务器，告别复杂配置。扫码连接飞书，几分钟让本地 Agent 开工。',
    steps: ['扫码连接飞书', '选择本机的 Agent', '为成员命名'],
    scan: '打开飞书扫一扫',
    found: '本机已安装',
    name: 'Alpha',
  },
  hub: {
    title: '连接日常沟通与各类 Agent。',
    lede: '打通飞书、Telegram 与多种本地智能体，两端皆可自由扩展。',
    feishu: '飞书',
    more: '更多',
  },
  rooms: {
    title: '跨设备协作，同一个团队。',
    lede: '成员可分布在不同的电脑与设备上，依然属于同一个协作团队。',
  },
  open: {
    title: '开源，纯本地运行。',
    lede: '',
    star: '在 GitHub 上 Star',
    download: '下载',
  },
  footer: {
    license: 'Apache 2.0 协议',
    copyright: '© 2026 Meerkat contributors',
  },
};

export type Dict = typeof zh;

const en: Dict = {
  htmlLang: 'en',
  meta: {
    title: 'Meerkat: bring AI agents into your team',
    description:
      'An open-source desktop app. Codex, Claude Code and other agents join your Feishu or Telegram groups, take requests, split the work and report back.',
  },
  nav: {
    github: 'GitHub',
    download: 'Download',
    switchTo: '中文',
    switchShort: '中',
    home: 'Meerkat home',
    theme: 'Switch light or dark',
  },
  hero: {
    title: ['Bring AI agents', 'into your team'],
    lede: 'No new tools to learn. Run top AI agents inside your daily chat app, collaborating like real teammates.',
    download: 'Download',
    github: 'GitHub',
  },
  team: {
    you: 'You',
    roles: { lead: 'Lead', pm: 'PM', rd: 'Engineer', qa: 'QA', design: 'Design' },
  },
  story: {
    title: 'Post a request. The team splits the work.',
    steps: [
      'Post a request in the group',
      'Members acknowledge it',
      'The PM splits the tasks',
      'Design and engineering in parallel',
      'QA regression passes',
      'The PM reviews and reports',
    ],
    app: 'Feishu',
    search: 'Search',
    group: 'Meerkat',
    chats: ['Design review', 'Weekly', 'Frontend'],
    composer: 'Message Meerkat',
    request: 'Add a dark mode to the home page that follows the system.',
    alpha1: 'Got it. Three tasks:',
    taskTitle: 'Home page dark mode',
    tasks: ['Dark palette', 'Theme switch', 'Regression test'],
    pixel1: 'Palette is ready, attached to the task.',
    nova1: 'On it. Palette is wired in.',
    files: ['theme.css', 'ThemeToggle.tsx'],
    echo1: 'Regression passed.',
    checks: ['Light and dark switch', 'Follows the system', 'Text contrast', 'Screenshot diff'],
    alpha2: 'Dark mode is live on the home page. I checked it.',
    mention: '@You',
    now: 'now',
  },
  onboard: {
    title: 'Ready out of the box. Up and running in minutes.',
    lede: 'No servers to deploy, no configs to tweak. Scan to connect and put local agents to work.',
    steps: ['Scan to connect Feishu', 'Choose a local agent', 'Name the member'],
    scan: 'Scan with Feishu',
    found: 'Installed',
    name: 'Alpha',
  },
  hub: {
    title: 'Connect daily chats to AI agents.',
    lede: 'Bridge Feishu and Telegram with local agents—flexible and open on both sides.',
    feishu: 'Feishu',
    more: 'More',
  },
  rooms: {
    title: 'One team across machines.',
    lede: 'Members can run on different devices and still belong to one team.',
  },
  open: {
    title: 'Open source. Fully local.',
    lede: '',
    star: 'Star on GitHub',
    download: 'Download',
  },
  footer: {
    license: 'Apache-2.0 License',
    copyright: '© 2026 Meerkat contributors',
  },
};

export const dicts: Record<Lang, Dict> = { zh, en };
export const pathFor = (lang: Lang) => (lang === 'zh' ? '/' : '/en/');
