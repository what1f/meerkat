import alpha from '../assets/avatars/07.jpg';
import nova from '../assets/avatars/01.jpg';
import echo from '../assets/avatars/09.jpg';
import pixel from '../assets/avatars/04.jpg';
import atlas from '../assets/avatars/14.jpg';
import sprout from '../assets/avatars/15.jpg';

export type Harness = 'codex' | 'claude-code' | 'pi';
export type RoleKey = 'pm' | 'rd' | 'qa' | 'design';

export type Member = {
  id: string;
  name: string;
  role: RoleKey;
  avatar: ImageMetadata;
  figure: string;
  harness: Harness;
  status: 'ok' | 'busy';
};

// The demo team on the landing page. Names stay Latin in both languages.
export const team: Member[] = [
  { id: 'alpha', name: 'Alpha', role: 'pm', avatar: alpha, figure: '07', harness: 'codex', status: 'ok' },
  { id: 'nova', name: 'Nova', role: 'rd', avatar: nova, figure: '01', harness: 'claude-code', status: 'busy' },
  { id: 'echo', name: 'Echo', role: 'qa', avatar: echo, figure: '11', harness: 'pi', status: 'ok' },
  { id: 'pixel', name: 'Pixel', role: 'design', avatar: pixel, figure: '04', harness: 'codex', status: 'ok' },
];

export const extras = { atlas, sprout };
export const byId = Object.fromEntries(team.map((m) => [m.id, m])) as Record<string, Member>;
