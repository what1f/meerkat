import { useState } from 'react';
import * as S from '../../scene';
import * as D from '../../design';
export function SceneSection() {
  const [value, setValue] = useState('Name');
  const [on, setOn] = useState(false);
  const [level, setLevel] = useState(1);
  const badge = (
    <S.Badge
      avatar={{
        src: new URL('../../assets/avatars/01.jpg', import.meta.url).href,
        status: 'ok',
        statusTip: 'Ready',
      }}
      name={value}
      role="Role"
      bio="Notes"
      foot="Model"
      size="card"
    />
  );
  return (
    <section aria-label="Scene">
      <h2>Scene</h2>
      <S.Board layout="rails">
        <S.Hang lanyard="member" swingOnHover>
          {badge}
        </S.Hang>
      </S.Board>
      <S.Board layout="free" holes="fade">
        <S.Hang lanyard="lead" motion="enter">
          {badge}
        </S.Hang>
      </S.Board>
      <S.Hang motion="leave">{badge}</S.Hang>
      <S.Badge avatar={{}} name={value} size="stage" dimmed />
      <S.Badge avatar={{}} name={null} size="card" ghost />
      <S.BadgeVerso
        mark={<D.BrandMark brand="feishu" />}
        actions={[
          { label: 'Sync', onClick: () => {} },
          { label: 'Syncing', onClick: () => {}, busy: true },
        ]}
      />
      <S.FlipCard
        flipped={on}
        front={badge}
        back={<D.Button onClick={() => setOn(!on)}>Flip</D.Button>}
        onFlip={() => setOn(!on)}
      />
      <S.VacantSlot label="Add member" onClick={() => {}} shape="badge" />
      <S.VacantSlot label="Add list" onClick={() => {}} shape="clipboard" />
      <S.Tag>Notes</S.Tag>
      <S.Hang motion="turn-in" swingKey={level}>
        <S.Badge avatar={{}} name={value} size="intro" />
      </S.Hang>
      {(['loading', 'wait', 'scanned', 'expired'] as const).map((state) => (
        <S.QrTag
          key={state}
          state={state}
          value="https://example.com"
          brand="feishu"
          scannerAvatar={new URL('../../assets/avatars/01.jpg', import.meta.url).href}
          onRefresh={() => {}}
          refreshLabel="Refresh"
        />
      ))}
      <S.TokenTag value={value} onChange={setValue} label="Token" />
      <S.TokenTag value={value} onChange={setValue} label="Invalid token" invalid />
      <S.PaperLabel value={value} onCommit={setValue} label="Team" />
      <S.ShiftSign on={on} onToggle={() => setOn(!on)} onLabel="On" offLabel="Off" label="Shift" />
      <S.Clipboard
        title="Tasks"
        actions={<D.Button>Menu</D.Button>}
        footer={<D.Button>Add</D.Button>}
      >
        <S.ChecklistRow done={on} text="Task" onToggle={() => setOn(!on)} />
        <S.ChecklistRow.New placeholder="New task" onAdd={setValue} />
      </S.Clipboard>
      <S.Console
        figure={<S.Figure id="01" pose="stand" />}
        sticker={<S.Sticker reaction="SMILE" label="Reaction" onClick={() => {}} />}
        notice={<D.Callout tone="quiet">Paused</D.Callout>}
        screen={
          <S.ConsoleScreen variant="readout" onClick={() => {}}>
            <S.ScreenReadout
              ai={{ mark: <D.BrandMark brand="codex" />, label: 'Codex' }}
              model="Model"
              place={{ label: 'Office', status: 'ok' }}
              level={{ index: 1, total: 3, label: 'Thinking' }}
            />
          </S.ConsoleScreen>
        }
        body={
          <S.SoulSlot
            value={value}
            onChange={setValue}
            label="Personality"
            placeholder="Notes"
            onFocusChange={setOn}
          />
        }
        deck={<S.ConsoleKey label="Save" icon="check" lit go onClick={() => {}} />}
        drawer={{ label: 'Advanced', content: <D.Button>Options</D.Button> }}
        reading={on}
      />
      <S.ConsoleScreen variant="list">
        {(['run', 'ok', 'fail'] as const).map((test) => (
          <S.HarnessRow
            key={test}
            mark={<D.BrandMark brand="codex" />}
            label={test}
            test={test}
            selected={test === 'ok'}
            reply="Ready"
            onSelect={() => {}}
          />
        ))}
        <S.HarnessRow
          mark={<D.BrandMark brand="pi" />}
          label="Pi"
          action={{ label: 'Install', onClick: () => {} }}
        />
      </S.ConsoleScreen>
      <S.RigFigure id="01" />
      <S.Sticker reaction={null} label="Add reaction" onClick={() => {}} />
      <D.Popover trigger={<S.RigFigure id="01" label="Figure trigger" onClick={() => {}} />}>
        Figure content
      </D.Popover>
      <D.Popover
        trigger={<S.Sticker reaction="SMILE" label="Sticker trigger" onClick={() => {}} />}
      >
        Sticker content
      </D.Popover>
      <D.Popover
        trigger={
          <S.ConsoleScreen variant="readout" onClick={() => {}}>
            Screen trigger
          </S.ConsoleScreen>
        }
      >
        Screen content
      </D.Popover>
      <D.Popover trigger={<S.ConsoleKey label="Key trigger" onClick={() => {}} />}>
        Key content
      </D.Popover>
      <S.ConsoleKey label="Disabled" disabled onClick={() => {}} />
      <S.Lever
        levels={['Low', 'Medium', 'High']}
        value={level}
        onChange={setLevel}
        aria-label="Thinking level"
      />
      {(['work', 'sleep', 'carry', 'stand'] as const).map((pose) => (
        <S.Figure key={pose} id="01" pose={pose} scale={0.2} />
      ))}
      {(['dark', 'asleep', 'alert', 'busy', 'ok'] as const).map((screen) => (
        <S.Seat
          key={screen}
          setup="laptop"
          screen={screen}
          occupant={{ figure: '01' }}
          label={screen}
        />
      ))}
      <S.Seat
        setup="mini"
        screen="ok"
        occupant="empty-chair"
        arriving
        label="Empty seat"
        onClick={() => {}}
      />
      <S.WorkRow title="Office" status="ok" menu={<D.Button>Menu</D.Button>} arriving>
        <S.AddRow label="Add office" onClick={() => {}} />
      </S.WorkRow>
      <S.Reaction id="SMILE" size={24} />
      <S.Mascot pose="peek" />
      <S.PullCord label="Appearance" onToggle={() => setOn(!on)} />
    </section>
  );
}
