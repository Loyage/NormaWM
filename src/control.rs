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
    FocusFirstWindow,
    Launch(Vec<String>),
    PauseAi,
    ResumeAi,
    CancelAiTasks,
    Shutdown,
}

#[derive(Debug, Clone)]
pub struct ControlStatus {
    pub socket_name: String,
    pub workspace: u8,
    pub managed_windows: usize,
    pub ai_paused: bool,
    pub ai_task_status: String,
    pub preview: String,
}

pub struct ControlServer {
    listener: UnixListener,
    clients: Vec<ControlClient>,
    socket_path: PathBuf,
}

struct ControlClient {
    stream: UnixStream,
    input: String,
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
        })
    }

    pub fn socket_path(&self) -> &PathBuf {
        &self.socket_path
    }

    pub fn poll_commands(&mut self) -> Vec<ControlCommand> {
        self.accept_pending_clients();

        let mut commands = Vec::new();
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
                        let chunk = String::from_utf8_lossy(&buffer[..bytes]);
                        client.input.push_str(&chunk);

                        while let Some(newline) = client.input.find('\n') {
                            let line = client.input[..newline].trim().to_string();
                            client.input.drain(..=newline);

                            if line.is_empty() {
                                continue;
                            }

                            match parse_command(&line) {
                                Ok(command) => commands.push(command),
                                Err(error) => {
                                    let _ = send_result(&mut client.stream, false, &error);
                                }
                            }
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

        commands
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

    fn accept_pending_clients(&mut self) {
        loop {
            match self.listener.accept() {
                Ok((stream, _addr)) => {
                    if stream.set_nonblocking(true).is_ok() {
                        self.clients.push(ControlClient {
                            stream,
                            input: String::new(),
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
        "STATUS" => Ok(ControlCommand::RequestStatus),
        "FOCUS_FIRST" => Ok(ControlCommand::FocusFirstWindow),
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

fn send_status(stream: &mut UnixStream, status: &ControlStatus) -> io::Result<()> {
    writeln!(stream, "BEGIN_STATUS")?;
    writeln!(stream, "socket: {}", status.socket_name)?;
    writeln!(stream, "workspace: {}", status.workspace)?;
    writeln!(stream, "managed_windows: {}", status.managed_windows)?;
    writeln!(stream, "ai_paused: {}", status.ai_paused)?;
    writeln!(stream, "ai_task_status: {}", status.ai_task_status)?;
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
