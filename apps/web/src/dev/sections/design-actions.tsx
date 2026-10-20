import { useState } from 'react';
import * as D from '../../design';
import { Spec, State } from '../Spec';
export function DesignActions() {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Spec name="Button">
        {(['default', 'primary', 'quiet', 'danger'] as const).map((variant) => (
          <State key={variant} label={variant}>
            <D.Button variant={variant}>Save</D.Button>
          </State>
        ))}
        {(['sm', 'md', 'lg'] as const).map((size) => (
          <State key={size} label={size}>
            <D.Button size={size}>Save</D.Button>
          </State>
        ))}
        <State label="disabled">
          <D.Button disabled>Save</D.Button>
        </State>
        <State label="loading">
          <D.Button loading>Save</D.Button>
        </State>
        <State label="leading">
          <D.Button leading={<D.Icon name="plus" />}>Add</D.Button>
        </State>
      </Spec>
      <Spec name="IconButton">
        {(['md', 'sm'] as const).map((size) => (
          <State key={size} label={size}>
            <D.IconButton icon="more" label="More" size={size} onClick={() => {}} />
          </State>
        ))}
      </Spec>
      <Spec name="LinkButton">
        <State label="plain">
          <D.LinkButton onClick={() => {}}>Learn more</D.LinkButton>
        </State>
        <State label={open ? 'disclosure open' : 'disclosure closed'}>
          <D.LinkButton disclosure open={open} onClick={() => setOpen(!open)}>
            Details
          </D.LinkButton>
        </State>
      </Spec>
      <Spec name="Icon">
        {([10, 12, 14, 15, 16, 18, 20, 22] as const).map((size) => (
          <State key={size} label={String(size)}>
            <D.Icon name="check" size={size} />
          </State>
        ))}
      </Spec>
    </>
  );
}
