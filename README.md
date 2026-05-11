# NormaWM

NormaWM is a Rust Wayland compositor prototype built with Smithay. It currently runs as a nested
compositor through the winit backend, which makes it practical to develop and test inside an
existing Linux desktop session.

The project goal is to build a keyboard-driven tiling window manager with an explicit control plane
for human users and future AI agents. The compositor keeps window state, rendering, command input,
and AI-facing snapshots separated so each layer can evolve without giving external agents direct
mutable access to compositor internals.

## Current Features

- Nested Wayland compositor using Smithay and winit.
- Basic `xdg_toplevel` window management.
- Numbered workspaces from `0` to `9`.
- Automatic workspace assignment for new application windows.
- Keyboard workspace switching.
- Human control panel that is not managed by AI.
- Local Unix socket control API.
- `norma` command line interface for querying and controlling the WM.
- AI-ready command/event boundary through `AiNexus`.
- AI-readable window digest written to `target/ai-input-preview.txt`.
- Unicode text input command using compositor-provided clipboard plus paste.

## Project Layout

- `src/main.rs`: `normawm` binary entrypoint.
- `src/runtime.rs`: compositor startup, event loop, rendering, input routing, control command handling.
- `src/compositor.rs`: Smithay handler implementations and compositor state.
- `src/wm.rs`: tiling/window/workspace state.
- `src/control.rs`: local control protocol and Unix socket server.
- `src/monitor.rs`: standalone human control panel UI.
- `src/ai.rs`: AI command/event and snapshot data model.
- `src/bin/norma.rs`: command line control client.
- `src/bin/normawm-control.rs`: standalone human control panel entrypoint.
- `src/bin/test_window.rs`: simple Wayland test client.

## Building

Use the pinned Nix development shell when possible:

```bash
nix develop
cargo check --bins
```

Useful checks:

```bash
cargo fmt --check
cargo check --bins
```

`Cargo.toml` sets `default-run = "normawm"`, so `cargo run` starts the compositor.

## Running the Compositor

Start NormaWM:

```bash
cargo run
```

The compositor logs the Wayland socket name, for example:

```text
NormaWM nested compositor started. Launch clients with WAYLAND_DISPLAY=normawm-0
```

Run a Wayland client inside NormaWM by pointing `WAYLAND_DISPLAY` at that socket:

```bash
WAYLAND_DISPLAY=normawm-0 cargo run --bin test_window
```

For Firefox:

```bash
MOZ_ENABLE_WAYLAND=1 WAYLAND_DISPLAY=normawm-0 firefox
```

X11-only applications require Xwayland support, which is not implemented yet.

## Workspaces

NormaWM uses numbered workspaces:

- Workspace `0` is reserved for the human control surface.
- Workspaces `1` through `9` are for normal application windows.
- Each new normal window is placed on the next workspace and focused.
- Rendering and keyboard focus only apply to the active workspace.
- AI snapshots exclude the human control surface.

Keyboard shortcuts:

```text
Mod+Alt+j       switch to the next workspace
Mod+Alt+k       switch to the previous workspace
Mod+Alt+0..9    switch directly to a numbered workspace
```

`Mod` means the Super/Meta key. Because NormaWM currently runs nested under another desktop
environment, the host compositor may intercept some Super-key combinations before NormaWM receives
them.

## Human Control Panel

The human control panel is a separate process. It connects to NormaWM through the local control
socket and is intended for manual supervision and override. It is not part of the normal AI-managed
window set.

Start it as a host window:

```bash
cargo run --bin normawm-control
```

Its keyboard controls are:

```text
R      refresh status
F      focus first window
P      pause/resume AI control
C      mark AI tasks cancelled
T      launch test_window inside NormaWM
Q      request compositor shutdown
Esc    close only the control panel
```

The control panel can also be launched as a Wayland client inside NormaWM. If it identifies as
`normawm-control`, NormaWM moves it to workspace `0` and excludes it from AI-visible window digests:

