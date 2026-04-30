#![allow(dead_code)]

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

#[derive(Debug, Clone)]
pub enum AiCommand {
    RequestSnapshot,
    SetClearColor([f32; 4]),
    FocusFirstWindow,
    Shutdown,
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
