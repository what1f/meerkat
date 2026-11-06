# Actual Desktop acceptance

`../window-e2e.sh` validates a recorded human or Machine Use review. It does not
automate UI interaction or infer visual correctness from OCR. No hidden Desktop
flags, injected view state, Mock client, or test-only product composition is allowed.

1. Build the four product executables. Set `MEERKAT_VISUAL_BINARY` and, for release
   acceptance, `MEERKAT_RELEASE_BIN_DIR` and `MEERKAT_RELEASE_REQUIRED=1`.
2. Run `apps/desktop/window-e2e.sh --prepare <new-artifact-directory>`. This creates
   a **pending** template bound to the exact source and binary SHA-256.
3. Launch those binaries in a private home. Record Desktop, Runtime and Machine
   PIDs and `ps -p <pid> -o lstart=` values in `owned_processes`. Use only authorized
   live Provider test accounts; never inject fixture rows into the Runtime database.
4. Operate the real window for every scenario in the template. Save an unaltered
   full-window PNG or JPEG in its original format and the associated
   action/accessibility log, with their SHA-256. Do not relabel JPEG bytes as PNG.
   Review layout, spacing, controls and clipping in the running product. Record
   specific observations, including failures; do not pass a scenario just because
   it opens.
5. Check the title bar, traffic lights, vibrancy, drag region, startup loader,
   localized startup errors and recovery, Open in Browser menu, and external links.
   Review English and Chinese in light and dark modes. Web page business and visual
   acceptance use the browser gates.
6. Quit normally. Verify all owned processes and endpoints disappeared. Only then
   record `cleanup=passed`. Do not kill unrelated user services. If forced cleanup
   was needed, the quit scenario failed; retain that result before repairing it.
7. Fill reviewer identity, per-scenario observations and statuses. Set the top-level
   status to passed only after the complete set passes. Run the gate with
   `MEERKAT_VISUAL_REVIEW=<review.json>` (or `--review`).

A partial review, changed screenshot, missing scenario, changed source/binary or
live owned process fails the gate. This checks evidence integrity; the reviewer is
responsible for the truth of visual observations. A local debug review cannot be
reused for a different release binary.

Tauri shell scenarios (C2):

- `window-chrome`: check traffic lights, title bar, initial size and minimum size.
- `window-vibrancy`: check the native background in light and dark modes.
- `window-drag-region`: drag the window from the reserved sidebar area.
- `startup-loader`: observe the boot page until navigation to the Runtime UI.
- `startup-error-recovery`: check localized failure states and retry/settings actions.
- `open-in-browser`: open the current Runtime URL from the menu.
- `external-link-navigation`: open an external link in the system browser and verify
  that the shell keeps its Runtime page.

For the update cold-start regression, run from the repository root:

```bash
python3 apps/desktop/tests/update_e2e.py --binary-dir target/debug \
  --artifact-dir target/e2e-artifacts/desktop-update-window --window
```

This starts real old helpers in a private home, replaces their executables, and
opens the real Tauri window with a 20-second cold Harness probe. Select only the
unique app path in `session.json` with Machine Use. Record that the loader remains
visible after six seconds and then reaches the discovered management URL without
Retry. Quit that test app normally. Save original screenshots and accessibility
states beside `session.json`; `cleanup.json` records process cleanup only and does
not certify the UI. The normal bootstrap E2E also covers this update with an
eight-second probe, without making a window-acceptance claim.
