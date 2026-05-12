# Architecture

读者对象：准备修改 NormaWM 内部代码的开发者。

本文覆盖范围：主要模块边界、数据流和运行时责任分配。

## High-Level Shape

NormaWM is split into a few explicit layers:

- `runtime`: owns startup and the main loop.
- `compositor`: stores Smithay state and implements Wayland handlers.
- `wm`: owns window/workspace/focus layout state.
- `control`: owns local IPC protocol and Unix socket server.
- `ai`: owns AI command/event types and digest data.
- `monitor`: owns the in-process background control monitor.

The main loop in `runtime::run_winit` coordinates these layers:

1. Poll host winit events.
2. Apply compositor hotkeys and input forwarding.
3. Prune dead windows and resize layout.
4. Drain human control commands.
5. Drain AI commands if AI control is not paused.
6. Accept Wayland clients.
7. Dispatch and flush Wayland requests.
8. Render visible windows.
9. Send frame callbacks.

## State Ownership

`NormaApp` is the central Smithay state object. It owns:

- compositor protocol state;
- xdg shell state;
- shm state;
- seat state;
- data device state;
- `AiNexus`;
- `TilingState`.

`TilingState` owns compositor-independent window manager state:

- stable window IDs;
- workspace number;
- focused window index;
- computed geometry;
- human-control marker.

External tools do not hold mutable references into this state. They send commands through the
control socket or future AI channels.

## Control Frontend Boundary

There is no dedicated GUI frontend in the current architecture. The control backend starts with
`normawm`, listens on the local Unix socket, and keeps monitor counters in-process. The user-facing
frontend is the `norma` CLI, which sends `msg` and `ctl` commands over that socket.

## Why This Boundary Matters

Wayland compositor code is sensitive to ordering and ownership. Keeping command input outside the
Smithay handler implementations makes it easier to reason about:

- when windows become managed;
- when configure events are sent;
- when keyboard focus changes;
- when AI/human control can override focus or input.

Note: Smithay 0.7 API compatibility should be checked before changing handler signatures, seat
types, or data device behavior.
