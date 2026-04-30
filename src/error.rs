use thiserror::Error;

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
