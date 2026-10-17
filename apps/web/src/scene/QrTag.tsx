import type { UIProps } from '../design/index';
import { BrandMark, Button, QrCode, Avatar, Skeleton } from '../design';
import css from './QrTag.module.css';
export type QrTagProps = Omit<UIProps, 'className' | 'style'> & {
  state: 'loading' | 'wait' | 'scanned' | 'expired';
  value: string;
  brand: 'feishu' | 'telegram';
  scannerAvatar?: string;
  onRefresh: () => void;
  refreshLabel: string;
};
export function QrTag({
  state,
  value,
  brand,
  scannerAvatar,
  onRefresh,
  refreshLabel,
  ...rest
}: QrTagProps) {
  return (
    <section {...rest} className={css.root} data-state={state} aria-busy={state === 'loading'}>
      <BrandMark brand={brand} />
      {state === 'loading' && <Skeleton shape="line" width={180} height={180} />}
      {state !== 'loading' && <QrCode value={value} size={180} />}
      {state === 'scanned' && <Avatar src={scannerAvatar} size={40} />}
      {state === 'expired' && <Button onClick={onRefresh}>{refreshLabel}</Button>}
    </section>
  );
}
