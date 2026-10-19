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
    settingsLabel="Settings"
    onSettings={() => {}}
  />
);
const dock = (
  <H.Dock
    team={team}
    items={[
      { id: 'members', icon: 'users', label: 'Members', active: true, onClick: () => {} },
      { id: 'team', icon: 'chat', label: 'Team', active: false, onClick: () => {} },
      { id: 'office', icon: 'monitor', label: 'Office', active: false, onClick: () => {} },
    ]}
  />
);
export function ShellSection() {
  return (
    <>
      <Spec name="AppShell" frame>
        <H.AppShell dock={dock} topBar={<H.TopBar title="Members" />} board="wall">
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
      <Spec name="Dock" frame>
        <H.AppShell dock={dock} topBar={<H.TopBar />}>
          {null}
        </H.AppShell>
      </Spec>
    </>
  );
}
