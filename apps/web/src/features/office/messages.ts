import { registerMessages } from '../../lib/i18n';

const en = {
  add: 'Add computer',
  addTitle: 'Add computer',
  runHint: 'Run this in a terminal on the other computer',
  waiting: 'Waiting to connect',
  network: 'Network',
  apply: 'Apply',
  addressPlaceholder: '192.168.1.8:7443',
  done: 'Done',
  more: 'More',
  builtin: 'Meerkat',
  rename: 'Rename',
  remove: 'Remove',
  namePlaceholder: 'Name',
  removeTitle: 'Remove {name}?',
  removeBody: 'Pair again to reconnect.',
  moveCrewFirst: 'Move the members here to another computer first',
  pairFailed: 'Couldn’t create a pairing command',
  moveFailed: 'Couldn’t move to this computer',
  cancelMove: 'Stay put',
  moveWithoutFiles: 'Move without files',
  renameFailed: 'Couldn’t rename',
  removeFailed: 'Couldn’t remove',
  loadFailed: 'Couldn’t load the office',
  retry: 'Try again',
};

declare module '../../lib/i18n' {
  interface MessageCatalog {
    office: typeof en;
  }
}

registerMessages('office', {
  en,
  zh: {
    add: '添加工区',
    addTitle: '添加工区',
    runHint: '在另一台电脑的终端里运行',
    waiting: '等待连接',
    network: '网络',
    apply: '应用',
    addressPlaceholder: '192.168.1.8:7443',
    done: '完成',
    more: '更多',
    builtin: 'Meerkat',
    rename: '重命名',
    remove: '移除',
    namePlaceholder: '工区名',
    removeTitle: '移除{name}？',
    removeBody: '再次连接需要重新配对。',
    moveCrewFirst: '先把这里的成员换到其他工区',
    pairFailed: '没能生成配对命令',
    moveFailed: '没能换到这个工区',
    cancelMove: '不搬了',
    moveWithoutFiles: '不带文件直接搬',
    renameFailed: '没能重命名',
    removeFailed: '没能移除',
    loadFailed: '工区暂时加载不出来',
    retry: '重试',
  },
});
