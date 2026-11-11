<p align="center">
  <img alt="Monet" src="docs/brand/monet.svg" width="96" height="96">
</p>

<h1 align="center">Monet</h1>

<p align="center">
  <b>A native interface-design editor: one Rust engine, a GPU-rendered desktop UI.</b><br>
  Frames, auto layout, components and variants, vector networks, styles, variables and prototypes,<br>
  in an application that opens instantly and works offline.
</p>

<p align="center">
  <img alt="100% Rust" src="https://img.shields.io/badge/100%25-Rust-b7410e?style=flat-square&logo=rust">
  <img alt="UI: GPUI" src="https://img.shields.io/badge/UI-GPUI-0EA5C6?style=flat-square">
  <img alt="License: MIT OR Apache-2.0" src="https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-3a3a3a?style=flat-square">
  <img alt="Status: early alpha" src="https://img.shields.io/badge/status-early%20alpha-d69e2e?style=flat-square">
</p>

---

## What this is

Monet is a desktop design tool. The lower half is a document engine: a scene graph, layout, vector
networks, text, components, variables, prototypes and a renderer. The upper half is a desktop
front-end that draws and drives it.

The engine's original front-end was written with **egui**, the immediate-mode toolkit: every frame
rebuilt the whole interface from scratch. This repository ports that front-end to **GPUI**, the
retained-mode, GPU-composited framework that powers Zed — a declarative element tree with
persistent state, taffy layout, and panels, popovers and the canvas composited as one scene on the
GPU.

The port is one for one. Every panel, dialog, popover, menu and keyboard path is rebuilt to match
the egui front-end's layout, dimensions and behaviour, and
[`docs/gpui-egui-parity-plan.md`](docs/gpui-egui-parity-plan.md) tracks what still differs. The
egui front-end stays available locally as the reference the port is checked against.

```
crates/
  doc/ engine/ layout/ render/ format/ geom/ color/ text/ svg/ pathops/ …   the engine
  ui-gpui/        the GPUI front-end this repository develops
apps/
  figcraft-cli/   command-line front-end
  figcraft-server/
```

## Highlights

- **GPU-composited canvas.** Pan, zoom and selection stay smooth on large documents; tiles are
  rasterised off the UI thread and streamed in, with a low-resolution overview first.
- **The whole editor is one scene.** Panels, popovers and the canvas share one render tree, so
  overlapping menus, drag previews and the motion timeline composite in the right order.
- **A real document engine.** Auto layout, component instances and overrides, styles, variables and
  modes, boolean operations, vector networks and prototyping all live in the engine, not the UI.
- **Immediate, offline, local-first.** No account, no server, no network round trip to open a file.
- **Motion.** Keyframes, easing and a timeline that drives the same scene the canvas draws.
- **Localised.** The UI follows the system language; every string comes from the catalogue.
- **Accessible by construction.** Focus order, keyboard navigation and screen-reader roles are part
  of the widget layer rather than an afterthought.

## Building

Requires a recent stable Rust toolchain. On Windows the MSVC toolchain is expected.

```sh
cargo run -p figcraft-ui-gpui --example run
```

Tests:

```sh
cargo test -p figcraft-ui-gpui --lib --features test-support
```

## Contributing

Every change lands through a pull request; `main` is protected and CI must pass. See
[`CONTRIBUTING.md`](CONTRIBUTING.md) for the review flow, the parity rule for UI work, and the
checks that run on every PR.

## License

MIT OR Apache-2.0.
