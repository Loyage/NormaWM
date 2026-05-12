# Command Line Interface

读者对象：需要通过 shell、脚本、人工命令或未来 agent 控制 NormaWM 的用户。

本文覆盖范围：`norma msg ...` 查询命令和 `norma ctl ...` 控制命令。

## Command Shape

Development form:

```bash
cargo run --bin norma -- <command>
```

Installed target form:

```bash
norma <command>
```

The CLI connects to:

```text
$XDG_RUNTIME_DIR/normawm-control.sock
```

## Query Commands

Status:

```bash
cargo run --bin norma -- msg status
```

The status output includes compositor state and background monitor fields:

```text
monitor_uptime_ms
monitor_commands_seen
monitor_status_broadcasts
```

Windows:

```bash
cargo run --bin norma -- msg windows
```

Workspaces:

```bash
cargo run --bin norma -- msg workspaces
```

Focused window:

```bash
cargo run --bin norma -- msg focused-window
```

Window accessibility composition:

```bash
cargo run --bin norma -- msg window --window window-1
```

This returns JSON for the selected window. The payload includes top-level WM metadata and either
an AT-SPI2 accessibility tree or, if AT-SPI2 is unavailable, a fallback `surface-tree` view built
from the compositor's own surface state:

```json
{
  "window": {
    "id": "window-1",
    "workspace": 1,
    "title": "Example",
    "app_id": "example",
    "focused": true,
    "human_control": false,
    "visible": true,
    "layout_geometry": { "x": 24, "y": 24, "width": 752, "height": 552 }
  },
  "accessibility": {
    "protocol": "at-spi2",
    "matched_by": "title_exact+window_role",
    "applications_seen": [
      {
        "bus_name": ":1.42",
        "path": "/org/a11y/atspi/accessible/root",
        "name": "Example",
        "role": "application",
        "child_count": 1
      }
    ],
    "node_count": 12,
    "truncated": false,
    "tree": {
      "bus_name": ":1.42",
      "path": "/org/a11y/atspi/accessible/root/window/0",
      "depth": 1,
      "name": "Example",
      "role": "frame",
      "role_debug": "Frame",
      "description": null,
      "child_count": 2,
      "interfaces": ["Accessible", "Component"],
      "attributes": {},
      "component": {
        "screen_extents": { "x": 24, "y": 24, "width": 752, "height": 552 },
        "window_extents": { "x": 0, "y": 0, "width": 752, "height": 552 },
        "alpha": 1.0,
        "layer": "Widget",
        "mdi_z_order": 0
      },
      "children": [],
      "errors": []
    }
  }
}
```

If the window ID does not exist, the command returns an error response. If the client does not
expose a matching AT-SPI2 tree, the command falls back to the compositor's own surface tree.
That fallback is a structural view of the Wayland surfaces, not accessibility metadata.

Runtime notes:

- The host session may have an AT-SPI2 registry available.
- Applications only appear in the AT-SPI2 branch if they expose accessibility metadata.
- Some toolkits may require accessibility to be enabled before they publish useful object trees.
- If accessibility data is missing, the command falls back to the compositor surface tree.

## Control Commands

Focus a window:

```bash
cargo run --bin norma -- ctl focus --window window-1
```

Switch workspace:

```bash
cargo run --bin norma -- ctl workspace 3
```

Launch a program:

```bash
cargo run --bin norma -- ctl launch firefox
```

AI control:

```bash
cargo run --bin norma -- ctl ai pause
cargo run --bin norma -- ctl ai resume
cargo run --bin norma -- ctl ai cancel
```

Shutdown:

```bash
cargo run --bin norma -- ctl shutdown
```

## Input Text

Input into the focused window:

```bash
cargo run --bin norma -- ctl input "你好 NormaWM"
```

Input into a specific window:

```bash
cargo run --bin norma -- ctl input --window window-1 "指定窗口文本"
```

Input from stdin:

```bash
printf "第一行\n第二行\n" | cargo run --bin norma -- ctl input --stdin --window window-1
```

Implementation notes:

- The CLI sends an `INPUT_TEXT` frame to the control socket.
- The compositor focuses the target window, sets a compositor-provided UTF-8 clipboard selection,
  and sends `Ctrl+V`.
- This overwrites the current clipboard.
- The client must support clipboard paste.

## Output Format

Most commands print human-readable text. `norma msg window --window <window-id>` prints JSON from
AT-SPI2 because its output is intended for scripts, debuggers, and future browser-based control
frontends.
