import type { UIProps } from './types';
import feishu from '../assets/brand/feishu.svg';
import telegram from '../assets/brand/telegram.svg';
import codex from '../assets/brand/lobehub/codex.svg';
import claudeCode from '../assets/brand/lobehub/claude-code.svg';
import pi from '../assets/brand/lobehub/pi.svg';
import deepseek from '../assets/brand/lobehub/deepseek.svg';
import claude from '../assets/brand/lobehub/claude.svg';
import openai from '../assets/brand/lobehub/openai.svg';
import gemini from '../assets/brand/lobehub/gemini.svg';
import qwen from '../assets/brand/lobehub/qwen.svg';
import mistral from '../assets/brand/lobehub/mistral.svg';
import moonshot from '../assets/brand/lobehub/moonshot.svg';
import meta from '../assets/brand/lobehub/meta.svg';
import grok from '../assets/brand/lobehub/grok.svg';
import zhipu from '../assets/brand/lobehub/zhipu.svg';
import minimax from '../assets/brand/lobehub/minimax.svg';
const marks = {
  feishu,
  telegram,
  codex,
  claude_code: claudeCode,
  pi,
  deepseek,
  claude,
  openai,
  gemini,
  qwen,
  mistral,
  moonshot,
  meta,
  grok,
  zhipu,
  minimax,
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
  [/gpt/, 'openai'],
  [/gemini|gemma/, 'gemini'],
  [/qwen/, 'qwen'],
  [/mistral|mixtral|codestral/, 'mistral'],
  [/moonshot|kimi/, 'moonshot'],
  [/llama|meta/, 'meta'],
  [/grok|xai/, 'grok'],
  [/glm|zhipu/, 'zhipu'],
  [/minimax/, 'minimax'],
];
export type ModelBrand =
  | 'deepseek'
  | 'claude'
  | 'openai'
  | 'gemini'
  | 'qwen'
  | 'mistral'
  | 'moonshot'
  | 'meta'
  | 'grok'
  | 'zhipu'
  | 'minimax';
export function modelBrand(...names: string[]): ModelBrand | undefined {
  for (const name of names) {
    const text = name.toLowerCase();
    const hit = vendors.find(([re]) => re.test(text));
    if (hit) return hit[1];
  }
  return undefined;
}
