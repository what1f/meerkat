import type { BrandMarkProps } from '../../design';
import type { Language } from '../../lib/i18n';

type Region = { base_url: string; key_url: string };

// Model services the Built-in Agent reaches with only an API key. Each speaks the
// OpenAI Chat Completions format; services with a mainland and a global site use the
// one that matches the interface language.
export type Provider = {
  id: string;
  name: { zh: string; en: string };
  mark: BrandMarkProps['brand'];
  cn?: Region;
  global: Region;
};

export const providers: Provider[] = [
  {
    id: 'deepseek',
    name: { zh: 'DeepSeek', en: 'DeepSeek' },
    mark: 'deepseek',
    global: {
      base_url: 'https://api.deepseek.com/v1',
      key_url: 'https://platform.deepseek.com/api_keys',
    },
  },
  {
    id: 'moonshot',
    name: { zh: 'Kimi', en: 'Kimi' },
    mark: 'moonshot',
    cn: {
      base_url: 'https://api.moonshot.cn/v1',
      key_url: 'https://platform.moonshot.cn/console/api-keys',
    },
    global: {
      base_url: 'https://api.moonshot.ai/v1',
      key_url: 'https://platform.moonshot.ai/console/api-keys',
    },
  },
  {
    id: 'zhipu',
    name: { zh: '智谱 GLM', en: 'Z.ai GLM' },
    mark: 'zhipu',
    cn: {
      base_url: 'https://open.bigmodel.cn/api/paas/v4',
      key_url: 'https://open.bigmodel.cn/usercenter/apikeys',
    },
    global: {
      base_url: 'https://api.z.ai/api/paas/v4',
      key_url: 'https://z.ai/manage-apikey/apikey-list',
    },
  },
  {
    id: 'qwen',
    name: { zh: '通义千问', en: 'Qwen' },
    mark: 'qwen',
    cn: {
      base_url: 'https://dashscope.aliyuncs.com/compatible-mode/v1',
      key_url: 'https://bailian.console.aliyun.com/?apiKey=1',
    },
    global: {
      base_url: 'https://dashscope-intl.aliyuncs.com/compatible-mode/v1',
      key_url: 'https://modelstudio.console.alibabacloud.com/?apiKey=1',
    },
  },
  {
    id: 'minimax',
    name: { zh: 'MiniMax', en: 'MiniMax' },
    mark: 'minimax',
    cn: {
      base_url: 'https://api.minimaxi.com/v1',
      key_url: 'https://platform.minimaxi.com/user-center/basic-information/interface-key',
    },
    global: {
      base_url: 'https://api.minimax.io/v1',
      key_url: 'https://www.minimax.io/platform/user-center/basic-information/interface-key',
    },
  },
  {
    id: 'openrouter',
    name: { zh: 'OpenRouter', en: 'OpenRouter' },
    mark: 'openrouter',
    global: { base_url: 'https://openrouter.ai/api/v1', key_url: 'https://openrouter.ai/keys' },
  },
  {
    id: 'openai',
    name: { zh: 'OpenAI', en: 'OpenAI' },
    mark: 'openai',
    global: {
      base_url: 'https://api.openai.com/v1',
      key_url: 'https://platform.openai.com/api-keys',
    },
  },
  {
    id: 'gemini',
    name: { zh: 'Gemini', en: 'Gemini' },
    mark: 'gemini',
    global: {
      base_url: 'https://generativelanguage.googleapis.com/v1beta/openai',
      key_url: 'https://aistudio.google.com/apikey',
    },
  },
  {
    id: 'xai',
    name: { zh: 'Grok', en: 'Grok' },
    mark: 'grok',
    global: { base_url: 'https://api.x.ai/v1', key_url: 'https://console.x.ai' },
  },
];

export const regionOf = (p: Provider, language: Language) =>
  (language === 'zh' && p.cn) || p.global;

// The service behind a saved endpoint, so a signed-in Computer shows its name and logo.
export const providerOfUrl = (url: string | null | undefined) => {
  if (!url) return undefined;
  const host = url.replace(/^https?:\/\//, '').split('/')[0];
  return providers.find((p) =>
    [p.cn, p.global].some(
      (r) => r && r.base_url.replace(/^https?:\/\//, '').split('/')[0] === host,
    ),
  );
};
