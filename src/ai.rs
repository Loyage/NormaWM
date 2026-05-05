#![allow(dead_code)]

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

#[derive(Debug, Clone)]
pub enum AiCommand {
    RequestSnapshot,
    SetClearColor([f32; 4]),
    FocusFirstWindow,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiWindowRecord {
    pub window_id: String,
    pub role: &'static str,
    pub title: Option<String>,
    pub app_id: Option<String>,
    pub focused: bool,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiWindowDigest {
    pub workspace: String,
    pub output_width: i32,
    pub output_height: i32,
    pub window_count: usize,
    pub windows: Vec<AiWindowRecord>,
}

/// 将窗口摘要整理成一段可直接展示、记录或发送给 AI 的纯文本输入。
///
/// 这里故意使用稳定、扁平的行文本格式，而不是更复杂的结构化协议，
/// 方便在测试中直接断言内容，也方便后续先把它接到 prompt 再逐步升级。
pub fn format_ai_window_digest(digest: &AiWindowDigest) -> String {
    let mut lines = vec![
        "NormaWM window digest for AI".to_string(),
        format!("workspace: {}", digest.workspace),
        format!("output: {}x{}", digest.output_width, digest.output_height),
        format!("window_count: {}", digest.window_count),
        "windows:".to_string(),
    ];

    if digest.windows.is_empty() {
        lines.push("- none".to_string());
    } else {
        for window in &digest.windows {
            lines.push(format!(
                "- id={} role={} title={} app_id={} focused={} geometry=({}, {}) {}x{}",
                window.window_id,
                window.role,
                window.title.as_deref().unwrap_or("<unset>"),
                window.app_id.as_deref().unwrap_or("<unset>"),
                window.focused,
                window.x,
                window.y,
                window.width,
                window.height
            ));
        }
    }

    lines.join("\n")
}

#[derive(Debug, Clone)]
pub struct CompositorSnapshot {
    pub backend: &'static str,
    pub socket_name: String,
    pub workspace: &'static str,
    pub toplevel_count: usize,
    pub clear_color: [f32; 4],
}

#[derive(Debug, Clone)]
pub struct ActionResult {
    pub ok: bool,
    pub message: String,
}

impl ActionResult {
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: message.into(),
        }
    }

    pub fn err(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum AiEvent {
    Snapshot(CompositorSnapshot),
    ActionResult(ActionResult),
    PromptPreview(String),
}

pub struct AiNexus {
    command_rx: Receiver<AiCommand>,
    event_tx: Sender<AiEvent>,
}

pub struct AiNexusHandle {
    command_tx: Sender<AiCommand>,
    event_rx: Receiver<AiEvent>,
}

impl AiNexus {
    pub fn channel() -> (Self, AiNexusHandle) {
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();

        (
            Self {
                command_rx,
                event_tx,
            },
            AiNexusHandle {
                command_tx,
                event_rx,
            },
        )
    }

    pub fn drain_commands(&self) -> Vec<AiCommand> {
        let mut commands = Vec::new();

        loop {
            match self.command_rx.try_recv() {
                Ok(command) => commands.push(command),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }

        commands
    }

    pub fn emit(&self, event: AiEvent) {
        let _ = self.event_tx.send(event);
    }
}

impl AiNexusHandle {
    pub fn send(&self, command: AiCommand) -> Result<(), mpsc::SendError<AiCommand>> {
        self.command_tx.send(command)
    }

    pub fn try_recv(&self) -> Result<AiEvent, TryRecvError> {
        self.event_rx.try_recv()
    }
}
