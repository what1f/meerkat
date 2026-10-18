import type { UIProps } from '../design/index';
import { Avatar, BrandMark, Icon, IconButton, QrCode, Skeleton } from '../design';
import peg from './peg.module.css';
import css from './QrTag.module.css';
export type QrTagProps = Omit<UIProps, 'className' | 'style'> & {
  state: 'loading' | 'wait' | 'scanned' | 'expired';
  value: string;
  brand: 'feishu' | 'telegram';
  scannerAvatar?: string;
  onRefresh: () => void;
  refreshLabel: string;
  /** Closing the tag; shown only when given. */
  onClose?: () => void;
  closeLabel?: string;
};
// The sign-in code hangs as a tag; once scanned it shows who is signing in.
export function QrTag({
  state,
  value,
  brand,
  scannerAvatar,
  onRefresh,
  refreshLabel,
  onClose,
  closeLabel,
  ...rest
}: QrTagProps) {
  return (
    <section {...rest} className={peg.hang} data-state={state} aria-busy={state === 'loading'}>
      <span className={peg.peg} data-peg="" />
      <span className={peg.cord} />
      <div className={peg.shadow}>
        <div className={css.tag}>
          <span className={peg.eyelet} />
          {onClose && (
            <span className={css.close}>
              <IconButton icon="x" size="sm" label={closeLabel ?? ''} onClick={onClose} />
            </span>
          )}
          <div className={css.qr}>
            {state === 'loading' ? (
              <Skeleton shape="line" width="100%" height="100%" />
            ) : (
              <QrCode value={value} size={192} />
            )}
            <span className={css.logo}>
              <BrandMark brand={brand} size={26} />
            </span>
            {state === 'scanned' && (
              <span className={css.scanned}>
                <Avatar src={scannerAvatar} size={60} />
                <span className={css.ok}>
                  <Icon name="check" size={12} />
                </span>
              </span>
            )}
            {state === 'expired' && (
              <button
                className={css.refresh}
                type="button"
                aria-label={refreshLabel}
                title={refreshLabel}
                onClick={onRefresh}
              >
                <Icon name="refresh" size={22} />
              </button>
            )}
          </div>
        </div>
      </div>
    </section>
  );
}
