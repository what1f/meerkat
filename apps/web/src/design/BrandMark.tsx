import type { UIProps } from './types';
import feishu from '../assets/brand/feishu.svg';
import telegram from '../assets/brand/telegram.svg';
import codex from '../assets/brand/lobehub/codex.svg';
import claudeCode from '../assets/brand/lobehub/claude-code.svg';
import pi from '../assets/brand/lobehub/pi.svg';
import deepseek from '../assets/brand/lobehub/deepseek.svg';
import claude from '../assets/brand/lobehub/claude.svg';
import anthropic from '../assets/brand/lobehub/anthropic.svg';
import openai from '../assets/brand/lobehub/openai.svg';
import gemini from '../assets/brand/lobehub/gemini.svg';
import qwen from '../assets/brand/lobehub/qwen.svg';
import mistral from '../assets/brand/lobehub/mistral.svg';
import moonshot from '../assets/brand/lobehub/moonshot.svg';
import meta from '../assets/brand/lobehub/meta.svg';
import grok from '../assets/brand/lobehub/grok.svg';
const marks = {
  feishu,
  telegram,
  codex,
  claude_code: claudeCode,
  pi,
  deepseek,
  claude,
  anthropic,
  openai,
  gemini,
  qwen,
  mistral,
  moonshot,
  meta,
  grok,
};
import css from './BrandMark.module.css';
export type BrandMarkProps = Omit<UIProps, 'className' | 'style'> & {
  brand: keyof typeof marks;
  size?: number;
};
export function BrandMark({ brand, size = 20, ...rest }: BrandMarkProps) {
  return (
    <img
      className={css.root}
      data-brand={brand}
      src={marks[brand]}
      width={size}
      height={size}
      alt={brand}
      {...rest}
    />
  );
}

// The vendor behind a model name; the first rule that matches wins.
const vendors: [RegExp, ModelBrand][] = [
  [/deepseek/, 'deepseek'],
  [/claude/, 'claude'],
  [/anthropic/, 'anthropic'],
  [/codex/, 'codex'],
  [/gpt|openai|^o[134]($|-)|\/o[134]/, 'openai'],
  [/gemini|gemma/, 'gemini'],
  [/qwen|tongyi|dashscope/, 'qwen'],
  [/mistral|mixtral|codestral/, 'mistral'],
  [/moonshot|kimi/, 'moonshot'],
  [/llama|meta/, 'meta'],
  [/grok|xai/, 'grok'],
];
export type ModelBrand =
  | 'deepseek'
  | 'claude'
  | 'anthropic'
  | 'codex'
  | 'openai'
  | 'gemini'
  | 'qwen'
  | 'mistral'
  | 'moonshot'
  | 'meta'
  | 'grok';
export function modelBrand(...names: string[]): ModelBrand | undefined {
  for (const name of names) {
    const text = name.toLowerCase();
    const hit = vendors.find(([re]) => re.test(text));
    if (hit) return hit[1];
  }
  return undefined;
}
