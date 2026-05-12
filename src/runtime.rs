//! `NormaWM` 的启动与主循环。
//!
//! 这个模块负责把 backend、Wayland display、输入系统和 compositor 状态组装起来，
//! 然后持续驱动事件循环、渲染循环以及 AI 预览刷新。

use std::{collections::HashSet, sync::Arc, time::Instant};

use crate::{
    ai::{ActionResult, AiCommand, AiEvent, AiNexus},
    atspi::{query_window_accessibility_tree, JsonRectI32, WindowAccessibilityTarget},
    compositor::{ClientState, NormaApp, AI_PREVIEW_PATH, DEEP_GRAY},
    control::{launch_wayland_client, ControlCommand, ControlServer},
    error::NormaError,
    monitor::BackgroundMonitor,
    wm::TilingState,
};
use ::winit::platform::pump_events::PumpStatus;
use serde::Serialize;
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
    utils::{Logical, Point, Rectangle, Size, Transform},
    wayland::{
        compositor::{
            get_children, get_role, with_states, with_surface_tree_downward, SurfaceAttributes,
            TraversalAction,
        },
        selection::data_device::{set_data_device_focus, set_data_device_selection},
        shell::xdg::XdgShellState,
        shm::ShmState,
    },
};
use tracing::{info, warn};
use wayland_server::{protocol::wl_surface, ListeningSocket, Resource};

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
    let mut clients = Vec::new();
    let mut hotkeys = WorkspaceHotkeys::default();
    let mut monitor = BackgroundMonitor::start();
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
        "NormaWM control socket and background monitor are ready"
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
        for request in control_server.poll_commands() {
            monitor.record_command();
            let client_id = request.client_id;
            match request.command {
                ControlCommand::RequestStatus => {
                    let mut status = state.control_status();
                    monitor.enrich_status(&mut status);
                    control_server.send_status_to(client_id, &status);
                }
                ControlCommand::RequestWindows => {
                    control_server.send_text_to(client_id, &format_windows(&state));
                }
                ControlCommand::RequestWorkspaces => {
                    control_server.send_text_to(client_id, &format_workspaces(&state));
                }
                ControlCommand::RequestFocusedWindow => {
                    control_server.send_text_to(client_id, &format_focused_window(&state));
                }
                ControlCommand::RequestWindow(window_id) => {
                    match format_window_accessibility_composition(&state, &window_id) {
                        Ok(json) => control_server.send_text_to(client_id, &json),
                        Err(error) => control_server.send_result_to(client_id, false, &error),
                    }
                }
                ControlCommand::FocusWindow(window_id) => {
                    if let Some(surface) = state.wm_state.focus_window_id(&window_id) {
                        keyboard.set_focus(&mut state, Some(surface), 0.into());
                        control_server.send_result_to(
                            client_id,
                            true,
                            &format!("focused {window_id}"),
                        );
                        state.publish_ai_preview("human_focus_window");
                    } else {
                        control_server.send_result_to(
                            client_id,
                            false,
                            &format!("unknown window id: {window_id}"),
                        );
                    }
                    should_publish_control_status = true;
                }
                ControlCommand::SwitchWorkspace(workspace) => {
                    let focus = state.wm_state.switch_workspace(workspace);
                    keyboard.set_focus(&mut state, focus, 0.into());
                    control_server.send_result_to(
                        client_id,
                        true,
                        &format!("switched to workspace {workspace}"),
                    );
                    state.publish_ai_preview("human_switch_workspace");
                    should_publish_control_status = true;
                }
                ControlCommand::InjectText { target, text } => {
                    let result = inject_text_into_window(
                        &dh,
                        &mut state,
                        &keyboard,
                        target.as_deref(),
                        text,
                        start_time.elapsed().as_millis() as u32,
                    );
                    match result {
                        Ok(window_id) => {
                            control_server.send_result_to(
                                client_id,
                                true,
                                &format!("input text into {window_id}"),
                            );
                        }
                        Err(error) => {
                            control_server.send_result_to(client_id, false, &error);
                        }
                    }
                    should_publish_control_status = true;
                }
                ControlCommand::FocusFirstWindow => {
                    if let Some(surface) = state.wm_state.focus_first() {
                        keyboard.set_focus(&mut state, Some(surface), 0.into());
                        control_server.send_result_to(
                            client_id,
                            true,
                            "focused the first toplevel surface",
                        );
                        state.publish_ai_preview("human_focus_first_window");
                    } else {
                        control_server.send_result_to(
                            client_id,
                            false,
                            "no toplevel surfaces are available",
                        );
                    }
                    should_publish_control_status = true;
                }
                ControlCommand::Launch(command) => {
                    match launch_wayland_client(&command, &state.socket_name) {
                        Ok(pid) => {
                            control_server.send_result_to(
                                client_id,
                                true,
                                &format!("launched {} as pid {pid}", command.join(" ")),
                            );
                        }
                        Err(error) => {
                            control_server.send_result_to(
                                client_id,
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
                    control_server.send_result_to(client_id, true, "AI control paused");
                    should_publish_control_status = true;
                }
                ControlCommand::ResumeAi => {
                    state.ai_paused = false;
                    state.ai_task_status = "idle".to_string();
                    control_server.send_result_to(client_id, true, "AI control resumed");
                    should_publish_control_status = true;
                }
                ControlCommand::CancelAiTasks => {
                    state.ai_task_status = "cancelled by human control".to_string();
                    control_server.send_result_to(client_id, true, "AI tasks marked cancelled");
                    should_publish_control_status = true;
                }
                ControlCommand::Shutdown => {
                    state.shutdown_requested = true;
                    control_server.send_result_to(client_id, true, "shutdown requested");
                    should_publish_control_status = true;
                }
            }
        }

        if monitor.should_broadcast_status(should_publish_control_status) {
            let mut status = state.control_status();
            monitor.enrich_status(&mut status);
            control_server.broadcast_status(&status);
            monitor.record_status_broadcast();
        }

        // 处理来自 AI 接入面的命令。人类控制面暂停 AI 时，这些命令会被拒绝。
        for command in state.ai_nexus.drain_commands() {
            if state.ai_paused {
                state.ai_nexus.emit(AiEvent::ActionResult(ActionResult::err(
                    "AI control is paused by the human control plane",
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

fn inject_text_into_window(
    dh: &wayland_server::DisplayHandle,
    state: &mut NormaApp,
    keyboard: &smithay::input::keyboard::KeyboardHandle<NormaApp>,
    target: Option<&str>,
    text: String,
    time: u32,
) -> Result<String, String> {
    let window_id = match target {
        Some(window_id) => window_id.to_string(),
        None => state
            .wm_state
            .focused_window_id()
            .ok_or_else(|| "no focused window is available".to_string())?
            .to_string(),
    };

    let Some(surface) = state.wm_state.focus_window_id(&window_id) else {
        return Err(format!("unknown window id: {window_id}"));
    };
    let Some(client) = surface.client() else {
        return Err(format!("target window is no longer alive: {window_id}"));
    };

    keyboard.set_focus(state, Some(surface), 0.into());
    set_data_device_focus(dh, &state.seat, Some(client));
    set_data_device_selection(
        dh,
        &state.seat,
        vec![
            "text/plain;charset=utf-8".to_string(),
            "text/plain".to_string(),
        ],
        text,
    );
    send_ctrl_v(state, keyboard, time);
    state.publish_ai_preview("control_input_text");

    Ok(window_id)
}

fn send_ctrl_v(
    state: &mut NormaApp,
    keyboard: &smithay::input::keyboard::KeyboardHandle<NormaApp>,
    time: u32,
) {
    keyboard.input::<(), _>(
        state,
        KEY_LEFTCTRL,
        KeyState::Pressed,
        0.into(),
        time,
        |_, _, _| FilterResult::Forward,
    );
    keyboard.input::<(), _>(
        state,
        KEY_V,
        KeyState::Pressed,
        0.into(),
        time,
        |_, _, _| FilterResult::Forward,
    );
    keyboard.input::<(), _>(
        state,
        KEY_V,
        KeyState::Released,
        0.into(),
        time,
        |_, _, _| FilterResult::Forward,
    );
    keyboard.input::<(), _>(
        state,
        KEY_LEFTCTRL,
        KeyState::Released,
        0.into(),
        time,
        |_, _, _| FilterResult::Forward,
    );
}

fn format_windows(state: &NormaApp) -> String {
    let windows = state.wm_state.control_windows();
    if windows.is_empty() {
        return "no windows".to_string();
    }

    windows
        .into_iter()
        .map(|window| {
            format!(
                "{} workspace={} focused={} title={} app_id={} human_control={}",
                window.id,
                window.workspace,
                window.focused,
                window.title.as_deref().unwrap_or("<unset>"),
                window.app_id.as_deref().unwrap_or("<unset>"),
                window.human_control
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_workspaces(state: &NormaApp) -> String {
    let active = state.wm_state.active_workspace();
    (0..=9)
        .map(|workspace| {
            if workspace == active {
                format!("* {workspace}")
            } else {
                format!("  {workspace}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_focused_window(state: &NormaApp) -> String {
    state
        .wm_state
        .focused_window_id()
        .map(str::to_string)
        .unwrap_or_else(|| "no focused window".to_string())
}

fn format_window_accessibility_composition(
    state: &NormaApp,
    window_id: &str,
) -> Result<String, String> {
    let window_info = state
        .wm_state
        .control_windows()
        .into_iter()
        .find(|window| window.id == window_id)
        .ok_or_else(|| format!("unknown window id: {window_id}"))?;
    let managed_window = state
        .wm_state
        .windows()
        .iter()
        .find(|window| window.id == window_id && window.surface.alive())
        .ok_or_else(|| format!("unknown window id: {window_id}"))?;

    let target = WindowAccessibilityTarget {
        id: window_info.id,
        workspace: window_info.workspace,
        title: window_info.title,
        app_id: window_info.app_id,
        focused: window_info.focused,
        human_control: window_info.human_control,
        visible: managed_window.workspace == state.wm_state.active_workspace(),
        layout_geometry: JsonRectI32 {
            x: managed_window.geometry.loc.x,
            y: managed_window.geometry.loc.y,
            width: managed_window.geometry.size.w,
            height: managed_window.geometry.size.h,
        },
    };

    let composition = match query_window_accessibility_tree(target.clone()) {
        Ok(composition) => WindowInspectionResponse::from_atspi(composition),
        Err(reason) => build_surface_tree_response(target, managed_window, reason),
    };

    serde_json::to_string_pretty(&composition)
        .map_err(|error| format!("failed to serialize window composition: {error}"))
}

fn build_surface_tree_response(
    window: WindowAccessibilityTarget,
    managed_window: &crate::wm::ManagedToplevel,
    reason: String,
) -> WindowInspectionResponse {
    let mut surfaces = Vec::new();
    collect_surface_composition(
        managed_window.surface.wl_surface(),
        None,
        0,
        managed_window.geometry.loc,
        &mut surfaces,
    );

    WindowInspectionResponse {
        window: WindowInspectionWindow::from(window),
        inspection: WindowInspection::SurfaceTree {
            reason,
            surface_count: surfaces.len(),
            surfaces,
        },
    }
}

fn collect_surface_composition(
    surface: &wl_surface::WlSurface,
    parent_index: Option<usize>,
    depth: usize,
    base_location: Point<i32, Logical>,
    surfaces: &mut Vec<SurfaceNode>,
) {
    let snapshot = surface_render_snapshot(surface);
    let absolute_location = Point::from((
        base_location.x + snapshot.local_offset.x,
        base_location.y + snapshot.local_offset.y,
    ));
    let index = surfaces.len();

    surfaces.push(SurfaceNode {
        index,
        parent_index,
        depth,
        role: get_role(surface).unwrap_or("unassigned").to_string(),
        alive: surface.is_alive(),
        absolute_location: JsonPointI32::from(absolute_location),
        local_offset: JsonPointI32::from(snapshot.local_offset),
        mapped: snapshot.mapped,
        buffer: snapshot.buffer,
        view: snapshot.view,
        pending_frame_callbacks: snapshot.pending_frame_callbacks,
        pending_damage_count: snapshot.pending_damage_count,
    });

    if !snapshot.traverse_children {
        return;
    }

    for child in get_children(surface) {
        collect_surface_composition(&child, Some(index), depth + 1, absolute_location, surfaces);
    }
}

fn surface_render_snapshot(surface: &wl_surface::WlSurface) -> SurfaceSnapshot {
    with_states(surface, |states| {
        let mut attrs_guard = states.cached_state.get::<SurfaceAttributes>();
        let attrs = attrs_guard.current();
        let pending_frame_callbacks = attrs.frame_callbacks.len();
        let pending_damage_count = attrs.damage.len();

        let Some(renderer_data) = states
            .data_map
            .get::<smithay::backend::renderer::utils::RendererSurfaceStateUserData>(
        ) else {
            return SurfaceSnapshot {
                pending_frame_callbacks,
                pending_damage_count,
                ..SurfaceSnapshot::default()
            };
        };
        let Ok(renderer_state) = renderer_data.lock() else {
            return SurfaceSnapshot {
                pending_frame_callbacks,
                pending_damage_count,
                ..SurfaceSnapshot::default()
            };
        };

        let view = renderer_state.view().map(SurfaceViewInfo::from);
        let local_offset = view
            .as_ref()
            .map(|view| view.offset.to_point())
            .unwrap_or_else(|| Point::from((0, 0)));
        let mapped = renderer_state.buffer().is_some() && view.is_some();
        let buffer = renderer_state.buffer().map(|_| SurfaceBufferInfo {
            surface_width: renderer_state.surface_size().map(|size| size.w),
            surface_height: renderer_state.surface_size().map(|size| size.h),
            buffer_width: renderer_state.buffer_size().map(|size| size.w),
            buffer_height: renderer_state.buffer_size().map(|size| size.h),
            buffer_scale: renderer_state.buffer_scale(),
            buffer_transform: format!("{:?}", renderer_state.buffer_transform()),
        });

        SurfaceSnapshot {
            mapped,
            traverse_children: view.is_some(),
            local_offset,
            buffer,
            view,
            pending_frame_callbacks,
            pending_damage_count,
        }
    })
}

#[derive(Debug, Default)]
struct SurfaceSnapshot {
    mapped: bool,
    traverse_children: bool,
    local_offset: Point<i32, Logical>,
    buffer: Option<SurfaceBufferInfo>,
    view: Option<SurfaceViewInfo>,
    pending_frame_callbacks: usize,
    pending_damage_count: usize,
}

#[derive(Debug, Serialize)]
struct WindowInspectionResponse {
    window: WindowInspectionWindow,
    inspection: WindowInspection,
}

impl WindowInspectionResponse {
    fn from_atspi(composition: crate::atspi::WindowAccessibilityComposition) -> Self {
        Self {
            window: WindowInspectionWindow::from(composition.window),
            inspection: WindowInspection::Atspi {
                matched_by: composition.accessibility.matched_by,
                applications_seen: composition.accessibility.applications_seen,
                node_count: composition.accessibility.node_count,
                truncated: composition.accessibility.truncated,
                tree: composition.accessibility.tree,
            },
        }
    }
}

#[derive(Debug, Serialize)]
struct WindowInspectionWindow {
    id: String,
    workspace: u8,
    title: Option<String>,
    app_id: Option<String>,
    focused: bool,
    human_control: bool,
    visible: bool,
    layout_geometry: JsonRectI32,
}

impl From<WindowAccessibilityTarget> for WindowInspectionWindow {
    fn from(target: WindowAccessibilityTarget) -> Self {
        Self {
            id: target.id,
            workspace: target.workspace,
            title: target.title,
            app_id: target.app_id,
            focused: target.focused,
            human_control: target.human_control,
            visible: target.visible,
            layout_geometry: target.layout_geometry,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "protocol", rename_all = "kebab-case")]
enum WindowInspection {
    Atspi {
        matched_by: String,
        applications_seen: Vec<crate::atspi::AtspiApplicationSummary>,
        node_count: usize,
        truncated: bool,
        tree: crate::atspi::AtspiNode,
    },
    SurfaceTree {
        reason: String,
        surface_count: usize,
        surfaces: Vec<SurfaceNode>,
    },
}

#[derive(Debug, Serialize)]
struct SurfaceNode {
    index: usize,
    parent_index: Option<usize>,
    depth: usize,
    role: String,
    alive: bool,
    absolute_location: JsonPointI32,
    local_offset: JsonPointI32,
    mapped: bool,
    buffer: Option<SurfaceBufferInfo>,
    view: Option<SurfaceViewInfo>,
    pending_frame_callbacks: usize,
    pending_damage_count: usize,
}

#[derive(Debug, Serialize)]
struct SurfaceBufferInfo {
    surface_width: Option<i32>,
    surface_height: Option<i32>,
    buffer_width: Option<i32>,
    buffer_height: Option<i32>,
    buffer_scale: i32,
    buffer_transform: String,
}

#[derive(Debug, Serialize)]
struct SurfaceViewInfo {
    src: JsonRectF64,
    dst: JsonSizeI32,
    offset: JsonPointI32,
}

impl From<smithay::backend::renderer::utils::SurfaceView> for SurfaceViewInfo {
    fn from(view: smithay::backend::renderer::utils::SurfaceView) -> Self {
        Self {
            src: JsonRectF64::from(view.src),
            dst: JsonSizeI32::from(view.dst),
            offset: JsonPointI32::from(view.offset),
        }
    }
}

#[derive(Debug, Serialize)]
struct JsonRectF64 {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl From<Rectangle<f64, Logical>> for JsonRectF64 {
    fn from(rect: Rectangle<f64, Logical>) -> Self {
        Self {
            x: rect.loc.x,
            y: rect.loc.y,
            width: rect.size.w,
            height: rect.size.h,
        }
    }
}

#[derive(Debug, Serialize)]
struct JsonSizeI32 {
    width: i32,
    height: i32,
}

impl From<Size<i32, Logical>> for JsonSizeI32 {
    fn from(size: Size<i32, Logical>) -> Self {
        Self {
            width: size.w,
            height: size.h,
        }
    }
}

#[derive(Debug, Serialize)]
struct JsonPointI32 {
    x: i32,
    y: i32,
}

impl JsonPointI32 {
    fn to_point(&self) -> Point<i32, Logical> {
        Point::from((self.x, self.y))
    }
}

impl From<Point<i32, Logical>> for JsonPointI32 {
    fn from(point: Point<i32, Logical>) -> Self {
        Self {
            x: point.x,
            y: point.y,
        }
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
const KEY_V: Keycode = xkb_keycode(47);
const KEY_LEFTCTRL: Keycode = xkb_keycode(29);
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
