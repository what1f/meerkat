import { DesignSection } from './sections/design';
import { SceneSection } from './sections/scene';
import { ShellSection } from './sections/shell';
export default function DesignPreview() {
  return (
    <div
      data-testid="design-preview"
      style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 24 }}
    >
      {(['light', 'dark'] as const).map((theme) => (
        <div key={theme} data-theme={theme}>
          <h1>{theme}</h1>
          <DesignSection />
          <SceneSection />
          <ShellSection />
        </div>
      ))}
    </div>
  );
}
