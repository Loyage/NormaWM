# User Guide

读者对象：想实际使用当前 NormaWM prototype 的用户和开发者。

本文覆盖范围：workspace、快捷键、普通应用、人类控制面。CLI 细节见
[Command Line Interface](./cli.md)。

## Workspaces

NormaWM uses numbered workspaces from `0` to `9`.

- Workspace `0` is reserved for the human control surface.
- Workspaces `1` through `9` are for normal application windows.
- New normal windows are assigned to the next workspace and focused.
- Rendering and keyboard focus apply only to the active workspace.
- AI-visible window digests exclude the human control surface.

## Keyboard Shortcuts

```text
Mod+Alt+j       switch to the next workspace
Mod+Alt+k       switch to the previous workspace
Mod+Alt+0..9    switch directly to a numbered workspace
```

`Mod` means Super/Meta.

Because NormaWM currently runs nested under another compositor, the host desktop may intercept some
Super-key combinations before NormaWM receives them.

## Launching Applications

Run Wayland clients by setting `WAYLAND_DISPLAY`:

```bash
WAYLAND_DISPLAY=normawm-0 foot
MOZ_ENABLE_WAYLAND=1 WAYLAND_DISPLAY=normawm-0 firefox
```

Or launch through the control plane:

```bash
cargo run --bin norma -- ctl launch firefox
```

The control-plane launch path injects the current NormaWM Wayland socket automatically.

## Human Control Panel

The human control panel is a separate process. It is not AI-managed and is intended for manual
supervision.

```bash
cargo run --bin normawm-control
```

Panel shortcuts:

```text
R      refresh status
F      focus first window
P      pause/resume AI control
C      mark AI tasks cancelled
T      launch test_window inside NormaWM
Q      request compositor shutdown
Esc    close only the control panel
```

When launched as a Wayland client into NormaWM, it is moved to workspace `0` and excluded from AI
digests if identified as `normawm-control`.
