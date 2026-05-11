//! 本地人类控制面 IPC。
//!
//! 这里故意使用很小的换行文本协议，避免在 compositor 主循环里引入
//! async runtime 或复杂序列化依赖。控制面通过 Unix socket 发送命令，
//! compositor 非阻塞轮询并把状态广播回所有连接的控制端。

use std::{
    env, fs,
    io::{self, ErrorKind, Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    process::Command,
};

pub const CONTROL_SOCKET_NAME: &str = "normawm-control.sock";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlCommand {
    RequestStatus,
    RequestWindows,
    RequestWorkspaces,
    RequestFocusedWindow,
    FocusFirstWindow,
    FocusWindow(String),
    SwitchWorkspace(u8),
    InjectText {
        target: Option<String>,
        text: String,
    },
    Launch(Vec<String>),
    PauseAi,
    ResumeAi,
    CancelAiTasks,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlRequest {
    pub client_id: usize,
    pub command: ControlCommand,
}

#[derive(Debug, Clone)]
pub struct ControlStatus {
    pub socket_name: String,
    pub workspace: u8,
    pub managed_windows: usize,
    pub ai_paused: bool,
    pub ai_task_status: String,
    pub windows: Vec<ControlWindowInfo>,
    pub preview: String,
}

#[derive(Debug, Clone)]
pub struct ControlWindowInfo {
    pub id: String,
    pub workspace: u8,
    pub title: Option<String>,
    pub app_id: Option<String>,
    pub focused: bool,
    pub human_control: bool,
}

pub struct ControlServer {
    listener: UnixListener,
    clients: Vec<ControlClient>,
    socket_path: PathBuf,
    next_client_id: usize,
}

struct ControlClient {
    id: usize,
    stream: UnixStream,
    input: Vec<u8>,
    pending: Option<PendingPayload>,
}

struct PendingPayload {
    command: PendingPayloadCommand,
    len: usize,
}

enum PendingPayloadCommand {
    InputText { target: Option<String> },
}

impl ControlServer {
    pub fn bind_default() -> io::Result<Self> {
        let socket_path = control_socket_path();
        if socket_path.exists() {
            fs::remove_file(&socket_path)?;
        }

        let listener = UnixListener::bind(&socket_path)?;
        listener.set_nonblocking(true)?;

        Ok(Self {
            listener,
            clients: Vec::new(),
            socket_path,
            next_client_id: 1,
        })
    }

    pub fn socket_path(&self) -> &PathBuf {
        &self.socket_path
    }

    pub fn poll_commands(&mut self) -> Vec<ControlRequest> {
        self.accept_pending_clients();

        let mut requests = Vec::new();
        let mut dead_clients = Vec::new();

        for (index, client) in self.clients.iter_mut().enumerate() {
            let mut buffer = [0; 1024];

            loop {
                match client.stream.read(&mut buffer) {
                    Ok(0) => {
                        dead_clients.push(index);
                        break;
                    }
                    Ok(bytes) => {
                        client.input.extend_from_slice(&buffer[..bytes]);

                        while let Some(request) = parse_next_client_request(client) {
                            match request {
                                Ok(command) => requests.push(ControlRequest {
                                    client_id: client.id,
                                    command,
                                }),
                                Err(error) => {
                                    let _ = send_result(&mut client.stream, false, &error);
                                }
                            };
                        }
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                    Err(_) => {
                        dead_clients.push(index);
                        break;
                    }
                }
            }
        }

        dead_clients.sort_unstable();
        dead_clients.dedup();
        for index in dead_clients.into_iter().rev() {
            self.clients.remove(index);
        }

        requests
    }

    pub fn broadcast_status(&mut self, status: &ControlStatus) {
        let mut dead_clients = Vec::new();

        for (index, client) in self.clients.iter_mut().enumerate() {
            if send_status(&mut client.stream, status).is_err() {
                dead_clients.push(index);
            }
        }

        for index in dead_clients.into_iter().rev() {
            self.clients.remove(index);
        }
    }

    pub fn send_status_to(&mut self, client_id: usize, status: &ControlStatus) {
        let Some(client) = self
            .clients
            .iter_mut()
            .find(|client| client.id == client_id)
        else {
            return;
        };

        let _ = send_status(&mut client.stream, status);
    }

    pub fn send_text_to(&mut self, client_id: usize, text: &str) {
        let Some(client) = self
            .clients
            .iter_mut()
            .find(|client| client.id == client_id)
        else {
            return;
        };

        let _ = writeln!(client.stream, "BEGIN_TEXT")
            .and_then(|_| writeln!(client.stream, "{text}"))
            .and_then(|_| writeln!(client.stream, "END_TEXT"))
            .and_then(|_| client.stream.flush());
    }

    pub fn broadcast_result(&mut self, ok: bool, message: &str) {
        let mut dead_clients = Vec::new();

        for (index, client) in self.clients.iter_mut().enumerate() {
            if send_result(&mut client.stream, ok, message).is_err() {
                dead_clients.push(index);
            }
        }

        for index in dead_clients.into_iter().rev() {
            self.clients.remove(index);
        }
    }

    pub fn send_result_to(&mut self, client_id: usize, ok: bool, message: &str) {
        let Some(client) = self
            .clients
            .iter_mut()
            .find(|client| client.id == client_id)
        else {
            return;
        };

        let _ = send_result(&mut client.stream, ok, message);
    }

    fn accept_pending_clients(&mut self) {
        loop {
            match self.listener.accept() {
                Ok((stream, _addr)) => {
                    if stream.set_nonblocking(true).is_ok() {
                        let id = self.next_client_id;
                        self.next_client_id += 1;
                        self.clients.push(ControlClient {
                            id,
                            stream,
                            input: Vec::new(),
                            pending: None,
                        });
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket_path);
    }
}

pub fn control_socket_path() -> PathBuf {
    env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join(CONTROL_SOCKET_NAME)
}

pub fn launch_wayland_client(command: &[String], wayland_display: &str) -> io::Result<u32> {
    let Some(program) = command.first() else {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "launch command cannot be empty",
        ));
    };

    let child = Command::new(program)
        .args(&command[1..])
        .env("WAYLAND_DISPLAY", wayland_display)
        .spawn()?;

    Ok(child.id())
}

fn parse_command(line: &str) -> Result<ControlCommand, String> {
    let mut parts = line.split_whitespace();
    let Some(command) = parts.next() else {
        return Err("empty command".to_string());
    };

    match command {
        "STATUS" | "MSG_STATUS" => Ok(ControlCommand::RequestStatus),
        "MSG_WINDOWS" => Ok(ControlCommand::RequestWindows),
        "MSG_WORKSPACES" => Ok(ControlCommand::RequestWorkspaces),
        "MSG_FOCUSED_WINDOW" => Ok(ControlCommand::RequestFocusedWindow),
        "FOCUS_FIRST" => Ok(ControlCommand::FocusFirstWindow),
        "FOCUS_WINDOW" => {
            let Some(window_id) = parts.next() else {
                return Err("FOCUS_WINDOW requires a window id".to_string());
            };
            Ok(ControlCommand::FocusWindow(window_id.to_string()))
        }
        "WORKSPACE" => {
            let Some(workspace) = parts.next() else {
                return Err("WORKSPACE requires a number".to_string());
            };
            let workspace = workspace
                .parse::<u8>()
                .map_err(|_| "WORKSPACE requires a number from 0 to 9".to_string())?;
            if workspace > 9 {
                return Err("WORKSPACE requires a number from 0 to 9".to_string());
            }
            Ok(ControlCommand::SwitchWorkspace(workspace))
        }
        "LAUNCH" => {
            let argv = parts.map(str::to_string).collect::<Vec<_>>();
            if argv.is_empty() {
                Err("LAUNCH requires a program".to_string())
            } else {
                Ok(ControlCommand::Launch(argv))
            }
        }
        "AI_PAUSE" => Ok(ControlCommand::PauseAi),
        "AI_RESUME" => Ok(ControlCommand::ResumeAi),
        "AI_CANCEL" => Ok(ControlCommand::CancelAiTasks),
        "SHUTDOWN" => Ok(ControlCommand::Shutdown),
        _ => Err(format!("unknown control command: {command}")),
    }
}

fn parse_next_client_request(client: &mut ControlClient) -> Option<Result<ControlCommand, String>> {
    if let Some(pending) = client.pending.take() {
        if client.input.len() < pending.len {
            client.pending = Some(pending);
            return None;
        }

        let payload = client.input.drain(..pending.len).collect::<Vec<_>>();
        let text = match String::from_utf8(payload) {
            Ok(text) => text,
            Err(_) => return Some(Err("INPUT_TEXT payload must be valid UTF-8".to_string())),
        };

        if text.is_empty() {
            return Some(Err("INPUT_TEXT payload cannot be empty".to_string()));
        }

        return Some(Ok(match pending.command {
            PendingPayloadCommand::InputText { target } => {
                ControlCommand::InjectText { target, text }
            }
        }));
    }

    let newline = client.input.iter().position(|byte| *byte == b'\n')?;
    let line = client.input.drain(..=newline).collect::<Vec<_>>();
    let line = String::from_utf8_lossy(&line);
    let line = line.trim();

    if line.is_empty() {
        return parse_next_client_request(client);
    }

    if let Some(pending) = parse_payload_header(line) {
        match pending {
            Ok(pending) => {
                client.pending = Some(pending);
                parse_next_client_request(client)
            }
            Err(error) => Some(Err(error)),
        }
    } else {
        Some(parse_command(line))
    }
}

fn parse_payload_header(line: &str) -> Option<Result<PendingPayload, String>> {
    let mut parts = line.split_whitespace();
    let command = parts.next()?;

    if command != "INPUT_TEXT" {
        return None;
    }

    let Some(target) = parts.next() else {
        return Some(Err(
            "INPUT_TEXT requires a target or focused followed by byte length".to_string(),
        ));
    };
    let Some(len) = parts.next() else {
        return Some(Err("INPUT_TEXT requires a byte length".to_string()));
    };
    if parts.next().is_some() {
        return Some(Err(
            "INPUT_TEXT takes exactly two header arguments".to_string()
        ));
    }

    let len = match len.parse::<usize>() {
        Ok(len) => len,
        Err(_) => return Some(Err("INPUT_TEXT byte length must be a number".to_string())),
    };
    if len == 0 {
        return Some(Err("INPUT_TEXT payload cannot be empty".to_string()));
    }

    let target = match target {
        "focused" | "-" => None,
        id => Some(id.to_string()),
    };

    Some(Ok(PendingPayload {
        command: PendingPayloadCommand::InputText { target },
        len,
    }))
}

fn send_status(stream: &mut UnixStream, status: &ControlStatus) -> io::Result<()> {
    writeln!(stream, "BEGIN_STATUS")?;
    writeln!(stream, "socket: {}", status.socket_name)?;
    writeln!(stream, "workspace: {}", status.workspace)?;
    writeln!(stream, "managed_windows: {}", status.managed_windows)?;
    writeln!(stream, "ai_paused: {}", status.ai_paused)?;
    writeln!(stream, "ai_task_status: {}", status.ai_task_status)?;
    writeln!(stream, "windows:")?;
    for window in &status.windows {
        writeln!(
            stream,
            "- id={} workspace={} title={} app_id={} focused={} human_control={}",
            window.id,
            window.workspace,
            window.title.as_deref().unwrap_or("<unset>"),
            window.app_id.as_deref().unwrap_or("<unset>"),
            window.focused,
            window.human_control
        )?;
    }
    writeln!(stream)?;
    writeln!(stream, "{}", status.preview)?;
    writeln!(stream, "END_STATUS")?;
    stream.flush()
}

fn send_result(stream: &mut UnixStream, ok: bool, message: &str) -> io::Result<()> {
    writeln!(
        stream,
        "RESULT {} {}",
        if ok { "ok" } else { "err" },
        message
    )?;
    stream.flush()
}
