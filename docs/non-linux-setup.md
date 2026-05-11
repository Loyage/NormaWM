# Windows And macOS Setup

读者对象：使用 Windows 或 macOS，并希望从零开始运行 NormaWM 的开发者。

本文覆盖范围：安装路线、推荐环境、首次运行、常见限制。NormaWM 是 Linux Wayland
compositor 原型，不能作为 Windows 或 macOS 的原生窗口管理器运行。

## Recommended Path

推荐路径是在 Windows 或 macOS 上先准备一个 Linux 开发环境，然后在 Linux 内运行
NormaWM。

原因：

- NormaWM 面向 Wayland compositor 开发，核心运行环境是 Linux 图形栈。
- 当前项目使用 nested winit backend，适合在一个已有 Linux 桌面会话中启动。
- 仓库提供的 NixOS QEMU VM 目标目前是 `x86_64-linux`，不是 Windows 或 macOS 原生目标。

## Windows From Zero

Windows 用户有两条路线。

### Route A: Linux VM

这是最接近真实 Linux 图形环境的路线。

1. 在 BIOS/UEFI 中启用 CPU virtualization。
2. 安装一个虚拟机工具，例如 Hyper-V、VMware、VirtualBox 或 QEMU 前端。
3. 安装一个 Linux 桌面发行版，例如 NixOS、Fedora、Ubuntu 或 Arch Linux。
4. 在 Linux VM 中安装基础工具：

```bash
git
curl
rustup
nix
```

5. clone 仓库：

```bash
git clone <repo-url> NormaWM
cd NormaWM
```

6. 如果 Linux VM 中安装了 Nix，使用仓库环境：

```bash
nix develop
cargo check --bins
cargo run
```

7. 另开一个 Linux VM 终端，启动测试窗口：

```bash
WAYLAND_DISPLAY=normawm-0 cargo run --bin test_window
```

`normawm-0` 需要替换为 compositor 日志中打印的实际 socket 名。

### Route B: WSL2

WSL2 适合做构建、格式化和一部分开发检查，但它不是首选的 compositor 图形运行环境。

1. 安装 WSL2：

```powershell
wsl --install -d Ubuntu
```

2. 进入 WSL2 后安装 Nix 或 Rust 工具链。
3. clone 仓库并运行：

```bash
git clone <repo-url> NormaWM
cd NormaWM
nix develop
cargo check --bins
```

如果宿主系统启用了 WSLg，部分 GUI 程序可以显示，但 QEMU、OpenGL、Wayland nested
compositor 的行为会受 Windows、GPU driver 和 WSLg 版本影响。遇到图形问题时，优先切换到
Route A 的完整 Linux VM。

## macOS From Zero

macOS 用户推荐使用 Linux VM。

1. 安装一个虚拟机工具，例如 UTM、VMware Fusion、Parallels 或 QEMU。
2. 创建一个 Linux 桌面 VM。建议分配：

```text
CPU: 2 cores or more
Memory: 4 GiB or more
Disk: 20 GiB or more
Graphics: 3D acceleration enabled when available
```

3. 在 Linux VM 中安装基础工具：

```bash
git
curl
rustup
nix
```

4. clone 仓库并运行：

```bash
git clone <repo-url> NormaWM
cd NormaWM
nix develop
cargo check --bins
cargo run
```

5. 另开终端运行测试窗口：

```bash
WAYLAND_DISPLAY=normawm-0 cargo run --bin test_window
```

macOS 原生 Nix 可以用于阅读代码或做一部分跨平台求值，但当前 NormaWM runtime 和
`.#vm` app 都面向 Linux。实际运行 compositor 时请使用 Linux VM。

## Running The Project VM

如果你已经在 Linux 环境中，包括 Windows/macOS 上的 Linux VM，可以直接使用仓库提供的
NixOS QEMU VM：

```bash
nix run .#vm
```

这会启动一个包含 NormaWM 工具链的 NixOS 图形虚拟机。进入 VM 后：

```bash
normawm
WAYLAND_DISPLAY=normawm-0 test_window
norma msg windows
norma ctl input "hello from qemu"
```

完整说明见 [QEMU VM](./qemu-vm.md)。

## What Works Where

| Host environment | Build/check | Run nested NormaWM | Run `nix run .#vm` |
| --- | --- | --- | --- |
| Linux desktop | Yes | Yes | Yes on `x86_64-linux` |
| Windows + Linux VM | Yes inside VM | Yes inside VM | Yes inside `x86_64-linux` VM |
| Windows + WSL2 | Usually | Depends on WSLg/graphics | Not recommended |
| macOS + Linux VM | Yes inside VM | Yes inside VM | Yes inside `x86_64-linux` VM |
| Native macOS | Limited | No | No |

## Troubleshooting

- If `cargo run` starts but clients cannot connect, check the printed Wayland socket and use that
  exact value in `WAYLAND_DISPLAY`.
- If a GUI window is blank inside a VM, enable 3D acceleration or try a different virtual GPU.
- If `nix run .#vm` is slow on Windows/macOS, run it inside a Linux VM with hardware virtualization
  enabled.
- If an X11-only app does not start inside NormaWM, that is expected; Xwayland support is not
  implemented yet.
