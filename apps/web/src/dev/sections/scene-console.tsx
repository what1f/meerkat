import { useState } from 'react';
import * as S from '../../scene';
import * as D from '../../design';
import { Spec, State } from '../Spec';
const levels = ['Low', 'Medium', 'High'];
const long =
  'You keep the team calm. Answer in short sentences, ask before deleting anything, and write notes at the end of every day so the next person can pick up.';
export function SceneConsole() {
  const [soul, setSoul] = useState('Patient and precise.');
  const [reading, setReading] = useState(false);
  const [level, setLevel] = useState(1);
  const [picked, setPicked] = useState('codex');
  return (
    <>
      <Spec name="Console" surface="board">
        <S.Board layout="rails">
          <S.Console
            figure={<S.RigFigure id="01" label="Change look" onClick={() => {}} />}
            sticker={<S.Sticker reaction="SMILE" label="Reaction" onClick={() => {}} />}
            screen={
              <S.ConsoleScreen variant="readout" onClick={() => {}}>
                <S.ScreenReadout
                  ai={{ mark: <D.BrandMark brand="codex" />, label: 'Codex' }}
                  model="gpt-6.1-sol"
                  place={{ label: 'MacBook Pro', status: 'ok' }}
                  level={{ index: level, total: 3, label: levels[level] ?? '' }}
                />
              </S.ConsoleScreen>
            }
            body={
              <S.SoulSlot
                value={soul}
                onChange={setSoul}
                label="PERSONALITY"
                placeholder="Who is this teammate?"
                onFocusChange={setReading}
              />
            }
            deck={
              <>
                <S.ConsoleKey icon="pause" label="Pause" onClick={() => {}} />
                <S.ConsoleKey icon="trash" label="Delete" onClick={() => {}} />
                <S.ConsoleKey label="Save" go onClick={() => {}} />
              </>
            }
            drawer={{
              label: 'Advanced',
              content: (
                <S.Lever levels={['Low', 'Medium', 'High']} value={level} onChange={setLevel} />
              ),
            }}
            reading={reading}
          />
        </S.Board>
      </Spec>
      <Spec name="ConsoleKey">
        <State label="text">
          <S.ConsoleKey label="Save" onClick={() => {}} />
        </State>
        <State label="icon">
          <S.ConsoleKey icon="pause" label="Pause" onClick={() => {}} />
        </State>
        <State label="lit">
          <S.ConsoleKey icon="zap" label="Wake" lit onClick={() => {}} />
        </State>
        <State label="go">
          <S.ConsoleKey label="Save" go onClick={() => {}} />
        </State>
        <State label="go disabled">
          <S.ConsoleKey label="Save" go disabled onClick={() => {}} />
        </State>
      </Spec>
      <Spec name="SoulSlot long text">
        <S.SoulSlot
          value={long}
          onChange={() => {}}
          label="PERSONALITY"
          placeholder=""
          onFocusChange={() => {}}
        />
      </Spec>
      <Spec name="Lever">
        <S.Lever levels={['Low', 'Medium', 'High', 'Max']} value={level} onChange={setLevel} />
      </Spec>
      <Spec name="ConsoleScreen list and HarnessRow">
        <S.ConsoleScreen variant="list">
          {(
            [
              ['codex', 'Codex', 'ok', 'Ready'],
              ['claude-code', 'Claude Code', 'fail', 'Signed out'],
              ['pi', 'Pi', 'run', ''],
            ] as const
          ).map(([brand, label, test, reply]) => (
            <S.HarnessRow
              key={brand}
              mark={<D.BrandMark brand={brand} />}
              label={label}
              test={picked === brand ? test : undefined}
              reply={reply}
              selected={picked === brand}
              onSelect={() => setPicked(brand)}
            />
          ))}
          <S.HarnessRow
            mark={<D.BrandMark brand="pi" />}
            label="Pi"
            action={{ label: 'Install', onClick: () => {} }}
          />
        </S.ConsoleScreen>
      </Spec>
      <Spec name="Sticker and RigFigure">
        <State label="reaction">
          <S.Sticker reaction="SMILE" label="Reaction" onClick={() => {}} />
        </State>
        <State label="none">
          <S.Sticker reaction={null} label="Add reaction" onClick={() => {}} />
        </State>
        <State label="figure">
          <div style={{ position: 'relative', width: 60, height: 90, marginTop: 86 }}>
            <S.RigFigure id="01" label="Change look" onClick={() => {}} />
          </div>
        </State>
      </Spec>
    </>
  );
}
