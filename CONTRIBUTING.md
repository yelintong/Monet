# Contributing

## Branches and pull requests

`main` is protected. Every change lands through a pull request, including changes made by
maintainers:

1. Branch from `main` (`feat/…`, `fix/…`, `ui/…`).
2. Open a pull request and fill in the template.
3. CI must pass (`cargo check --workspace` and the GPUI test suite).
4. AI review (CodeRabbit) comments automatically; address its findings or explain why not.
5. Squash-merge once approved.

## Where a change belongs

The engine (document model, layout, rendering, file format, command catalog) is shared with the
upstream project. The desktop UI in `crates/ui-gpui` is this repository's own work.

- **Engine-level improvement** → propose it upstream; keep the change out of this repository unless
  it is required by a UI feature and agreed first.
- **UI / GPUI work** (panels, canvas interaction, dialogs, input, theming) → this repository.

## The parity rule for UI work

`crates/ui-egui` is the reference implementation. A UI change must match it one-to-one: the same
controls, the same order, the same sections, the same sizes. Do not substitute a simplified
control (a row of chips for a dropdown, a fixed popover for a draggable window, a note for a
section header) and do not hard-code option lists that the engine already provides.

## Checks

```sh
cargo check --workspace
cargo test -p figcraft-ui-gpui --lib --features test-support
```

The test suite has three known failures that predate this repository's UI work; they are listed in
`docs/gpui-egui-parity-plan.md` and are not caused by a change unless the count moves.

## Code and comments

- Comments and identifiers are English.
- Do not reference the project this one grew out of, other products, or their trademarks in code
  comments. Describe what the code does, not where it came from.
- New user-visible strings go through the catalogue (`crates/ui-egui/locales/*.toml`), never inline.
