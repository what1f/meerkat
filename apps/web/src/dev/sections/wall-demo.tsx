import { useState } from 'react';
import * as S from '../../scene';
import * as D from '../../design';
// The team wall: every kind of hanging thing takes the drag handle directly.
const start: S.HoleItem[] = [
  { key: 'rules', kind: 'tag', pos: { x: 8, y: 0 } },
  { key: 'shift', kind: 'sign', pos: { x: 24, y: 1 } },
  { key: 'tasks', kind: 'clipboard', pos: { x: 8, y: 21 } },
  { key: 'vacant', kind: 'clipboard', pos: { x: 24, y: 21 } },
];
export function WallDemo() {
  const [items, setItems] = useState(start);
  const [on, setOn] = useState(true);
  const [done, setDone] = useState(false);
  const handles = S.useHoleDrag(items, (key, pos) =>
    setItems((all) => all.map((i) => (i.key === key ? { ...i, pos } : i))),
  );
  const at = (key: string) => ({ ref: handles[key]?.ref, style: handles[key]?.style });
  return (
    <div style={{ height: 1010 }}>
      <S.Board layout="free">
        <S.Tag {...at('rules')}>
          <h6>RULES</h6>
          <p>Reply in Feishu.</p>
        </S.Tag>
        <S.ShiftSign
          {...at('shift')}
          on={on}
          onToggle={() => setOn(!on)}
          onLabel="上班"
          offLabel="下班"
          label="Shift"
        />
        <S.Clipboard
          {...at('tasks')}
          title="This week"
          actions={<D.IconButton icon="more" label="More" size="sm" />}
          footer={null}
        >
          <S.ChecklistRow
            done={done}
            text="Write the weekly notes"
            onToggle={() => setDone(!done)}
          />
        </S.Clipboard>
        <S.VacantSlot {...at('vacant')} label="Add tasklist" onClick={() => {}} shape="clipboard" />
      </S.Board>
    </div>
  );
}
