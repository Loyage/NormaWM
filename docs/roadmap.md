# Roadmap

读者对象：想了解 NormaWM 后续方向的开发者和用户。

本文覆盖范围：当前未完成能力和优先级方向。它不是承诺清单。

## Desktop Protocols

- `wl_output`
- server-side or client-side decorations
- `linux-dmabuf`
- viewport/scaling support
- popup/menu correctness
- primary selection and richer clipboard behavior

## Input

- pointer motion, click, focus, and cursor image handling;
- interactive move/resize;
- touch support;
- text-input/input-method protocols for Unicode input beyond clipboard paste.

## Window Management

- per-workspace focus history;
- moving windows between workspaces;
- tree-based tiling layout;
- floating windows;
- fullscreen and maximize semantics;
- configurable keybindings.

## Control Plane

- structured output mode for `norma`, likely `--json`;
- protocol versioning;
- richer error taxonomy;
- peer credential checks for local socket clients;
- command source labels: human, CLI, AI, script.

## AI

- external AI runner process;
- task lifecycle and cancellation;
- richer window and input context;
- safe action approval policies;
- replayable action logs.

## System Backend

NormaWM currently uses a nested winit backend. Long-term work may include DRM/KMS backend support,
but that should come after protocol, input, and window state are more mature.

The current QEMU VM support validates the nested backend inside a guest graphical session. A future
DRM/KMS VM should test NormaWM as the real compositor on a virtual GPU and tty.
