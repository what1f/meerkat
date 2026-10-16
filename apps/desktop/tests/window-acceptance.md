# Actual Desktop acceptance

`../visual-e2e.sh` validates a recorded human or Machine Use review. It does not
automate UI interaction or infer visual correctness from OCR. No hidden Desktop
flags, injected view state, Mock client, or test-only product composition is allowed.

1. Build the four product executables. Set `LANKIDE_VISUAL_BINARY` and, for release
   acceptance, `LANKIDE_RELEASE_BIN_DIR` and `LANKIDE_RELEASE_REQUIRED=1`.
2. Run `apps/desktop/visual-e2e.sh --prepare <new-artifact-directory>`. This creates
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
5. For `real-profile-images`, compare the actual Provider avatar/cover with the
   rendered resource, then verify that missing data stays empty. For onboarding,
   verify the visible next action and recovery. For model selection, check the
   native directory and explicit custom configuration. Review light and dark modes.
6. Quit normally. Verify all owned processes and endpoints disappeared. Only then
   record `cleanup=passed`. Do not kill unrelated user services. If forced cleanup
   was needed, the quit scenario failed; retain that result before repairing it.
7. Fill reviewer identity, per-scenario observations and statuses. Set the top-level
   status to passed only after the complete set passes. Run the gate with
   `LANKIDE_VISUAL_REVIEW=<review.json>` (or `--review`).

A partial review, changed screenshot, missing scenario, changed source/binary or
live owned process fails the gate. This checks evidence integrity; the reviewer is
responsible for the truth of visual observations. A local debug review cannot be
reused for a different release binary.

Task 14 scenarios:

- `first-launch-local-machine`: launch the product Desktop in a fresh private
  home. In Settings, the local Machine appears online without a pairing dialog.
- `machine-list-offline`: pair a second Machine through the product daemon,
  connect it, then stop it. In Settings and the Agent create/edit selector, both
  Machines remain visible with the second offline. Revoked Machines are absent.
  Create an Agent on the chosen Machine and save an edited placement; verify it
  through management. A new form defaults to the local Machine.
- `pairing-code-dialog`: Add Machine first shows one address input and Enable.
  After enabling remote access, it shows one selectable command and Copy command.
  Run the copied command with a second private Machine home; verify the connected
  name, automatic close, and updated list. Check command renewal after expiry.
  Review Settings, address, command, and connected states in English and Chinese,
  in both themes, including wrapping, contrast, spacing, and button alignment.
