//! `NormaWM` 的启动与主循环。
//!
//! 这个模块负责把 backend、Wayland display、输入系统和 compositor 状态组装起来，
//! 然后持续驱动事件循环、渲染循环以及 AI 预览刷新。

use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    ai::{ActionResult, AiCommand, AiEvent, AiNexus},
    compositor::{ClientState, NormaApp, AI_PREVIEW_PATH, DEEP_GRAY},
    control::{launch_wayland_client, ControlCommand, ControlServer},
    error::NormaError,
    wm::TilingState,
};
use ::winit::platform::pump_events::PumpStatus;
use smithay::{
    backend::{
        input::{InputEvent, KeyState, KeyboardKeyEvent, Keycode},
        renderer::{
            element::{
                surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement},
                Kind,
            },
            gles::GlesRenderer,
            utils::draw_render_elements,
            Color32F, Frame, Renderer,
        },
        winit::{self as backend_winit, WinitEvent},
    },
    input::keyboard::FilterResult,
    reexports::wayland_server::Display,
    utils::{Logical, Rectangle, Size, Transform},
    wayland::{
        compositor::{with_surface_tree_downward, SurfaceAttributes, TraversalAction},
        shell::xdg::XdgShellState,
        shm::ShmState,
    },
};
use tracing::{info, warn};
use wayland_server::{protocol::wl_surface, ListeningSocket};

/// 初始化全局 tracing subscriber。
///
/// 默认打开 `normawm` 和 `smithay` 的日志，便于在早期阶段直接观察协议与渲染行为。
pub fn init_tracing() {
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "normawm=debug,smithay=info".into()),
        )
        .finish();

    let _ = tracing::subscriber::set_global_default(subscriber);
}

