import { useState } from 'react';
import * as S from '../../scene';
import * as D from '../../design';
import { Spec, State } from '../Spec';
import { ClipboardDemo } from './drag-demos';
const avatar = new URL('../../assets/avatars/01.jpg', import.meta.url).href;
export function ScenePaper() {
  const [text, setText] = useState('Meerkat');
  const [token, setToken] = useState('');
  const [on, setOn] = useState(true);
  return (
    <>
      <Spec name="Tag">
        <S.Tag>
          <h6>RULES</h6>
          <D.DocArea value={text} onChange={setText} placeholder="Rules" />
        </S.Tag>
      </Spec>
      <Spec name="QrTag">
        {(['loading', 'wait', 'scanned', 'expired', 'error'] as const).map((state) => (
          <State key={state} label={state}>
            <S.QrTag
              state={state}
              value="https://example.com"
              brand="feishu"
              scannerAvatar={avatar}
              onRefresh={() => {}}
              errorLabel={state}
              refreshLabel="Refresh"
            />
          </State>
        ))}
      </Spec>
      <Spec name="TokenTag">
        <State label="empty">
          <S.TokenTag value={token} onChange={setToken} label="Bot token" />
        </State>
        <State label="invalid">
          <S.TokenTag value="123:abc" onChange={() => {}} label="Bot token" invalid />
        </State>
      </Spec>
      <Spec name="PaperLabel and ShiftSign" surface="board">
        <S.Board layout="rails">
          <S.PaperLabel value={text} onCommit={setText} label="Team name" />
          <S.ShiftSign
            on={on}
            onToggle={() => setOn(!on)}
            onLabel="上班"
            offLabel="下班"
            label="Shift"
          />
        </S.Board>
      </Spec>
      <Spec name="Clipboard, drag rows to sort">
        <ClipboardDemo />
      </Spec>
    </>
  );
}
