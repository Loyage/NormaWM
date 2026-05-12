# Getting Started

读者对象：想把 NormaWM 跑起来并做 smoke test 的开发者。

本文覆盖范围：开发环境、构建、运行 compositor、启动测试窗口和 `norma` 控制命令。

## Development Environment

推荐使用仓库的 Nix shell：

```bash
nix develop
```

基础检查：

```bash
cargo fmt --check
cargo check --bins
```

`Cargo.toml` 设置了：

```toml
default-run = "normawm"
```

所以：

```bash
cargo run
```

等价于启动 `normawm` compositor。

## Start NormaWM

```bash
cargo run
```

启动后日志会打印 Wayland socket，例如：

```text
NormaWM nested compositor started. Launch clients with WAYLAND_DISPLAY=normawm-0
```

后续命令里的 `normawm-0` 需要以实际日志为准。

## Run A Test Client

另开终端：

```bash
WAYLAND_DISPLAY=normawm-0 cargo run --bin test_window
```

如果要运行 Firefox：

```bash
MOZ_ENABLE_WAYLAND=1 WAYLAND_DISPLAY=normawm-0 firefox
```

X11-only applications require Xwayland support, which is not implemented yet.

## Use The CLI

开发期命令形式：

```bash
cargo run --bin norma -- msg status
cargo run --bin norma -- msg windows
cargo run --bin norma -- ctl workspace 1
cargo run --bin norma -- ctl input "hello from norma"
```

安装后的目标形式：

```bash
norma msg status
norma msg windows
norma ctl input "hello from norma"
```

## Manual Smoke Test

```bash
cargo run
WAYLAND_DISPLAY=normawm-0 cargo run --bin test_window
cargo run --bin norma -- msg status
cargo run --bin norma -- msg windows
cargo run --bin norma -- ctl input "hello from norma"
```

## QEMU VM

To run NormaWM inside a reproducible NixOS QEMU VM:

```bash
nix run .#vm
```

See [QEMU VM](./qemu-vm.md) for the full workflow.

## Windows And macOS

NormaWM is a Linux Wayland compositor prototype. Windows and macOS users should run it inside a
Linux environment instead of expecting a native host window manager.

See [Windows And macOS Setup](./non-linux-setup.md) for a from-zero setup path.
