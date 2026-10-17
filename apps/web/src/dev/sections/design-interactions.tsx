import { useState } from 'react';
import * as D from '../../design';
const allCells = ['Alpha', 'Beta', 'Gamma'].map((value) => ({
  value,
  content: value,
  label: value,
}));
export function DesignInteractions() {
  const [cells, setCells] = useState(allCells);
  const [selected, setSelected] = useState('Beta');
  const [value, setValue] = useState('Commit text');
  const [live, setLive] = useState('Live text');
  const [events, setEvents] = useState('');
  const [prevent, setPrevent] = useState(false);
  const record = (event: string) => setEvents((current) => `${current} ${event}`);
  return (
    <section aria-label="Interaction scenarios">
      <D.Button onClick={() => setCells(allCells.slice(0, 1))}>Shrink grid</D.Button>
      <D.Button onClick={() => setCells([])}>Clear grid</D.Button>
      <D.Button onClick={() => setCells(allCells)}>Refill grid</D.Button>
      <D.Button onClick={() => setCells([...allCells].reverse())}>Reverse grid</D.Button>
      <D.GridPicker
        data-testid="dynamic-grid"
        cells={cells}
        value={selected}
        onPick={setSelected}
        columns={2}
      />
      <D.Button
        onPointerDown={(e) => e.preventDefault()}
        onClick={() => setPrevent(!prevent)}
        aria-pressed={prevent}
      >
        Prevent events
      </D.Button>
      <D.InlineText
        data-testid="commit-events"
        value={value}
        onCommit={setValue}
        placeholder="Commit"
        textStyle="body"
        onClick={(e) => {
          record('click');
          if (prevent) e.preventDefault();
        }}
        onBlur={(e) => {
          record('blur');
          if (prevent) e.preventDefault();
        }}
        onKeyDown={(e) => {
          record(e.key);
          if (prevent) e.preventDefault();
        }}
      />
      <D.InlineText
        data-testid="live-events"
        mode="live"
        value={live}
        onChange={setLive}
        placeholder="Live"
        textStyle="body"
        onClick={() => record('live-click')}
        onBlur={() => record('live-blur')}
        onKeyDown={(e) => record(`live-${e.key}`)}
      />
      <output data-testid="inline-events">{events}</output>
    </section>
  );
}
