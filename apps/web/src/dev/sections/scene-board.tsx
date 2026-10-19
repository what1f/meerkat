import { useState } from 'react';
import * as S from '../../scene';
import * as D from '../../design';
import { Spec, State } from '../Spec';
import { FreeBoardDemo, RailsDemo } from './drag-demos';
import { WallDemo } from './wall-demo';
const avatar = new URL('../../assets/avatars/01.jpg', import.meta.url).href;
const badge = (name: string, size: 'card' | 'stage' | 'intro' = 'card') => (
  <S.Badge
    avatar={{ src: avatar, status: 'ok', statusTip: 'Ready' }}
    name={name}
    role="Writer"
    bio="Keeps the team's notes tidy and answers in Feishu."
    foot="gpt-6.1"
    size={size}
  />
);
export function SceneBoard() {
  const [replay, setReplay] = useState(0);
  const [reading, setReading] = useState(false);
  const [flipped, setFlipped] = useState(false);
  return (
    <>
      <Spec name="Board rails, drag to reorder" surface="board">
        <RailsDemo />
      </Spec>
      <Spec name="Board free, drag between holes" surface="board">
        <FreeBoardDemo />
      </Spec>
      <Spec name="Team wall, drag anything to another hole" surface="board">
        <WallDemo />
      </Spec>
      <Spec name="Hang motion" surface="board">
        <S.Board layout="rails" holes="fade">
          <D.Button onClick={() => setReplay(replay + 1)}>Replay</D.Button>
          {(['enter', 'leave', 'turn-in'] as const).map((motion) => (
            <S.Hang key={`${motion}${replay}`} motion={motion} swingKey={replay}>
              {badge(motion)}
            </S.Hang>
          ))}
        </S.Board>
      </Spec>
      <Spec name="Hang, open on click, faded while reading" surface="board">
        <S.Board layout="rails">
          <S.Hang onClick={() => setReading(!reading)}>{badge('Open me')}</S.Hang>
          <S.Hang faded={reading}>{badge('Stage')}</S.Hang>
        </S.Board>
      </Spec>
      <Spec name="Badge">
        <State label="stage">{badge('Leo', 'stage')}</State>
        <State label="intro">{badge('Leo', 'intro')}</State>
        <State label="dimmed">
          <S.Badge avatar={{ src: avatar }} name="Paused" size="card" dimmed />
        </State>
        <State label="ghost">
          <S.Badge avatar={{}} name={null} size="card" ghost />
        </State>
      </Spec>
      <Spec name="FlipCard / BadgeVerso" surface="board">
        <S.Hang lanyard="lead">
          <S.FlipCard
            flipped={flipped}
            onFlip={() => setFlipped(!flipped)}
            front={badge('Alice')}
            back={
              <S.BadgeVerso
                mark={<D.BrandMark brand="feishu" />}
                actions={[
                  { label: 'Sync profile', busy: true, onClick: () => {} },
                  { label: 'Edit role', onClick: () => {} },
                ]}
              />
            }
          />
        </S.Hang>
      </Spec>
      <Spec name="VacantSlot">
        {(['badge', 'clipboard'] as const).map((shape) => (
          <State key={shape} label={shape}>
            <S.VacantSlot label="Add" onClick={() => {}} shape={shape} />
          </State>
        ))}
      </Spec>
    </>
  );
}
