//! `NormaWM` 二进制入口。
//!
//! 这里只保留真正的进程入口；复杂的 compositor 状态与主循环
//! 分别拆到 `compositor.rs` 和 `runtime.rs`。

use normawm::{ai::AiNexus, error::NormaError, runtime};

/// 进程入口：初始化日志和 AI 通道，然后进入 winit backend 主循环。
fn main() -> Result<(), NormaError> {
    runtime::init_tracing();

    let (ai_nexus, _ai_handle) = AiNexus::channel();
    runtime::run_winit(ai_nexus)
}
