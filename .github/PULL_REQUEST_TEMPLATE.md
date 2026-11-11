## What changed

<!-- One or two sentences. Link the issue this closes, if any. -->

## Where it belongs

- [ ] This is **UI / GPUI** work (panels, canvas interaction, dialogs, input, theming) → this repo
- [ ] This is **engine** work (document model, layout, rendering, format, commands) → upstream project

## Checks

- [ ] `cargo check --workspace` passes
- [ ] `cargo test -p figcraft-ui-gpui --lib --features test-support` passes
- [ ] UI work: the change matches the reference implementation in `crates/ui-egui` (no simplified
      substitutes for menus, popovers, drag or list rows)
- [ ] No new strings without a catalogue entry
- [ ] Screenshots or a short recording for visual changes
