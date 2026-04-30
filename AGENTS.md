# Repository Guidelines

## Project Structure & Module Organization
`NormaWM` is a Rust Wayland compositor prototype built on Smithay. Core runtime code lives in `src/`:
- `src/main.rs`: compositor bootstrap, render loop, Wayland socket, input wiring
- `src/wm.rs`: minimal tiling state and `xdg_toplevel` layout management
- `src/ai.rs`: AI command/event channel boundary
- `src/error.rs`: compositor-facing error types
- `src/bin/test_window.rs`: simple Wayland client used for manual testing

Nix development files live at [`flake.nix`](./flake.nix) and [`flake.lock`](./flake.lock). `SYSTEM_PROMPT.md` captures the repository’s AI assistant constraints. Build artifacts belong in `target/` and must not be committed.

## Build, Test, and Development Commands
- `nix develop`: enter the pinned development shell with Rust, Wayland, EGL, and X11 dependencies
- `cargo check`: type-check the compositor
- `cargo check --bins`: verify both `normawm` and the test client
- `cargo run`: launch the nested compositor
- `WAYLAND_DISPLAY=normawm-0 cargo run --bin test_window`: connect the test client to the compositor socket
- `cargo fmt --check`: verify formatting
- `cargo fmt`: apply Rust formatting

## Coding Style & Naming Conventions
Use Rust 2021 idioms and `rustfmt` defaults (4-space indentation, trailing commas where formatter expects them). Prefer small modules with explicit state ownership over shared mutable interior state. Use `snake_case` for functions/modules, `CamelCase` for types, and descriptive enum names such as `AiCommand` or `NormaError`.

## Testing Guidelines
There is no full automated integration suite yet. For now, treat `cargo check --bins` and manual Wayland smoke tests as the baseline. New tests should go in `tests/` for integration coverage or inline `#[cfg(test)]` modules for focused unit logic. Name tests after observable behavior, for example `focus_first_window_updates_activation_state`.

## Commit & Pull Request Guidelines
Current history uses concise, imperative commit messages, e.g. `Initial commit: bootstrap NormaWM`. Follow that pattern: short subject, clear scope, no filler. PRs should include:
- a summary of behavior changes
- validation steps run (`cargo check`, `cargo fmt --check`, manual client launch)
- screenshots or logs for visible compositor behavior changes
- linked issues when applicable

## Architecture Notes
Favor message passing and state machines over `Rc<RefCell<_>>`. When changing Smithay-facing code, keep version-sensitive areas explicit and document assumptions near the change.
