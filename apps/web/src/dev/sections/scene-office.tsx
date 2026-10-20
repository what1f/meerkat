import * as S from '../../scene';
import { Spec, State } from '../Spec';
import { OfficeDemo } from './drag-demos';
const screens = ['ok', 'busy', 'alert', 'asleep', 'dark'] as const;
const poses = ['work', 'sleep', 'carry', 'stand'] as const;
// A figure pack arrives as frame URLs; here the built-in 04 frames stand in for one.
const frame = (pose: S.Pose) =>
  new URL(`../../assets/figures/04/${pose}.png`, import.meta.url).href;
const pack: S.FigurePack = {
  frames: {
    work: frame('work'),
    sleep: frame('sleep'),
    carry: frame('carry'),
    stand: frame('stand'),
  },
  scale: 1,
};
const standOnly: S.FigurePack = { frames: { stand: frame('stand') }, scale: 1 };
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
        {poses.map((pose) => (
          <State key={pose} label={pose}>
            <S.Figure id="01" pose={pose} scale={0.3} />
          </State>
        ))}
        {poses.map((pose) => (
          <State key={pose} label={`pack ${pose}`}>
            <S.Figure id="pack" pose={pose} scale={0.3} pack={pack} />
          </State>
        ))}
        {['ana', 'kai', 'mia'].map((seed) => (
          <State key={seed} label={`default, seed ${seed}`}>
            <S.Figure id="default" pose="work" scale={0.3} seed={seed} />
          </State>
        ))}
        <State label="pack without work">
          <S.Figure id="pack" pose="work" scale={0.3} pack={standOnly} seed="ana" />
        </State>
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
