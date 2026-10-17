import type { UIProps } from './types';
import feishu from '../assets/brand/feishu.svg';
import telegram from '../assets/brand/telegram.svg';
import codex from '../assets/brand/lobehub/codex.svg';
import claudeCode from '../assets/brand/lobehub/claude-code.svg';
import pi from '../assets/brand/lobehub/pi.svg';
const marks = { feishu, telegram, codex, 'claude-code': claudeCode, pi };
import css from './BrandMark.module.css';
export type BrandMarkProps = Omit<UIProps, 'className' | 'style'> & {
  brand: 'feishu' | 'telegram' | 'codex' | 'claude-code' | 'pi';
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
