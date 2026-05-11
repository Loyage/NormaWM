# Development Guide

读者对象：准备修改代码、提交 PR、维护文档的开发者。

本文覆盖范围：常用命令、代码边界、文档维护和提交前检查。

## Baseline Checks

```bash
cargo fmt --check
cargo check --bins
```

When modifying docs:

```bash
rg -n "\\]\\(" README.md docs
```

Inside the Nix development shell, mdBook is available for documentation builds:

```bash
mdbook build
```

The generated `book/` directory is build output and should not be committed.

## Code Style

- Rust 2021.
- Use `rustfmt` defaults.
- Prefer small modules with explicit state ownership.
- Prefer message passing over shared mutable state.
- Keep Smithay-facing assumptions close to the code that depends on them.

## Module Boundaries

- Runtime loop changes belong in `src/runtime.rs`.
- Smithay handler changes belong in `src/compositor.rs`.
- Window/workspace policy belongs in `src/wm.rs`.
- Local socket protocol belongs in `src/control.rs`.
- AI command/event data belongs in `src/ai.rs`.

## Documentation Rules

- README is the entry point, not the whole manual.
- Long-form docs live in `docs/`.
- Keep user-facing commands in `docs/cli.md`.
- Keep architecture decisions in `docs/architecture.md` or specialized design pages.
- Update docs in the same change when behavior changes.

## Commit Guidance

Use concise imperative commit messages, for example:

```text
Add human control workspace
Document NormaWM features
```
