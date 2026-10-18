import * as H from '../../shell';
import { Menu } from '../../design';
import { Spec, State } from '../Spec';
const team = (
  <H.TeamSwitcher
    teams={[
      { id: 'one', name: 'Lankide' },
      { id: 'two', name: 'Studio' },
    ]}
    current="one"
    onSelect={() => {}}
    onCreate={() => {}}
    createLabel="New team"
  />
);
const nav = (
  <>
    <H.NavItem icon="users" label="Members" active onClick={() => {}} testId="nav-members" />
    <H.NavItem icon="monitor" label="Computers" active={false} onClick={() => {}} testId="nav-pc" />
    <H.NavItem icon="chat" label="Chats" active={false} onClick={() => {}} testId="nav-chat" />
  </>
);
const settings = (
  <H.NavItem icon="settings" label="Settings" active={false} onClick={() => {}} testId="nav-set" />
);
export function ShellSection() {
  return (
    <>
      <Spec name="AppShell" frame>
        <H.AppShell
          sidebar={<H.Sidebar header={team} nav={nav} footer={settings} />}
          topBar={<H.TopBar title="Members" />}
          board="wall"
        >
          {null}
        </H.AppShell>
      </Spec>
      <Spec name="TopBar" surface="surface">
        <State label="title">
          <H.TopBar title="Members" />
        </State>
        <State label="back">
          <H.TopBar back={{ label: 'Settings', onClick: () => {} }} />
        </State>
        <State label="back menu">
          <H.TopBar
            back={{
              label: 'Lankide',
              menu: (
                <>
                  <Menu.Item onSelect={() => {}}>Save</Menu.Item>
                  <Menu.Item onSelect={() => {}}>Discard</Menu.Item>
                </>
              ),
            }}
          />
        </State>
      </Spec>
      <Spec name="NavItem and TeamSwitcher">
        <State label="team">{team}</State>
        <State label="nav">{nav}</State>
      </Spec>
    </>
  );
}
