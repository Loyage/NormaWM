//! `normawm` 的库入口。
//!
//! 这里把当前可复用的模块统一导出，便于：
//! 1. 二进制程序 `src/main.rs` / `src/bin/*` 共享逻辑
//! 2. `tests/` 中的集成测试直接复用窗口摘要与 AI 格式化逻辑

pub mod ai;
pub mod atspi;
pub mod compositor;
pub mod control;
pub mod error;
pub mod monitor;
pub mod runtime;
pub mod wm;
