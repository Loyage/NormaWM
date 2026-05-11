# NormaWM

NormaWM is a Rust Wayland compositor prototype built with Smithay. It currently runs as a nested
compositor through the winit backend, which makes it practical to develop and test inside an
existing Linux desktop session.

The long-term goal is to build a keyboard-driven tiling window manager with a human-first control
plane and an AI-ready command/event boundary.

## Highlights

- Nested Wayland compositor using Smithay and winit.
- Basic `xdg_toplevel` window management.
- Numbered workspaces from `0` to `9`.
- Standalone human control panel.
- Local Unix socket control API.
- `norma` CLI for querying and controlling the WM.
- AI-readable window digest and command/event boundary.
- Unicode text input through compositor-provided clipboard paste.

## Documentation

Full documentation lives in [`docs/SUMMARY.md`](./docs/SUMMARY.md). The docs directory is also
compatible with mdBook through [`book.toml`](./book.toml).

Important entry points:

- [Getting Started](./docs/getting-started.md)
- [Windows And macOS Setup](./docs/non-linux-setup.md)
- [QEMU VM](./docs/qemu-vm.md)
- [User Guide](./docs/user-guide.md)
- [Command Line Interface](./docs/cli.md)
- [Architecture](./docs/architecture.md)
- [Control Plane](./docs/control-plane.md)
- [AI Integration](./docs/ai-integration.md)

## Quick Start

Use the pinned development shell when possible:

```bash
nix develop
cargo check --bins
```

Start NormaWM:

```bash
cargo run
```

Run a test client inside the compositor. Replace `normawm-0` with the socket printed by the
compositor log:

```bash
WAYLAND_DISPLAY=normawm-0 cargo run --bin test_window
```

Query windows:

```bash
cargo run --bin norma -- msg windows
```

Input text into the focused window:

```bash
cargo run --bin norma -- ctl input "hello from norma"
```

## Validation

```bash
cargo fmt --check
cargo check --bins
```

Inside the Nix development shell, build the documentation with:

```bash
mdbook build
```