/// 启动 nested compositor 的主循环。
///
/// 这个函数是当前二进制最核心的“组装函数”：
/// - 创建 display / backend / seat / shell
/// - 注册 socket
/// - 驱动事件、Wayland client、渲染和 AI 预览刷新
pub fn run_winit(ai_nexus: AiNexus) -> Result<(), NormaError> {
    let mut display: Display<NormaApp> =
        Display::new().map_err(|error| NormaError::DisplayInit(error.to_string()))?;
    let dh = display.handle();
    let (mut backend, mut winit) = backend_winit::init::<GlesRenderer>()
        .map_err(|error| NormaError::WinitBackend(error.to_string()))?;
    let initial_size = to_logical_size(backend.window_size());

    let compositor_state = smithay::wayland::compositor::CompositorState::new::<NormaApp>(&dh);
    let shm_state = ShmState::new::<NormaApp>(&dh, vec![]);
    let mut seat_state = smithay::input::SeatState::new();
    let seat = seat_state.new_wl_seat(&dh, "normawm-winit");

    let listener = ListeningSocket::bind_auto("normawm", 0..16)
        .map_err(|error| NormaError::SocketBind(error.to_string()))?;
    let socket_name = listener
        .socket_name()
        .and_then(|name| name.to_str())
        .unwrap_or("normawm-unknown")
        .to_owned();

    let mut state = NormaApp {
        compositor_state,
        xdg_shell_state: XdgShellState::new::<NormaApp>(&dh),
        shm_state,
        seat_state,
        data_device_state: smithay::wayland::selection::data_device::DataDeviceState::new::<NormaApp>(
            &dh,
        ),
        seat,
        ai_nexus,
        clear_color: DEEP_GRAY,
        socket_name: socket_name.clone(),
        shutdown_requested: false,
        ai_paused: false,
        ai_task_status: "idle".to_string(),
        wm_state: TilingState::new(initial_size),
    };
    let start_time = Instant::now();
    let mut last_control_status = Instant::now();
    let mut clients = Vec::new();
    let mut hotkeys = WorkspaceHotkeys::default();
    let mut control_server =
        ControlServer::bind_default().map_err(|error| NormaError::SocketBind(error.to_string()))?;

    let keyboard = state
        .seat
        .add_keyboard(Default::default(), 200, 200)
        .map_err(|error| NormaError::KeyboardInit(error.to_string()))?;

    info!(
        socket = %socket_name,
        "NormaWM nested compositor started. Launch clients with WAYLAND_DISPLAY={socket_name}"
    );
    info!(
        socket = %control_server.socket_path().display(),
        "NormaWM human control socket is ready"
    );
    info!(
        target: "normawm::ai_preview",
        path = AI_PREVIEW_PATH,
        "AI input preview will be mirrored to this file"
    );
    state.publish_ai_preview("startup");

    loop {
        // 先处理宿主窗口系统送来的事件，例如输入与尺寸变化。
        let status = winit.dispatch_new_events(|event| match event {
            WinitEvent::Resized { .. } => {}
            WinitEvent::Input(event) => match event {
                InputEvent::Keyboard { event } => {
                    let hotkey = hotkeys.handle_key_event(event.key_code(), event.state());
                    if let Some(action) = hotkey.action {
                        let focus = match action {
                            WorkspaceHotkeyAction::Switch(workspace) => {
                                state.wm_state.switch_workspace(workspace)
                            }
                            WorkspaceHotkeyAction::Relative(delta) => {
                                state.wm_state.switch_workspace_relative(delta)
                            }
                        };
                        keyboard.set_focus(&mut state, focus, 0.into());
                        state.publish_ai_preview("workspace_hotkey");
                    }

                    keyboard.input::<(), _>(
                        &mut state,
                        event.key_code(),
                        event.state(),
                        0.into(),
                        0,
                        |_, _, _| {
                            if hotkey.intercept {
                                FilterResult::Intercept(())
                            } else {
                                FilterResult::Forward
                            }
                        },
                    );
                }
                InputEvent::PointerMotionAbsolute { .. } => {
                    let focus = state.wm_state.focus_first();
                    if focus.is_some() {
                        keyboard.set_focus(&mut state, focus, 0.into());
                    }
                }
                _ => {}
            },
            _ => {}
        });

        match status {
            PumpStatus::Continue => {}
            PumpStatus::Exit(_) => return Ok(()),
        }

        // 在每轮主循环里把窗口管理状态与输出尺寸同步到最新。
        if state.wm_state.prune_dead_windows() {
            state.publish_ai_preview("prune_dead_windows");
        }
        if state
            .wm_state
            .set_output_size(to_logical_size(backend.window_size()))
        {
            state.publish_ai_preview("output_resized");
        }

        // 处理来自本地人类控制面的命令。人类控制优先级高于 AI 自动控制。
        let mut should_publish_control_status = false;
        for command in control_server.poll_commands() {
            match command {
                ControlCommand::RequestStatus => {
                    should_publish_control_status = true;
                }
                ControlCommand::FocusFirstWindow => {
                    if let Some(surface) = state.wm_state.focus_first() {
                        keyboard.set_focus(&mut state, Some(surface), 0.into());
                        control_server.broadcast_result(true, "focused the first toplevel surface");
                        state.publish_ai_preview("human_focus_first_window");
                    } else {
                        control_server
                            .broadcast_result(false, "no toplevel surfaces are available");
                    }
                    should_publish_control_status = true;
                }
                ControlCommand::Launch(command) => {
                    match launch_wayland_client(&command, &state.socket_name) {
                        Ok(pid) => {
                            control_server.broadcast_result(
                                true,
                                &format!("launched {} as pid {pid}", command.join(" ")),
                            );
                        }
                        Err(error) => {
                            control_server.broadcast_result(
                                false,
                                &format!("failed to launch {}: {error}", command.join(" ")),
                            );
                        }
                    }
                    should_publish_control_status = true;
                }
                ControlCommand::PauseAi => {
                    state.ai_paused = true;
                    state.ai_task_status = "paused by human control".to_string();
                    control_server.broadcast_result(true, "AI control paused");
                    should_publish_control_status = true;
                }
                ControlCommand::ResumeAi => {
                    state.ai_paused = false;
                    state.ai_task_status = "idle".to_string();
                    control_server.broadcast_result(true, "AI control resumed");
                    should_publish_control_status = true;
                }
                ControlCommand::CancelAiTasks => {
                    state.ai_task_status = "cancelled by human control".to_string();
                    control_server.broadcast_result(true, "AI tasks marked cancelled");
                    should_publish_control_status = true;
                }
                ControlCommand::Shutdown => {
                    state.shutdown_requested = true;
                    control_server.broadcast_result(true, "shutdown requested");
                    should_publish_control_status = true;
                }
            }
        }

        if should_publish_control_status
            || last_control_status.elapsed() >= Duration::from_millis(500)
        {
            control_server.broadcast_status(&state.control_status());
            last_control_status = Instant::now();
        }

        // 处理来自 AI 接入面的命令。人类控制面暂停 AI 时，这些命令会被拒绝。
        for command in state.ai_nexus.drain_commands() {
            if state.ai_paused {
                state.ai_nexus.emit(AiEvent::ActionResult(ActionResult::err(
                    "AI control is paused by the human control panel",
                )));
                continue;
            }

            match command {
                AiCommand::RequestSnapshot => {
                    state.ai_nexus.emit(AiEvent::Snapshot(state.snapshot()));
                    state.publish_ai_preview("request_snapshot");
                }
                AiCommand::SetClearColor(color) => {
                    state.clear_color = color;
                    state.ai_nexus.emit(AiEvent::ActionResult(ActionResult::ok(
                        "updated clear color",
                    )));
                }
                AiCommand::FocusFirstWindow => {
                    if let Some(surface) = state.wm_state.focus_first() {
                        keyboard.set_focus(&mut state, Some(surface), 0.into());
                        state.ai_nexus.emit(AiEvent::ActionResult(ActionResult::ok(
                            "focused the first toplevel surface",
                        )));
                        state.publish_ai_preview("focus_first_window");
                    } else {
                        state.ai_nexus.emit(AiEvent::ActionResult(ActionResult::err(
                            "no toplevel surfaces are available yet",
                        )));
                    }
                }
                AiCommand::Shutdown => {
                    state.shutdown_requested = true;
                    state.ai_nexus.emit(AiEvent::ActionResult(ActionResult::ok(
                        "shutdown requested",
                    )));
                }
            }
        }

        if state.shutdown_requested {
            warn!("NormaWM received shutdown request from AiNexus");
            return Ok(());
        }

        // 把窗口管理层认定的焦点同步到 Smithay 键盘焦点。
        let focus_surface = state.wm_state.focused_surface();
        if keyboard.current_focus() != focus_surface {
            keyboard.set_focus(&mut state, focus_surface, 0.into());
        }

        // 受理新的 Wayland client 连接。
        while let Some(stream) = listener
            .accept()
            .map_err(|error| NormaError::AcceptClient(error.to_string()))?
        {
            let client = display
                .handle()
                .insert_client(stream, Arc::new(ClientState::default()))
                .map_err(|error| NormaError::InsertClient(error.to_string()))?;
            clients.push(client);
        }

        display
            .dispatch_clients(&mut state)
            .map_err(|error| NormaError::WaylandDispatch(error.to_string()))?;
        display
            .flush_clients()
            .map_err(|error| NormaError::WaylandFlush(error.to_string()))?;

        // 按窗口管理层计算好的逻辑坐标，把各个窗口 surface 渲染到 nested 输出里。
        let size = backend.window_size();
        let damage = Rectangle::from_size(size);
        {
            let (renderer, mut framebuffer) = backend
                .bind()
                .map_err(|error| NormaError::RendererBind(error.to_string()))?;

            let elements = state
                .wm_state
                .visible_windows()
                .flat_map(|window| {
                    render_elements_from_surface_tree(
                        renderer,
                        window.surface.wl_surface(),
                        (window.geometry.loc.x, window.geometry.loc.y),
                        1.0,
                        1.0,
                        Kind::Unspecified,
                    )
                })
                .collect::<Vec<WaylandSurfaceRenderElement<GlesRenderer>>>();

            let mut frame = renderer
                .render(&mut framebuffer, size, Transform::Flipped180)
                .map_err(|error| NormaError::Render(error.to_string()))?;

            frame
                .clear(
                    Color32F::new(
                        state.clear_color[0],
                        state.clear_color[1],
                        state.clear_color[2],
                        state.clear_color[3],
                    ),
                    &[damage],
                )
                .map_err(|error| NormaError::Render(error.to_string()))?;
            draw_render_elements(&mut frame, 1.0, &elements, &[damage])
                .map_err(|error| NormaError::Render(error.to_string()))?;
            let _ = frame
                .finish()
                .map_err(|error| NormaError::Render(error.to_string()))?;
        }

        for window in state.wm_state.visible_windows() {
            send_frames_surface_tree(
                window.surface.wl_surface(),
                start_time.elapsed().as_millis() as u32,
            );
        }

        backend
            .submit(Some(&[damage]))
            .map_err(|error| NormaError::Submit(error.to_string()))?;
    }
}

