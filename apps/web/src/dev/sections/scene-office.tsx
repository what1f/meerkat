import * as S from '../../scene';
import { Spec, State } from '../Spec';
import { OfficeDemo } from './drag-demos';
const screens = ['ok', 'busy', 'alert', 'asleep', 'dark'] as const;
export function SceneOffice() {
  return (
    <>
      <Spec name="WorkRow, carry members between rows" surface="surface">
        <OfficeDemo />
      </Spec>
      <Spec name="Seat screens" surface="surface">
        {screens.map((screen) => (
          <State key={screen} label={screen}>
            <S.Seat setup="laptop" screen={screen} occupant={{ figure: '01' }} label={screen} />
          </State>
        ))}
        <State label="mini, empty chair, arriving">
          <S.Seat setup="mini" screen="ok" occupant="empty-chair" arriving label="Empty" />
        </State>
      </Spec>
      <Spec name="Figure">
        {(['work', 'sleep', 'carry', 'stand'] as const).map((pose) => (
          <State key={pose} label={pose}>
            <S.Figure id="01" pose={pose} scale={0.3} />
          </State>
        ))}
      </Spec>
      <Spec name="Reaction, Mascot, PullCord">
        <State label="reaction">
          <S.Reaction id="SMILE" size={32} />
        </State>
        <State label="mascot">
          <S.Mascot pose="peek" />
        </State>
        <State label="pull cord">
          <div style={{ position: 'relative', width: 56, height: 96 }}>
            <S.PullCord label="Appearance" onToggle={() => {}} />
          </div>
        </State>
      </Spec>
    </>
  );
}
