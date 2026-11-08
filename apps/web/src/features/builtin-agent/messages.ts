import { registerMessages } from '../../lib/i18n';

const en = {
  name: 'Meerkat',
  connectModel: 'Connect a model',
  chatgpt: 'ChatGPT',
  other: 'Other',
  service: 'Service',
  account: 'Connected to',
  switch: 'Switch',
  dialogTitle: 'Connect a model on {machine}',
  openBrowser: 'Sign in with browser',
  baseUrl: 'Endpoint URL',
  key: 'API key',
  pasteKey: 'Paste API key',
  getKey: 'Get a key',
  connect: 'Connect',
  invalidKey: "This key doesn't work",
  unreachable: "Can't reach this endpoint",
  waiting: 'Finish signing in in your browser',
  cancel: 'Cancel',
  signOut: 'Disconnect',
  failed: "Couldn't connect. Try again.",
};

declare module '../../lib/i18n' {
  interface MessageCatalog {
    builtinAgent: typeof en;
  }
}

registerMessages('builtinAgent', {
  en,
  zh: {
    name: 'Meerkat',
    connectModel: '接入模型',
    chatgpt: 'ChatGPT',
    other: '其他',
    service: '服务商',
    account: '当前接入',
    switch: '切换',
    dialogTitle: '为 {machine} 接入模型',
    openBrowser: '在浏览器中登录',
    baseUrl: '接口地址',
    key: 'API Key',
    pasteKey: '粘贴 API Key',
    getKey: '获取 Key',
    connect: '连接',
    invalidKey: '这个 Key 用不了',
    unreachable: '连不上这个地址',
    waiting: '在浏览器中完成登录',
    cancel: '取消',
    signOut: '断开',
    failed: '没能接入，再试一次',
  },
});
