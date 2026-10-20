import { useState } from 'react';
import * as S from '../../scene';
import * as D from '../../design';
// Each preview cell renders its own copy, so every demo owns its state and drag hook.
const avatar = new URL('../../assets/avatars/01.jpg', import.meta.url).href;
const badge = (name: string) => (
  <S.Badge
    avatar={{ src: avatar, status: 'ok', statusTip: 'Ready' }}
    name={name}
    role="Writer"
    bio="Keeps the team's notes tidy and answers in Feishu."
    foot="gpt-6.1"
    size="card"
  />
);
export function RailsDemo() {
  const [order, setOrder] = useState(['Leo', 'Mia', 'Kai']);
  const { itemProps } = S.useSortable(order, setOrder);
  return (
    <S.Board layout="rails">
      {order.map((name) => (
        <S.Hang
          key={name}
          lanyard={name === 'Leo' ? 'lead' : 'member'}
          swingOnHover={name === 'Mia'}
          {...itemProps(name)}
        >
          {badge(name)}
        </S.Hang>
      ))}
      <S.VacantSlot label="Add member" onClick={() => {}} shape="badge" />
    </S.Board>
  );
}
const start: S.HoleItem[] = [
  { key: 'a', kind: 'badge', pos: { x: 4, y: 1 } },
  { key: 'b', kind: 'badge', pos: { x: 14, y: 1 } },
];
export function FreeBoardDemo() {
  const [items, setItems] = useState(start);
  const handles = S.useHoleDrag(items, (key, pos) =>
    setItems((all) => all.map((i) => (i.key === key ? { ...i, pos } : i))),
  );
  return (
    <S.Board layout="free">
      {items.map((item) => (
        <S.Hang key={item.key} ref={handles[item.key]?.ref} style={handles[item.key]?.style}>
          {badge(item.key === 'a' ? 'Ana' : 'Bo')}
        </S.Hang>
      ))}
    </S.Board>
  );
}
export function ClipboardDemo() {
  const [tasks, setTasks] = useState([
    { id: 'a', text: 'Write the weekly notes', done: true },
    { id: 'b', text: 'Reply to the design review', done: false },
    { id: 'c', text: 'Book the meeting room', done: false },
  ]);
  const { itemProps } = S.useSortable(
    tasks.map((t) => t.id),
    (ids) => setTasks(ids.map((id) => tasks.find((t) => t.id === id)!)),
  );
  return (
    <S.Clipboard
      title="This week"
      actions={<D.IconButton icon="more" label="More" size="sm" onClick={() => {}} />}
      footer={null}
    >
      {tasks.map((task) => (
        <div key={task.id} {...itemProps(task.id)}>
          <S.ChecklistRow
            done={task.done}
            text={task.text}
            avatar={task.id === 'b' ? avatar : undefined}
            onToggle={() =>
              setTasks(tasks.map((t) => (t.id === task.id ? { ...t, done: !t.done } : t)))
            }
          />
        </div>
      ))}
      <S.ChecklistRow.New placeholder="New task" onAdd={() => {}} />
    </S.Clipboard>
  );
}
export function OfficeDemo() {
  const [rows, setRows] = useState<Record<string, string[]>>({
    home: ['01', '04'],
    office: ['07'],
  });
  const [titles, setTitles] = useState<Record<string, string>>({
    home: 'MacBook Pro',
    office: 'Office mini',
  });
  const [renaming, setRenaming] = useState<{ row: string; n: number } | null>(null);
  const { seatHandle, rowProps } = S.useCarry((id, to) =>
    setRows((all) =>
      Object.fromEntries(
        Object.entries(all).map(([k, ids]) => [
          k,
          k === to ? [...ids.filter((i) => i !== id), id] : ids.filter((i) => i !== id),
        ]),
      ),
    ),
  );
  return (
    <div style={{ width: '100%' }}>
      {Object.entries(rows).map(([key, ids]) => (
        <S.WorkRow
          key={key}
          {...rowProps(key)}
          title={
            <D.InlineText
              key={renaming?.row === key ? renaming.n : 0}
              value={titles[key] ?? ''}
              placeholder="Computer"
              textStyle="title"
              autoFocus={renaming?.row === key}
              onCommit={(v) => setTitles({ ...titles, [key]: v })}
            />
          }
          status={key === 'home' ? 'ok' : 'idle'}
          menu={
            <D.Menu
              align="end"
              trigger={<D.IconButton icon="more" label="More" size="sm" onClick={() => {}} />}
            >
              <D.Menu.Item
                returnFocus={false}
                onSelect={() => setRenaming({ row: key, n: (renaming?.n ?? 0) + 1 })}
              >
                Rename
              </D.Menu.Item>
            </D.Menu>
          }
        >
          {ids.map((id) => (
            <S.Seat
              key={id}
              setup={key === 'home' ? 'laptop' : 'mini'}
              screen={key === 'home' ? 'ok' : 'asleep'}
              occupant={{ figure: id }}
              label={`Member ${id}`}
              carry={seatHandle(id)}
              onClick={() => {}}
            />
          ))}
        </S.WorkRow>
      ))}
      <S.AddRow label="Add computer" onClick={() => {}} />
    </div>
  );
}
