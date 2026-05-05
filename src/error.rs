//! `NormaWM` 的错误分层定义。
//!
//! 当前错误类型主要覆盖启动、渲染和 Wayland client 管理三个阶段。
//! 设计目标不是追求最细粒度，而是先把“错误发生在哪一层”说清楚，
//! 方便在 CLI 和日志里快速定位问题。

use thiserror::Error;

/// compositor 主流程中的领域错误。
///
/// 每个变体都对应一个较清晰的失败阶段，方便后续再细分成更结构化的子错误。
#[derive(Debug, Error)]
pub enum NormaError {
    #[error("failed to create wayland display: {0}")]
    DisplayInit(String),
    #[error("failed to bind wayland socket under XDG_RUNTIME_DIR: {0}")]
    SocketBind(String),
    #[error("failed to create winit backend: {0}")]
    WinitBackend(String),
    #[error("failed to create keyboard seat: {0}")]
    KeyboardInit(String),
    #[error("failed to bind renderer to the winit surface: {0}")]
    RendererBind(String),
    #[error("render pass failed: {0}")]
    Render(String),
    #[error("failed to submit rendered frame to the backend: {0}")]
    Submit(String),
    #[error("failed to accept a wayland client: {0}")]
    AcceptClient(String),
    #[error("failed to register a wayland client: {0}")]
    InsertClient(String),
    #[error("wayland dispatch failed: {0}")]
    WaylandDispatch(String),
    #[error("wayland flush failed: {0}")]
    WaylandFlush(String),
}
