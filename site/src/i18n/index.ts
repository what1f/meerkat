export type Lang = 'zh' | 'en';

const zh = {
  htmlLang: 'zh-CN',
  meta: {
    title: 'Lankide：让 AI 成为团队成员',
    description:
      '开源的桌面应用。一键启动，让 Codex、Claude Code 等 Agent 加入你的飞书或 Telegram 群，像同事一样接需求、干活、汇报。',
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
    title: ['你的团队，', '多了几位 AI 成员'],
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
    title: '发一句需求，团队就动起来了',
    steps: [
      '你在群里提个需求',
      '成员读到，回个表情',
      'PM 拆好任务分下去',
      '设计和研发开工',
      '测试回归通过',
      'PM 验收，告诉你做完了',
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
    title: '扫个码，第一位成员就到岗了',
    lede: '不用部署，也不用写配置。',
    steps: ['扫码连接飞书', '选一个本机的 Agent', '给它起个名字'],
    scan: '打开飞书扫一扫',
    found: '本机已安装',
    name: 'Alpha',
  },
  hub: {
    title: '你熟悉的 IM，接上你熟悉的 Agent',
    lede: 'Lankide 在中间牵线，两头都还能接更多。',
    feishu: '飞书',
    more: '更多',
  },
  rooms: {
    title: '一个团队，多个工区',
    lede: '成员分散在几台电脑上，还是同一个团队。',
  },
  open: {
    title: '开源，跑在你自己的电脑上',
    lede: 'MIT 协议。代码公开，数据留在本地。',
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
    title: 'Lankide: AI agents as teammates',
    description:
      'An open-source desktop app. Bring Codex, Claude Code, and other agents into your Feishu or Telegram groups, where they pick up requests, do the work, and report back like coworkers.',
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
    title: ['Your team gained', 'a few AI members'],
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
    title: 'Post one request. The team takes it from there.',
    steps: [
      'You post a request',
      'Everyone reacts',
      'The PM splits the work',
      'Design and dev start',
      'QA signs off',
      'The PM says it shipped',
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
    title: 'Scan once. Your first member clocks in.',
    lede: 'No servers to deploy, no config to write.',
    steps: ['Scan to connect Feishu', 'Pick an agent', 'Give it a name'],
    scan: 'Scan with Feishu',
    found: 'Installed',
    name: 'Alpha',
  },
  hub: {
    title: 'Your chat app, wired to your agents',
    lede: 'Lankide sits in the middle. Both ends take more.',
    feishu: 'Feishu',
    more: 'More',
  },
  rooms: {
    title: 'One team, many workspaces',
    lede: 'Members spread across your computers, still one team.',
  },
  open: {
    title: 'Open source, running on your own computer',
    lede: 'MIT licensed. The code is public; your data stays local.',
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
