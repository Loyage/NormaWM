# Window Management

读者对象：准备修改 workspace、focus、tiling 行为的开发者。

本文覆盖范围：窗口 ID、workspace 分配、focus 和 layout 当前策略。

## Managed Windows

Every managed `xdg_toplevel` becomes a `ManagedToplevel` inside `TilingState`.

Each managed window has:

- stable ID, such as `window-1`;
- `ToplevelSurface`;
- computed geometry;
- workspace number;
- `human_control` marker.

Stable IDs are used by:

- `norma msg windows`;
- `norma ctl focus --window ...`;
- `norma ctl input --window ...`;
- AI window digests.

## Workspace Policy

Workspace `0` is reserved for human control. Normal windows start at workspace `1`.

When a normal window is created:

1. It receives the next stable window ID.
2. It is assigned to the next workspace.
3. The compositor switches to that workspace.
4. The new window becomes focused.

The current policy caps automatic assignment at workspace `9`.

## Focus Policy

`TilingState` stores focused window as an index into the window vector. Public methods translate
control operations into `WlSurface` values that runtime can pass to Smithay keyboard focus.

Focus changes usually call:

- layout refresh;
- xdg toplevel configure;
- keyboard focus sync in runtime.

## Layout Policy

The current layout is intentionally minimal:

- only windows on the active workspace are rendered;
- visible windows are arranged vertically;
- outer and inner gaps are fixed constants;
- each visible window receives an xdg configure with size and activated state.

This is a foundation for future tree-based tiling, splits, floating windows, and per-workspace focus
history.
