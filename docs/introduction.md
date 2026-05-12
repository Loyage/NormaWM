# Introduction

读者对象：第一次了解 NormaWM 的开发者、未来贡献者、以及需要判断项目方向的人。

本文覆盖范围：项目愿景、当前阶段、主要能力和边界。具体命令请看
[Getting Started](./getting-started.md) 和 [Command Line Interface](./cli.md)。

## What NormaWM Is

NormaWM is a Rust Wayland compositor prototype built on Smithay. It currently runs as a nested
compositor through the winit backend, which makes it possible to develop and debug it inside an
existing Linux desktop session.

NormaWM 的长期目标不是只做一个能打开窗口的 compositor，而是构建一个对 AI agent
友好的 window manager：

- compositor 持有真实 Wayland/window/input/render 状态；
- human control plane 可以随时监督、暂停、覆盖 AI 行为；
- AI 只能通过明确的 command/event 边界读写状态；
- 窗口、workspace、输入注入等行为都能被命令行和未来 agent 稳定寻址。

## Current Stage

当前项目仍是 prototype/MVP 阶段。已经存在的核心能力：

- nested Wayland compositor；
- basic `xdg_toplevel` management；
- numbered workspaces；
- local Unix socket control API；
- `norma` CLI；
- background control monitor started with the compositor；
- AI-ready snapshot/digest boundary；
- Unicode text input through clipboard paste.

还没有完成的桌面能力包括 pointer 完整交互、Xwayland、`wl_output`、dmabuf、decorations、
完整 popup/menu 处理、真实 AI runner 等。

## Design Principles

- Prefer explicit state machines over shared mutable state.
- Prefer message passing and narrow command/event boundaries.
- Keep human control above AI control.
- Keep Smithay version-sensitive code visible and local.
- Treat documentation as architecture, not as an afterthought.

Note: Smithay 0.7 API compatibility should be checked before changing compositor-facing protocol code.
