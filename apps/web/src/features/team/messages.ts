import { registerMessages } from '../../lib/i18n';
const en = {
  name: 'Team name',
  rules: 'Rules',
  rulesPlaceholder: 'Agreements every member follows',
  tasks: 'Task list',
  more: 'More',
  turnOff: 'Turn off',
  unavailable: 'The task list can’t be opened right now',
  retry: 'Try again',
  shift: 'Shift',
  on: 'Open',
  off: 'Closed',
  failed: 'Couldn’t save changes',
};
declare module '../../lib/i18n' {
  interface MessageCatalog {
    team: typeof en;
  }
}
registerMessages('team', {
  en,
  zh: {
    name: '团队名',
    rules: '规约',
    rulesPlaceholder: '所有成员都会遵守的约定',
    tasks: '任务清单',
    more: '更多',
    turnOff: '停用',
    unavailable: '任务清单暂时打不开',
    retry: '重试',
    shift: '上班或下班',
    on: '上班',
    off: '下班',
    failed: '没有保存成功',
  },
});