#[derive(Debug, Clone, Copy)]
enum WorkspaceHotkeyAction {
    Switch(u8),
    Relative(i8),
}

#[derive(Debug, Clone, Copy)]
struct WorkspaceHotkey {
    action: Option<WorkspaceHotkeyAction>,
    intercept: bool,
}

#[derive(Debug, Default)]
struct WorkspaceHotkeys {
    pressed: HashSet<Keycode>,
    intercepted: HashSet<Keycode>,
}

impl WorkspaceHotkeys {
    fn handle_key_event(&mut self, keycode: Keycode, state: KeyState) -> WorkspaceHotkey {
        match state {
            KeyState::Pressed => {
                self.pressed.insert(keycode);

                if let Some(action) = self.action_for_pressed_key(keycode) {
                    self.intercepted.insert(keycode);
                    return WorkspaceHotkey {
                        action: Some(action),
                        intercept: true,
                    };
                }

                WorkspaceHotkey {
                    action: None,
                    intercept: false,
                }
            }
            KeyState::Released => {
                self.pressed.remove(&keycode);
                let intercept = self.intercepted.remove(&keycode);

                WorkspaceHotkey {
                    action: None,
                    intercept,
                }
            }
        }
    }

    fn action_for_pressed_key(&self, keycode: Keycode) -> Option<WorkspaceHotkeyAction> {
        if !self.mod_and_alt_pressed() {
            return None;
        }

        match keycode {
            KEY_J => Some(WorkspaceHotkeyAction::Relative(1)),
            KEY_K => Some(WorkspaceHotkeyAction::Relative(-1)),
            KEY_0 => Some(WorkspaceHotkeyAction::Switch(0)),
            KEY_1 => Some(WorkspaceHotkeyAction::Switch(1)),
            KEY_2 => Some(WorkspaceHotkeyAction::Switch(2)),
            KEY_3 => Some(WorkspaceHotkeyAction::Switch(3)),
            KEY_4 => Some(WorkspaceHotkeyAction::Switch(4)),
            KEY_5 => Some(WorkspaceHotkeyAction::Switch(5)),
            KEY_6 => Some(WorkspaceHotkeyAction::Switch(6)),
            KEY_7 => Some(WorkspaceHotkeyAction::Switch(7)),
            KEY_8 => Some(WorkspaceHotkeyAction::Switch(8)),
            KEY_9 => Some(WorkspaceHotkeyAction::Switch(9)),
            _ => None,
        }
    }

