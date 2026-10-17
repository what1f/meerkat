import type { UIProps } from './types';
import QRCode from 'qrcode';
import css from './QrCode.module.css';
export type QrCodeProps = Omit<UIProps<SVGSVGElement>, 'className' | 'style'> & {
  value: string;
  size: number;
};
export function QrCode({ value, size, ...rest }: QrCodeProps) {
  if (!value) return <svg {...rest} className={css.root} width={size} height={size} />;
  const matrix = QRCode.create(value).modules;
  const quiet = 4;
  return (
    <svg
      {...rest}
      className={css.root}
      width={size}
      height={size}
      viewBox={`0 0 ${matrix.size + quiet * 2} ${matrix.size + quiet * 2}`}
      role="img"
      aria-label={rest['aria-label']}
    >
      <rect width="100%" height="100%" fill="white" />
      {Array.from(matrix.data, (dark, index) =>
        dark ? (
          <rect
            key={index}
            x={(index % matrix.size) + quiet}
            y={Math.floor(index / matrix.size) + quiet}
            width="1"
            height="1"
            fill="black"
          />
        ) : null,
      )}
    </svg>
  );
}
