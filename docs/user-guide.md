# User Guide

读者对象：想实际使用当前 NormaWM prototype 的用户和开发者。

本文覆盖范围：workspace、快捷键、普通应用、后台控制监控和 CLI。CLI 细节见
[Command Line Interface](./cli.md)。

## Workspaces

NormaWM uses numbered workspaces from `0` to `9`.

- Workspace `0` is reserved for future human-control surfaces.
- Workspaces `1` through `9` are for normal application windows.
- New normal windows are assigned to the next workspace and focused.
- Rendering and keyboard focus apply only to the active workspace.
- AI-visible window digests exclude windows identified as human-control surfaces.

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

## Background Control Monitor

NormaWM starts the control socket and monitor when the compositor starts. There is no separate
control-panel frontend to launch. Use `norma` from a terminal to inspect and control the WM:

```bash
cargo run --bin norma -- msg status
cargo run --bin norma -- msg windows
cargo run --bin norma -- ctl workspace 1
cargo run --bin norma -- ctl ai pause
cargo run --bin norma -- ctl input "hello from norma"
```

`norma msg status` includes monitor fields such as uptime, observed command count, and status
broadcast count. These counters are maintained by the in-process background monitor.