```bash
WAYLAND_DISPLAY=normawm-0 cargo run --bin normawm-control
```

## Command Line Control

NormaWM exposes a niri-style CLI through the `norma` binary. During development, run it with:

```bash
cargo run --bin norma -- <command>
```

When installed as a binary, the intended form is:

```bash
norma <command>
```

### Querying State

Show compositor status:

```bash
cargo run --bin norma -- msg status
```

List windows:

```bash
cargo run --bin norma -- msg windows
```

List workspaces:

```bash
cargo run --bin norma -- msg workspaces
```

Show the focused window ID:

```bash
cargo run --bin norma -- msg focused-window
```

### Controlling Windows And Workspaces

Focus a specific window:

```bash
cargo run --bin norma -- ctl focus --window window-1
```

Switch workspace:

```bash
cargo run --bin norma -- ctl workspace 3
```

Launch an application inside NormaWM:

```bash
cargo run --bin norma -- ctl launch firefox
```

Pause, resume, or cancel AI work:

```bash
cargo run --bin norma -- ctl ai pause
cargo run --bin norma -- ctl ai resume
cargo run --bin norma -- ctl ai cancel
```

Request compositor shutdown:

```bash
cargo run --bin norma -- ctl shutdown
```

### Unicode Text Input

Input text into the currently focused window:

```bash
cargo run --bin norma -- ctl input "你好 NormaWM"
```

Input text into a specific window:

```bash
cargo run --bin norma -- ctl input --window window-1 "指定窗口文本"
```

Input multi-line text from stdin:

```bash
printf "第一行\n第二行\n" | cargo run --bin norma -- ctl input --stdin --window window-1
```

Current implementation details:

- Unicode input is implemented by setting a compositor-provided clipboard selection and sending
  `Ctrl+V` to the target window.
- This supports UTF-8 text, including Chinese.
- The command intentionally overwrites the current clipboard.
- The target application must support Wayland clipboard paste and `Ctrl+V`.

## Local Control Socket

The compositor listens on:

```text
$XDG_RUNTIME_DIR/normawm-control.sock
```

If `XDG_RUNTIME_DIR` is unavailable, the code falls back to the system temporary directory.

The control protocol is intentionally local-only and simple. It is currently designed for local CLI
tools, the human control panel, and future AI adapters. It is not a network API.

## AI Integration Boundary

`src/ai.rs` defines the AI-facing control boundary:

- `AiCommand`: commands that an external agent can request.
- `AiEvent`: events emitted by the compositor.
- `AiNexus`: compositor-side command/event channel.
- `CompositorSnapshot`: lightweight compositor state.
- `AiWindowDigest`: AI-readable window/workspace digest.

The current AI interface is an MVP boundary, not a real model runner. The human control layer can
pause/resume AI control and mark AI tasks cancelled. Once a real AI worker is connected, it should
respect these control states before applying actions.

The latest AI-readable window digest is mirrored to:

```text
target/ai-input-preview.txt
```

## Current Limitations

- NormaWM is currently a nested compositor, not a DRM/KMS session compositor.
- Pointer input is minimal.
- Popup/menu handling is still incomplete.
- Xwayland is not implemented.
- `wl_output`, decorations, dmabuf, and several desktop protocols are incomplete or absent.
- Unicode command input depends on clipboard paste rather than Wayland text-input/input-method
  protocols.
- Workspace assignment is intentionally simple and caps new normal windows at workspace `9`.

## Development Notes

Baseline validation:

```bash
cargo fmt --check
cargo check --bins
```

Manual smoke test:

```bash
cargo run
WAYLAND_DISPLAY=normawm-0 cargo run --bin test_window
cargo run --bin norma -- msg windows
cargo run --bin norma -- ctl input "hello from norma"
```

When changing Smithay-facing code, keep version-sensitive assumptions explicit near the change.
