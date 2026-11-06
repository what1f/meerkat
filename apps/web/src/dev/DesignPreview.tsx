import { DesignActions } from './sections/design-actions';
import { DesignFields } from './sections/design-fields';
import { DesignIdentity } from './sections/design-identity';
import { DesignFeedback } from './sections/design-feedback';
import { DesignLayout } from './sections/design-layout';
import { DesignInteractions } from './sections/design-interactions';
import { SceneBoard } from './sections/scene-board';
import { ScenePaper } from './sections/scene-paper';
import { SceneConsole } from './sections/scene-console';
import { SceneOffice } from './sections/scene-office';
import { ShellSection } from './sections/shell';
import { Group, Spec } from './Spec';
import css from './Spec.module.css';
export default function DesignPreview() {
  return (
    <main className={css.page} data-testid="design-preview">
      <h1>Meerkat design</h1>
      <Group name="Design">
        <DesignActions />
        <DesignFields />
        <DesignIdentity />
        <DesignFeedback />
        <DesignLayout />
        <Spec name="Interaction scenarios" surface="surface">
          <DesignInteractions />
        </Spec>
      </Group>
      <Group name="Scene">
        <SceneBoard />
        <ScenePaper />
        <SceneConsole />
        <SceneOffice />
      </Group>
      <Group name="Shell">
        <ShellSection />
      </Group>
    </main>
  );
}
