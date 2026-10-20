import { useState } from 'react';
import * as D from '../../design';
import { Spec, State } from '../Spec';
const options = ['One', 'Two', 'Three'].map((value) => ({ value, label: value }));
export function DesignFields() {
  const [value, setValue] = useState('One');
  const [text, setText] = useState('Text');
  const [empty, setEmpty] = useState('');
  return (
    <>
      <Spec name="TextInput" surface="surface">
        <State label="md">
          <D.TextInput value={text} onChange={setText} placeholder="Name" aria-label="Name" />
        </State>
        <State label="empty">
          <D.TextInput value={empty} onChange={setEmpty} placeholder="Name" aria-label="Name" />
        </State>
        <State label="sm mono">
          <D.TextInput size="sm" mono value={text} onChange={setText} placeholder="Address" />
        </State>
        <State label="lg mono center invalid">
          <D.TextInput
            size="lg"
            mono
            align="center"
            invalid
            value={text}
            onChange={setText}
            placeholder="Model"
          />
        </State>
      </Spec>
      <Spec name="DocArea" surface="surface">
        <D.DocArea value={text} onChange={setText} placeholder="Notes" />
      </Spec>
      <Spec name="InlineText" surface="surface">
        {(['display', 'heading', 'title', 'body', 'small'] as const).map((textStyle) => (
          <State key={textStyle} label={textStyle}>
            <D.InlineText
              value={text}
              onCommit={setText}
              placeholder="Edit"
              textStyle={textStyle}
            />
          </State>
        ))}
        <State label="multiline">
          <D.InlineText
            value={text}
            onCommit={setText}
            placeholder="Notes"
            textStyle="body"
            multiline
          />
        </State>
        <State label="live">
          <D.InlineText
            mode="live"
            value={text}
            onChange={setText}
            placeholder="Live"
            textStyle="body"
          />
        </State>
      </Spec>
      <Spec name="SearchField" surface="surface">
        <D.SearchField value={text} onChange={setText} onEnter={() => {}} placeholder="Search" />
      </Spec>
      <Spec name="Select" surface="surface">
        {(['quiet', 'boxed'] as const).map((variant) => (
          <State key={variant} label={variant}>
            <D.Select
              variant={variant}
              options={[...options, { value: 'Off', label: 'Off', disabled: true }]}
              value={value}
              onChange={setValue}
              aria-label="Select"
            />
          </State>
        ))}
      </Spec>
      <Spec name="Segmented" surface="surface">
        {(['md', 'sm'] as const).map((size) => (
          <State key={size} label={size}>
            <D.Segmented size={size} options={options} value={value} onChange={setValue} />
          </State>
        ))}
      </Spec>
    </>
  );
}
