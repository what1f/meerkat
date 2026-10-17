import * as D from '../../design';
import { Spec, State } from '../Spec';
const icons: D.IconName[] = [
  'users',
  'chat',
  'monitor',
  'settings',
  'sidebar',
  'search',
  'updown',
  'chev',
  'chevl',
  'smile',
  'refresh',
  'plus',
  'x',
  'check',
  'more',
  'pause',
  'play',
  'trash',
  'pencil',
  'out',
  'copy',
  'shuffle',
  'upload',
  'up',
  'ban',
  'laptop',
  'box',
  'msg',
  'zap',
];
const avatar = new URL('../../assets/avatars/01.jpg', import.meta.url).href;
export function DesignIdentity() {
  return (
    <>
      <Spec name="Icon set">
        {icons.map((name) => (
          <State key={name} label={name}>
            <D.Icon name={name} />
          </State>
        ))}
      </Spec>
      <Spec name="BrandMark">
        {(['feishu', 'telegram', 'codex', 'claude-code', 'pi'] as const).map((brand) => (
          <State key={brand} label={brand}>
            <D.BrandMark brand={brand} />
          </State>
        ))}
      </Spec>
      <Spec name="StatusDot">
        {(['ok', 'busy', 'idle', 'err'] as const).map((status) => (
          <State key={status} label={status}>
            <D.StatusDot status={status} tip={status} />
          </State>
        ))}
      </Spec>
      <Spec name="Avatar sizes">
        {([20, 26, 32, 40, 44, 60, 76, 88] as const).map((size) => (
          <State key={size} label={String(size)}>
            <D.Avatar src={avatar} size={size} status="ok" />
          </State>
        ))}
      </Spec>
      <Spec name="Avatar states">
        <State label="empty dimmed">
          <D.Avatar size={40} dimmed />
        </State>
        <State label="creating">
          <D.Avatar src={avatar} size={40} creating />
        </State>
        <State label="selected editable">
          <D.Avatar src={avatar} size={40} selected onEdit={() => {}} editLabel="Edit" />
        </State>
        <State label="idle">
          <D.Avatar src={avatar} size={40} status="idle" statusTip="Paused" />
        </State>
      </Spec>
    </>
  );
}
