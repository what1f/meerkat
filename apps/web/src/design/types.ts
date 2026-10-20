import type { AriaAttributes, DOMAttributes } from 'react';
export type UIProps<E extends Element = HTMLElement> = Omit<
  AriaAttributes & DOMAttributes<E>,
  'children' | 'dangerouslySetInnerHTML' | 'onChange' | 'onSelect' | 'className' | 'style'
> & { 'data-testid'?: string };
export type Status = 'ok' | 'busy' | 'idle' | 'err';
