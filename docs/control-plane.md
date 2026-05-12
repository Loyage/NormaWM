# Control Plane

读者对象：需要理解 human control、CLI、AI control 如何共存的开发者。

本文覆盖范围：Unix socket control API、后台监控、request/response 模式、人类控制优先级。

## Control Socket

NormaWM listens on:

```text
$XDG_RUNTIME_DIR/normawm-control.sock
```

If `XDG_RUNTIME_DIR` is unavailable, it falls back to the system temporary directory.

The socket is local-only. It is not a network API.

## Clients

Current clients:

- `norma`: CLI for command-line control.
- future AI adapters or automation tools.

## Command Flow

`ControlServer` accepts Unix socket clients and parses two command shapes:

- line commands such as `MSG_WINDOWS`, `WORKSPACE 3`, `AI_PAUSE`;
- payload commands such as `INPUT_TEXT focused <byte-len>` followed by UTF-8 bytes.

The runtime handles each `ControlRequest` and replies to the originating client. Long-lived clients
can also receive broadcast status updates. The in-process background monitor starts with the WM and
adds monitor counters to `MSG_STATUS` responses.

## Human Control Priority

Human control is above AI control:

- human commands are processed before AI commands in the main loop;
- `AI_PAUSE` makes AI commands return an error instead of mutating compositor state;
- `AI_CANCEL` currently records cancellation state for future AI workers.

This preserves a manual override path even after a real AI runner is added.

## Protocol Stability

The protocol is intentionally simple and not yet versioned. Before exposing it outside the local
session, add:

- explicit protocol version;
- structured output mode;
- authentication or peer credential checks;
- clearer error taxonomy.
