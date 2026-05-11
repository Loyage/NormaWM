# QEMU VM

读者对象：想在虚拟机中运行和测试 NormaWM 的开发者。

本文覆盖范围：NixOS QEMU VM 的构建、启动、登录和 smoke test。本文不覆盖 DRM/KMS
裸 tty 后端实现。

## Current VM Model

NormaWM currently uses a nested winit backend. The QEMU setup therefore runs a small NixOS graphical
desktop inside the VM, then starts NormaWM inside that desktop session.

This means:

- QEMU provides the virtual machine.
- NixOS provides the guest OS and graphical session.
- Xfce/LightDM provides the host GUI session inside the guest.
- NormaWM runs nested inside that guest GUI session.

This is intentionally different from running NormaWM directly as a DRM/KMS compositor on a tty.
Direct DRM/KMS backend support is future work.

## Build And Run

Start the VM from the repository root:

```bash
nix run .#vm
```

Equivalent explicit build target:

```bash
nix build .#nixosConfigurations.normawm-vm.config.system.build.vm
```

The VM user is:

```text
user: norma
password: norma
```

The VM is configured to auto-login to the graphical session when possible.

## Smoke Test Inside The VM

Open a terminal in the VM and start NormaWM:

```bash
normawm
```

The compositor prints a Wayland socket name, usually similar to:

```text
normawm-0
```

Open another terminal inside the VM and run:

```bash
WAYLAND_DISPLAY=normawm-0 test_window
```

Query state:

```bash
norma msg windows
```

Input text:

```bash
norma ctl input "hello from qemu"
```

Start the human control panel:

```bash
normawm-control
```

## Installed Tools

The VM includes:

- `normawm`
- `normawm-control`
- `norma`
- `test_window`
- `xfce4-terminal`
- `xterm`
- `wayland-info`
- `eglinfo`
- `glxinfo`

## Limitations

- This VM validates the current nested compositor path.
- It does not validate a DRM/KMS compositor backend.
- Host graphics acceleration and QEMU display behavior can vary by machine.
- If `gtk,gl=on` display fails on the host, the VM configuration may need a different QEMU display
  option.

## Future Work

Longer term VM support should include:

- a DRM/KMS backend test VM;
- boot-to-NormaWM session option;
- automated smoke tests for `norma msg` and `norma ctl`;
- screenshot or protocol-level validation for rendered windows.