    fn mod_and_alt_pressed(&self) -> bool {
        let mod_pressed =
            self.pressed.contains(&KEY_LEFTMETA) || self.pressed.contains(&KEY_RIGHTMETA);
        let alt_pressed =
            self.pressed.contains(&KEY_LEFTALT) || self.pressed.contains(&KEY_RIGHTALT);

        mod_pressed && alt_pressed
    }
}

const fn xkb_keycode(evdev_code: u32) -> Keycode {
    Keycode::new(evdev_code + 8)
}

const KEY_1: Keycode = xkb_keycode(2);
const KEY_2: Keycode = xkb_keycode(3);
const KEY_3: Keycode = xkb_keycode(4);
const KEY_4: Keycode = xkb_keycode(5);
const KEY_5: Keycode = xkb_keycode(6);
const KEY_6: Keycode = xkb_keycode(7);
const KEY_7: Keycode = xkb_keycode(8);
const KEY_8: Keycode = xkb_keycode(9);
const KEY_9: Keycode = xkb_keycode(10);
const KEY_0: Keycode = xkb_keycode(11);
const KEY_J: Keycode = xkb_keycode(36);
const KEY_K: Keycode = xkb_keycode(37);
const KEY_LEFTALT: Keycode = xkb_keycode(56);
const KEY_RIGHTALT: Keycode = xkb_keycode(100);
const KEY_LEFTMETA: Keycode = xkb_keycode(125);
const KEY_RIGHTMETA: Keycode = xkb_keycode(126);

/// 将 backend 返回的物理尺寸转换成当前布局层使用的逻辑尺寸。
pub fn to_logical_size(size: Size<i32, smithay::utils::Physical>) -> Size<i32, Logical> {
    Size::from((size.w, size.h))
}

/// 把 frame callback 发送给整棵 surface tree。
///
/// 这是 Wayland 客户端继续下一帧渲染的重要节奏信号。
pub fn send_frames_surface_tree(surface: &wl_surface::WlSurface, time: u32) {
    with_surface_tree_downward(
        surface,
        (),
        |_, _, &()| TraversalAction::DoChildren(()),
        |_surface, states, &()| {
            for callback in states
                .cached_state
                .get::<SurfaceAttributes>()
                .current()
                .frame_callbacks
                .drain(..)
            {
                callback.done(time);
            }
        },
        |_, _, &()| true,
    );
}
