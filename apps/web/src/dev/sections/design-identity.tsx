import * as D from '../../design';
const iconNames: D.IconName[] = [
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
      {iconNames.map((name) => (
        <D.Icon key={name} name={name} />
      ))}
      {(['feishu', 'telegram', 'codex', 'claude-code', 'pi'] as const).map((brand) => (
        <D.BrandMark key={brand} brand={brand} />
      ))}

      {(['ok', 'busy', 'idle', 'err'] as const).map((status) => (
        <D.StatusDot key={status} status={status} tip={status} />
      ))}
      {([20, 26, 32, 40, 44, 60, 76, 88] as const).map((size) => (
        <D.Avatar key={size} src={avatar} size={size} status="ok" />
      ))}
      <D.Avatar size={40} dimmed />
      <D.Avatar src={avatar} size={40} creating />
      <D.Avatar src={avatar} size={40} selected onEdit={() => {}} editLabel="Edit avatar" />
      <D.Avatar src={avatar} size={40} status="idle" statusTip="Paused" />
    </>
  );
}
