//! AI 接入层的最小数据模型。
//!
//! 当前这个模块不直接运行模型，也不包含网络调用。
//! 它只负责三件事：
//! 1. 定义 compositor 与 AI 之间交换的命令/事件类型
//! 2. 定义“窗口摘要”这种 AI 可消费的中间数据
//! 3. 提供一个最小 channel 边界，避免 AI 直接持有 compositor 内部可变状态

#![allow(dead_code)]

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

/// AI 发往 compositor 的输入命令。
///
/// 这些命令目前仍然偏 MVP，只覆盖快照请求、焦点控制和简单外观修改。
#[derive(Debug, Clone)]
pub enum AiCommand {
    RequestSnapshot,
    SetClearColor([f32; 4]),
    FocusFirstWindow,
    Shutdown,
}

/// 单个窗口在 AI 视角下的摘要记录。
///
/// 这里保留的是 AI 判断布局和语义最常需要的信息：
/// 标识、角色、标题、app_id、焦点和几何。
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

/// 某一时刻整个工作区的窗口摘要。
///
/// 它是发给 AI 前的“中间表示”，可以继续被序列化成文本、JSON，
/// 或者在未来扩展成更复杂的 prompt 上下文。
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

/// compositor 在本地执行一个动作后的结果消息。
#[derive(Debug, Clone)]
pub struct ActionResult {
    pub ok: bool,
    pub message: String,
}

impl ActionResult {
    /// 构造一个成功结果，统一成功消息的表达方式。
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: message.into(),
        }
    }

    /// 构造一个失败结果，便于把动作失败原因继续往上层抛出或显示。
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

/// AI 接入边界的 compositor 侧端点。
///
/// 它只持有：
/// - 来自 AI 的命令接收端
/// - 发往 AI 的事件发送端
///
/// 这样设计的好处是主循环只需要“收命令 + 发事件”，而不需要共享大块可变状态。
pub struct AiNexus {
    command_rx: Receiver<AiCommand>,
    event_tx: Sender<AiEvent>,
}

/// AI 接入边界的外部控制端点。
///
/// 未来如果把外部 agent 进程接进来，它通常会持有这个句柄，而不是 `AiNexus` 本体。
pub struct AiNexusHandle {
    command_tx: Sender<AiCommand>,
    event_rx: Receiver<AiEvent>,
}

impl AiNexus {
    /// 创建一对双向 channel，分别给 compositor 侧和外部控制侧使用。
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

    /// 非阻塞地取出当前所有待处理命令。
    ///
    /// 这里不用阻塞式 `recv()`，是为了让 compositor 主循环自己掌握节奏，
    /// 不会因为外部 AI 暂时没有输入而卡住渲染与 Wayland dispatch。
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

    /// 向外部 AI 观察端发送一个事件。
    ///
    /// 这里对发送失败采取“静默忽略”策略，因为对端可能尚未连接或已经退出，
    /// 但这不应该影响 compositor 主循环继续工作。
    pub fn emit(&self, event: AiEvent) {
        let _ = self.event_tx.send(event);
    }
}

impl AiNexusHandle {
    /// 从外部向 compositor 发送一个命令。
    pub fn send(&self, command: AiCommand) -> Result<(), mpsc::SendError<AiCommand>> {
        self.command_tx.send(command)
    }

    /// 非阻塞读取 compositor 发回的事件。
    pub fn try_recv(&self) -> Result<AiEvent, TryRecvError> {
        self.event_rx.try_recv()
    }
}
