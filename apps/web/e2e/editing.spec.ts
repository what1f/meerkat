import { expect } from '@playwright/test';
import { test, rig } from './rig';

test('editing keeps its draft and contrast when window focus changes', async ({ page }) => {
  const workspace = 'editing_focus';
  await rig({ op: 'create_workspace', workspace });
  await rig({ op: 'create_agent', workspace, agent: 'ada' });
  await page.addInitScript((ws) => {
    localStorage.setItem('meerkat.workspace', JSON.stringify(ws));
  }, workspace);
  await page.goto('/#/member/ada');
  const soul = page.getByRole('textbox', { name: /性格|Personality/ });
  await soul.fill('Keep this unsaved draft.');
  const card = soul.locator('xpath=ancestor::label');
  await expect(card).toHaveCSS('opacity', '1');

  // WebKit can lose CSS focus without delivering an element blur when the
  // native window becomes inactive. Reproduce that mismatch independently
  // from the browser runner's own window activation behavior.
  await soul.evaluate((input) => {
    const stop = (event: Event) => event.stopImmediatePropagation();
    document.addEventListener('focusout', stop, true);
    input.blur();
    document.removeEventListener('focusout', stop, true);
    window.dispatchEvent(new Event('blur'));
  });
  await expect(card).toHaveCSS('opacity', '1');
  await soul.focus();
  await expect(card).toHaveCSS('opacity', '1');
  await expect(soul).toHaveValue('Keep this unsaved draft.');
  await soul.press('End');
  await soul.pressSequentially(' More.');
  await expect(soul).toHaveValue('Keep this unsaved draft. More.');

  // A real field blur still ends the reading effect.
  const brain = page.getByTestId('brain-picker');
  await brain.focus();
  await expect(brain).toHaveCSS('opacity', '1');
  await expect(page.getByTestId('member-save')).toBeEnabled();

  for (const field of await page.locator('input:not([type=checkbox]), textarea').all()) {
    await expect(field).toHaveAttribute('autocorrect', 'off');
    await expect(field).toHaveAttribute('autocapitalize', 'off');
    await expect(field).toHaveAttribute('spellcheck', 'false');
  }
  for (const target of [soul, page.locator('body')]) {
    expect(
      await target.evaluate((element) => {
        const event = new MouseEvent('contextmenu', { bubbles: true, cancelable: true });
        element.dispatchEvent(event);
        return event.defaultPrevented;
      }),
    ).toBe(true);
  }
});
