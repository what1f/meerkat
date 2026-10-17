import { useState } from 'react';
import * as D from '../../design';
import { Reaction } from '../../scene';
import { Spec, State } from '../Spec';
const options = ['One', 'Two', 'Three'].map((value) => ({ value, label: value }));
const cells = ['SMILE', 'CLAP', 'Coffee', 'CheckMark', 'AWESOME', 'BEER'].map((id) => ({
  value: id,
  label: id,
  content: <Reaction id={id} size={24} />,
}));
export function DesignLayout() {
  const [value, setValue] = useState('One');
  const [pick, setPick] = useState('SMILE');
  const [open, setOpen] = useState(false);
  const [mode, setMode] = useState<'light' | 'dark' | 'system'>('system');
  return (
    <>
      <Spec name="Group" surface="surface">
        <D.Group>
          <D.Group.Row label="Appearance">
            <D.Segmented size="sm" options={options} value={value} onChange={setValue} />
          </D.Group.Row>
          <D.Group.Row label="Details">
            <D.LinkButton disclosure open={open} onClick={() => setOpen(!open)}>
              More
            </D.LinkButton>
          </D.Group.Row>
          <D.Reveal open={open}>
            <D.Group.Row label="Revealed" align="top">
              <D.TextInput size="sm" value={value} onChange={setValue} placeholder="Value" />
            </D.Group.Row>
          </D.Reveal>
        </D.Group>
      </Spec>
      <Spec name="ThemeThumb">
        {(['light', 'dark', 'system'] as const).map((m) => (
          <D.ThemeThumb
            key={m}
            mode={m}
            selected={mode === m}
            label={m}
            onSelect={() => setMode(m)}
          />
        ))}
      </Spec>
      <Spec name="OptionColumn" surface="surface">
        <D.OptionColumn title="Models">
          {options.map((o) => (
            <D.OptionColumn.Item
              key={o.value}
              selected={value === o.value}
              dimmed={o.value === 'Three'}
              mono
              onSelect={() => setValue(o.value)}
            >
              {o.label}
            </D.OptionColumn.Item>
          ))}
        </D.OptionColumn>
      </Spec>
      <Spec name="GridPicker" surface="surface">
        <State label="inline">
          <D.GridPicker
            cells={cells}
            value={pick}
            onPick={setPick}
            columns={3}
            leading={<D.IconButton icon="upload" label="Upload" onClick={() => {}} />}
          />
        </State>
        <State label="in popover">
          <D.Popover trigger={<D.Button>Pick</D.Button>}>
            <D.GridPicker cells={cells} value={pick} onPick={setPick} columns={3} />
          </D.Popover>
        </State>
      </Spec>
      <Spec name="Menu and Popover">
        <D.Menu trigger={<D.Button>Menu</D.Button>}>
          <D.Menu.Item onSelect={() => {}} checked>
            Selected
          </D.Menu.Item>
          <D.Menu.Item onSelect={() => {}}>Rename</D.Menu.Item>
          <D.Menu.Separator />
          <D.Menu.Item onSelect={() => {}} danger>
            Delete
          </D.Menu.Item>
        </D.Menu>
        <D.Popover trigger={<D.IconButton icon="more" label="More" onClick={() => {}} />}>
          Popover content
        </D.Popover>
      </Spec>
      <Spec name="QrCode" surface="surface">
        <D.QrCode value="https://example.com" size={128} aria-label="QR code" />
      </Spec>
    </>
  );
}
