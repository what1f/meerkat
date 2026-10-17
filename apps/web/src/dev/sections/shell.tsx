import * as H from '../../shell';
import { Button, Menu } from '../../design';
export function ShellSection() {
  return (
    <section aria-label="Shell">
      <h2>Shell</h2>
      <H.AppShell
        sidebar={
          <H.Sidebar
            header={
              <H.TeamSwitcher
                teams={[{ id: 'one', name: 'Team' }]}
                current="one"
                onSelect={() => {}}
                onCreate={() => {}}
                createLabel="Create team"
              />
            }
            nav={
              <H.NavItem
                icon="users"
                label="Members"
                active
                onClick={() => {}}
                testId="preview-nav"
              />
            }
            footer={<Button>Settings</Button>}
          />
        }
        topBar={<H.TopBar title="Members" back={{ label: 'Back', onClick: () => {} }} />}
        board
      >
        {null}
      </H.AppShell>
      <H.TopBar
        back={{
          label: 'Back menu',
          menu: (
            <>
              <Menu.Item onSelect={() => {}}>Save</Menu.Item>
              <Menu.Item onSelect={() => {}}>Discard</Menu.Item>
            </>
          ),
        }}
      />
      <H.SidebarGrip />
      <H.NavItem
        icon="settings"
        label="Settings"
        active={false}
        onClick={() => {}}
        testId="preview-settings"
      />
    </section>
  );
}
