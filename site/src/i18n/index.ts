export type Lang = 'zh' | 'en';

const zh = {
  htmlLang: 'zh-CN',
  meta: {
    title: 'Lankide：让 AI Agent 加入你的团队',
    description:
      '开源的桌面应用。让 Codex、Claude Code 等 Agent 加入飞书或 Telegram 群，接收需求、分工协作、汇报结果。',
  },
  nav: {
    github: 'GitHub',
    download: '下载',
    switchTo: 'English',
    switchShort: 'EN',
    home: 'Lankide 首页',
    theme: '切换浅色或深色',
  },
  hero: {
    title: ['给你的团队', '添几个 AI 成员'],
    lede: '一键启动，让 AI Agent 跑在你最熟悉的 IM 里，像同事一样和你协作。',
    download: '下载',
    github: 'GitHub',
    board: 'Lankide',
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
    group: 'Lankide',
    chats: ['设计评审', '周会', '前端'],
    composer: '发送给 Lankide',
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
    title: '扫码添加，无需部署。',
    lede: '不需要服务器，也不需要配置文件。',
    steps: ['扫码连接飞书', '选择本机的 Agent', '为成员命名'],
    scan: '打开飞书扫一扫',
    found: '本机已安装',
    name: 'Alpha',
  },
  hub: {
    title: '连接常用的 IM 与 Agent。',
    lede: 'Lankide 位于两者之间，两端都支持扩展。',
    feishu: '飞书',
    more: '更多',
  },
  rooms: {
    title: '一个团队，多个工区。',
    lede: '成员分布在不同的电脑上，仍属于同一个团队。',
  },
  open: {
    title: '开源，纯本地运行。',
    lede: 'MIT 协议，代码公开，数据保存在本地。',
    star: '在 GitHub 上 Star',
    download: '下载',
  },
  footer: {
    license: 'MIT 协议',
    copyright: '© 2026 Lankide contributors',
  },
};

export type Dict = typeof zh;

const en: Dict = {
  htmlLang: 'en',
  meta: {
    title: 'Lankide: bring AI agents into your team',
    description:
      'An open-source desktop app. Codex, Claude Code and other agents join your Feishu or Telegram groups, take requests, split the work and report back.',
  },
  nav: {
    github: 'GitHub',
    download: 'Download',
    switchTo: '中文',
    switchShort: '中',
    home: 'Lankide home',
    theme: 'Switch light or dark',
  },
  hero: {
    title: ['Add a few AI members', 'to your team'],
    lede: 'One click, and AI agents run inside the chat app you already live in, working with you like coworkers.',
    download: 'Download',
    github: 'GitHub',
    board: 'Lankide',
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
    group: 'Lankide',
    chats: ['Design review', 'Weekly', 'Frontend'],
    composer: 'Message Lankide',
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
    title: 'Add a member with a scan.',
    lede: 'No server, no config files.',
    steps: ['Scan to connect Feishu', 'Choose a local agent', 'Name the member'],
    scan: 'Scan with Feishu',
    found: 'Installed',
    name: 'Alpha',
  },
  hub: {
    title: 'Connects chat apps to agents.',
    lede: 'Lankide sits between them, and both sides are extensible.',
    feishu: 'Feishu',
    more: 'More',
  },
  rooms: {
    title: 'One team, many workspaces.',
    lede: 'Members can run on different machines and still belong to one team.',
  },
  open: {
    title: 'Open source. Fully local.',
    lede: 'MIT licensed. The code is public and your data stays on your machine.',
    star: 'Star on GitHub',
    download: 'Download',
  },
  footer: {
    license: 'MIT License',
    copyright: '© 2026 Lankide contributors',
  },
};

export const dicts: Record<Lang, Dict> = { zh, en };
export const pathFor = (lang: Lang) => (lang === 'zh' ? '/' : '/en/');
