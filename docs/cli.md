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

The first version prints human-readable text. There is no `--json` mode yet.
