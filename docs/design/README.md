# Design

- [design-system.md](design-system.md): visual language, surfaces, tokens,
  components, board objects, figures, and copy rules.
- [onboarding.md](onboarding.md): first-run flow, user story, screen-by-screen
  decisions, and implementation plan.
- `onboarding/`: approved onboarding mockups (generated concept art, not
  screenshots).
- Avatars, figures, desk art, reactions, and brand marks live in
  `apps/web/src/assets/`; `system.html` loads them from there.
- `system.html`: interactive prototype of the design system and every page.
- `current.html`: the shipped UI as it is today, for comparison with
  `system.html`.

Mockups show intent. When a mockup and this text disagree, the text wins.
