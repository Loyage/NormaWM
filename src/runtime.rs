//! `NormaWM` 的启动与主循环。
//!
//! 这个模块负责把 backend、Wayland display、输入系统和 compositor 状态组装起来，
//! 然后持续驱动事件循环、渲染循环以及 AI 预览刷新。

use std::{sync::Arc, time::Instant};

use crate::{
    ai::{ActionResult, AiCommand, AiEvent, AiNexus},
    compositor::{ClientState, NormaApp, AI_PREVIEW_PATH, DEEP_GRAY},
    error::NormaError,
    monitor::spawn_monitor_window,
    wm::TilingState,
};
use ::winit::platform::pump_events::PumpStatus;
use smithay::{
    backend::{
        input::{InputEvent, KeyboardKeyEvent},
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
        wm_state: TilingState::new(initial_size),
        monitor: spawn_monitor_window(),
    };
    let start_time = Instant::now();
    let mut clients = Vec::new();

    let keyboard = state
        .seat
        .add_keyboard(Default::default(), 200, 200)
        .map_err(|error| NormaError::KeyboardInit(error.to_string()))?;

    info!(
        socket = %socket_name,
        "NormaWM nested compositor started. Launch clients with WAYLAND_DISPLAY={socket_name}"
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
                    keyboard.input::<(), _>(
                        &mut state,
                        event.key_code(),
                        event.state(),
                        0.into(),
                        0,
                        |_, _, _| FilterResult::Forward,
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

        // 处理来自 AI/外部控制面的命令。
        for command in state.ai_nexus.drain_commands() {
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
                .windows()
                .iter()
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

        for window in state.wm_state.windows() {
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
