# Unicode Text Input

读者对象：需要理解 `norma ctl input` 如何工作的用户和开发者。

本文覆盖范围：当前 Unicode 输入实现、限制和未来替代路径。

## User Commands

Focused window:

```bash
cargo run --bin norma -- ctl input "你好 NormaWM"
```

Specific window:

```bash
cargo run --bin norma -- ctl input --window window-1 "指定窗口文本"
```

Stdin:

```bash
printf "第一行\n第二行\n" | cargo run --bin norma -- ctl input --stdin --window window-1
```

## Implementation

The CLI sends:

```text
INPUT_TEXT <target-or-focused> <byte-len>\n
<utf8-bytes>
```

The compositor then:

1. Resolves the target window or focused window.
2. Switches to the target window workspace.
3. Sets keyboard focus to the target surface.
4. Sets data-device focus to the target client.
5. Publishes a compositor-provided clipboard selection containing the UTF-8 text.
6. Sends `Ctrl+V` to the target window.

## Why Clipboard Paste

Direct keycode injection is not enough for Chinese or arbitrary Unicode text, because keycodes depend
on keyboard layout and do not express composed text. Clipboard paste is a practical first step that
works with many Wayland clients.

## Limitations

- The current clipboard is overwritten.
- The target client must support clipboard paste.
- The target client must treat `Ctrl+V` as paste.
- This is not a replacement for Wayland text-input/input-method protocols.

Future versions should consider text-input/input-method support for richer IME semantics.

Note: Smithay 0.7 API compatibility should be checked before changing data-device selection code.
