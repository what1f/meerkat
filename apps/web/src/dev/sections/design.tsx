import { DesignIdentity } from './design-identity';
import { DesignFeedback } from './design-feedback';
import { DesignInteractions } from './design-interactions';
import { useState } from 'react';
import * as D from '../../design';
const avatar = new URL('../../assets/avatars/01.jpg', import.meta.url).href;
export function DesignSection() {
  const [value, setValue] = useState('One');
  const [checked, setChecked] = useState(false);
  const [mode, setMode] = useState<'light' | 'dark' | 'system'>('system');
  const [committed, setCommitted] = useState('');
  const options = ['One', 'Two', 'Three'].map((value) => ({
    value,
    label: value,
    icon: <D.Icon name="check" />,
  }));
  return (
    <section aria-label="Design">
      <h2>Design</h2>
      {(['default', 'primary', 'quiet', 'danger'] as const).map((variant) => (
        <D.Button key={variant} variant={variant}>
          {variant}
        </D.Button>
      ))}
      {(['sm', 'md', 'lg'] as const).map((size) => (
        <D.Button key={size} size={size}>
          {size}
        </D.Button>
      ))}
      {([10, 12, 14, 15, 16, 18, 20, 22] as const).map((size) => (
        <D.Icon key={size} name="check" size={size} />
      ))}
      <D.Button disabled>Disabled</D.Button>
      <D.Button loading>Loading</D.Button>
      <D.Button leading={<D.Icon name="plus" />}>Add</D.Button>
      <D.IconButton icon="plus" label="Add" onClick={() => {}} />
      <D.Popover trigger={<D.IconButton icon="more" label="Icon trigger" onClick={() => {}} />}>
        Icon content
      </D.Popover>
      <D.Popover trigger={<D.LinkButton onClick={() => {}}>Link trigger</D.LinkButton>}>
        Link content
      </D.Popover>
      <D.Popover
        trigger={<D.Avatar src={avatar} size={40} onEdit={() => {}} editLabel="Avatar trigger" />}
      >
        Avatar content
      </D.Popover>
      <D.LinkButton disclosure open={checked} onClick={() => setChecked(!checked)}>
        Details
      </D.LinkButton>
      <D.TextInput value={value} onChange={setValue} placeholder="Name" aria-label="Name" />
      <D.TextInput
        value={value}
        onChange={setValue}
        placeholder="Model"
        invalid
        mono
        align="center"
        size="lg"
      />
      <D.DocArea value={value} onChange={setValue} placeholder="Notes" />
      {(['display', 'heading', 'title', 'body', 'small'] as const).map((textStyle) => (
        <D.InlineText
          key={textStyle}
          value={value}
          onCommit={setValue}
          placeholder="Edit"
          textStyle={textStyle}
        />
      ))}
      <D.InlineText
        value={value}
        onCommit={setValue}
        placeholder="Edit notes"
        textStyle="body"
        multiline
      />

      <D.Segmented options={options} value={value} onChange={setValue} />
      <D.Segmented size="sm" options={options} value={value} onChange={setValue} />

      <D.Select options={options} value={value} onChange={setValue} aria-label="Quiet selection" />
      <D.Select
        options={[...options, { value: 'Disabled', label: 'Disabled', disabled: true }]}
        value={value}
        onChange={setValue}
        variant="boxed"
        aria-label="Boxed selection"
      />
      <D.SearchField
        value={value}
        onChange={setValue}
        onEnter={setCommitted}
        placeholder="Search or type"
      />
      <output data-testid="search-commit">{committed}</output>
      <D.OptionColumn title="Models">
        {options.map((o) => (
          <D.OptionColumn.Item
            key={o.value}
            selected={value === o.value}
            dimmed={o.value === 'Three'}
            mono
            leading={o.icon}
            onSelect={() => setValue(o.value)}
          >
            {o.label}
          </D.OptionColumn.Item>
        ))}
      </D.OptionColumn>
      {(['light', 'dark', 'system'] as const).map((m) => (
        <D.ThemeThumb
          key={m}
          mode={m}
          selected={mode === m}
          label={m}
          onSelect={() => setMode(m)}
        />
      ))}
      <D.InlineText
        mode="live"
        value={value}
        onChange={setValue}
        placeholder="Live name"
        textStyle="heading"
      />
      <D.TextInput size="sm" value={value} onChange={setValue} mono placeholder="Address" />
      <D.Group>
        <D.Group.Row label="Setting">
          <D.Segmented options={options} value={value} onChange={setValue} />
        </D.Group.Row>
      </D.Group>
      <D.Reveal open={checked}>Revealed</D.Reveal>
      <D.Reveal open>Visible content</D.Reveal>
      <DesignIdentity />
      <D.Menu trigger={<D.Button>Menu</D.Button>}>
        <D.Menu.Item onSelect={() => {}} checked>
          Selected
        </D.Menu.Item>
        <D.Menu.Separator />
        <D.Menu.Item onSelect={() => {}} danger>
          Delete
        </D.Menu.Item>
      </D.Menu>
      <D.Popover trigger={<D.Button>Picker</D.Button>}>
        <D.GridPicker
          cells={options.map((o) => ({ ...o, content: o.label }))}
          value={value}
          onPick={setValue}
          columns={2}
        />
      </D.Popover>
      <D.GridPicker
        cells={options.map((o) => ({ ...o, content: o.label }))}
        value={value}
        onPick={setValue}
        columns={2}
        leading={<D.Button>Upload</D.Button>}
        trailing={<D.Button>Import</D.Button>}
      />
      <DesignFeedback />
      <DesignInteractions />
      <D.Command text="lankide" copyLabel="Copy" copiedLabel="Copied" />
      <D.Dashes total={5} current={1} />
      <D.Skeleton shape="line" width={100} />
      <D.Skeleton shape="circle" width={40} height={40} />
      <D.Spinner />
      <D.Spinner size="md" />
      <D.EmptyState title="No items" action={<D.Button>Add</D.Button>} />
      <D.Callout tone="quiet">Paused</D.Callout>
      <D.Callout
        tone="attention"
        leading={<D.Icon name="ban" />}
        action={<D.Button>Resume</D.Button>}
      >
        Needs attention
      </D.Callout>
      <D.Message from="me" avatar={avatar}>
        Hello
      </D.Message>
      <D.Message from="them" typing>
        Typing
      </D.Message>
      <D.Message from="them" variant="done" animate={false}>
        Done
      </D.Message>
      <D.QrCode value="https://example.com" size={128} aria-label="QR code" />
    </section>
  );
}
