//! `NormaWM` 当前的主二进制入口。
//!
//! 这个文件主要负责“组装”系统：
//! - 初始化 Smithay 的 winit backend
//! - 注册 Wayland/seat/shm/xdg_shell handler
//! - 驱动主事件循环和渲染循环
//! - 在窗口状态变化时发布 AI 输入预览

use std::{fs, os::unix::io::OwnedFd, sync::Arc, time::Instant};

use ::winit::platform::pump_events::PumpStatus;
use normawm::{
    ai::{format_ai_window_digest, ActionResult, AiCommand, AiEvent, AiNexus, CompositorSnapshot},
    error::NormaError,
    wm::TilingState,
};
use smithay::{
    backend::{
        input::{InputEvent, KeyboardKeyEvent},
        renderer::{
            element::{
                surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement},
                Kind,
            },
            gles::GlesRenderer,
            utils::{draw_render_elements, on_commit_buffer_handler},
            Color32F, Frame, Renderer,
        },
        winit::{self as backend_winit, WinitEvent},
    },
    delegate_compositor, delegate_data_device, delegate_seat, delegate_shm, delegate_xdg_shell,
    input::{keyboard::FilterResult, pointer::CursorImageStatus, Seat, SeatHandler, SeatState},
    reexports::wayland_server::{protocol::wl_seat, Display},
    utils::{Logical, Rectangle, Serial, Size, Transform},
    wayland::{
        buffer::BufferHandler,
        compositor::{
            with_surface_tree_downward, CompositorClientState, CompositorHandler, CompositorState,
            SurfaceAttributes, TraversalAction,
        },
        selection::{
            data_device::{
                ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
            },
            SelectionHandler,
        },
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
        },
        shm::{ShmHandler, ShmState},
    },
};
use tracing::{info, warn};
use wayland_server::{
    backend::{ClientData, ClientId, DisconnectReason},
    protocol::{
        wl_buffer,
        wl_surface::{self, WlSurface},
    },
    Client, ListeningSocket,
};

const DEEP_GRAY: [f32; 4] = [0.13, 0.13, 0.14, 1.0];
const AI_PREVIEW_PATH: &str = "target/ai-input-preview.txt";

/// 整个 compositor 在运行时的聚合状态。
///
/// 这是 Smithay 各个 handler trait 共享的中心对象：
/// 图形、输入、Wayland、AI 预览与窗口管理状态都从这里读取。
struct NormaApp {
    compositor_state: CompositorState,
    xdg_shell_state: XdgShellState,
    shm_state: ShmState,
    seat_state: SeatState<Self>,
    data_device_state: DataDeviceState,
    seat: Seat<Self>,
    ai_nexus: AiNexus,
    clear_color: [f32; 4],
    socket_name: String,
    shutdown_requested: bool,
    wm_state: TilingState,
}

impl NormaApp {
    /// 生成一份面向外部观察者的轻量快照。
    ///
    /// 它不包含窗口逐项信息，主要用来响应简单状态查询。
    fn snapshot(&self) -> CompositorSnapshot {
        CompositorSnapshot {
            backend: "winit",
            socket_name: self.socket_name.clone(),
            workspace: "main",
            toplevel_count: self.wm_state.len(),
            clear_color: self.clear_color,
        }
    }

    /// 基于当前窗口管理状态生成一段完整的 AI 输入预览。
    fn build_ai_preview(&self) -> String {
        let digest = self.wm_state.build_ai_window_digest("main");
        format_ai_window_digest(&digest)
    }

    /// 在窗口状态变化时向“AI 前向输入”观察面发布最新预览。
    ///
    /// 当前有两个观察出口：
    /// 1. 通过 `AiEvent::PromptPreview` 发给外部接入端
    /// 2. 同步打印到终端并写入固定文件，便于人工检查
    fn publish_ai_preview(&self, reason: &str) {
        let preview = self.build_ai_preview();

        self.ai_nexus.emit(AiEvent::PromptPreview(preview.clone()));

        info!(target: "normawm::ai_preview", reason = reason, "\n{preview}");

        if let Err(error) = fs::write(AI_PREVIEW_PATH, &preview) {
            warn!(
                target: "normawm::ai_preview",
                path = AI_PREVIEW_PATH,
                %error,
                "failed to persist AI input preview"
            );
        }
    }
}

impl BufferHandler for NormaApp {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl XdgShellHandler for NormaApp {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    /// 新建顶层窗口时，把它纳入窗口管理并立即刷新 AI 输入预览。
    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        self.wm_state.insert_toplevel(surface);
        self.ai_nexus.emit(AiEvent::ActionResult(ActionResult::ok(
            "registered new toplevel surface",
        )));
        self.publish_ai_preview("new_toplevel");
    }

    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {}

    /// 目前还没有实现真正的交互拖拽移动。
    ///
    /// 这里先把请求解释为“用户正在与该窗口交互”，因此只同步焦点和预览。
    fn move_request(&mut self, surface: ToplevelSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        self.wm_state.focus_toplevel(&surface);
        self.wm_state.refresh();
        self.publish_ai_preview("move_request");
    }

    /// 目前还没有实现边缘拖拽式 resize。
    ///
    /// 对 MVP 而言，我们只刷新焦点与布局，并把这个变化反映到 AI 预览里。
    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        _seat: wl_seat::WlSeat,
        _serial: Serial,
        _edges: wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge,
    ) {
        self.wm_state.focus_toplevel(&surface);
        self.wm_state.refresh();
        self.publish_ai_preview("resize_request");
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {}

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32,
    ) {
    }

    /// 先把 maximize 解释成一次需要更新布局与 AI 观察面的状态变化。
    fn maximize_request(&mut self, surface: ToplevelSurface) {
        self.wm_state.focus_toplevel(&surface);
        self.wm_state.refresh();
        self.publish_ai_preview("maximize_request");
    }

    /// fullscreen 目前也走同样的“焦点 + 刷新预览”降级路径。
    fn fullscreen_request(
        &mut self,
        surface: ToplevelSurface,
        _output: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>,
    ) {
        self.wm_state.focus_toplevel(&surface);
        self.wm_state.refresh();
        self.publish_ai_preview("fullscreen_request");
    }

    /// 客户端销毁窗口时，把它从窗口管理器移除并刷新 AI 预览。
    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.wm_state.remove_toplevel(&surface);
        self.publish_ai_preview("toplevel_destroyed");
    }

    /// 客户端更新 app_id 时，重新导出 AI 预览。
    fn app_id_changed(&mut self, _surface: ToplevelSurface) {
        self.publish_ai_preview("app_id_changed");
    }

    /// 客户端更新标题时，重新导出 AI 预览。
    fn title_changed(&mut self, _surface: ToplevelSurface) {
        self.publish_ai_preview("title_changed");
    }
}

impl SelectionHandler for NormaApp {
    type SelectionUserData = ();
}

impl DataDeviceHandler for NormaApp {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for NormaApp {}

impl ServerDndGrabHandler for NormaApp {
    fn send(&mut self, _mime_type: String, _fd: OwnedFd, _seat: Seat<Self>) {}
}

impl CompositorHandler for NormaApp {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client
            .get_data::<ClientState>()
            .expect("client state must be installed during insert_client")
            .compositor_state
    }

    /// surface commit 发生后，让 Smithay 先处理 buffer 生命周期，
    /// 再在受管窗口上触发布局刷新。
    ///
    /// 这样可以确保客户端第一次真正提交 buffer 后，布局与 configure 都是最新的。
    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        if self
            .wm_state
            .windows()
            .iter()
            .any(|window| window.surface.wl_surface() == surface)
        {
            self.wm_state.refresh();
        }
    }
}

impl ShmHandler for NormaApp {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl SeatHandler for NormaApp {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&WlSurface>) {}

    fn cursor_image(&mut self, _seat: &Seat<Self>, _image: CursorImageStatus) {}
}

#[derive(Default)]
struct ClientState {
    compositor_state: CompositorClientState,
}

/// 绑定到每个 Wayland client 的附加数据。
///
/// 当前只存 compositor client state，但未来也可以扩展成权限、统计或 AI 标记。
impl ClientData for ClientState {
    fn initialized(&self, client_id: ClientId) {
        info!(?client_id, "wayland client initialized");
    }

    fn disconnected(&self, client_id: ClientId, reason: DisconnectReason) {
        info!(?client_id, ?reason, "wayland client disconnected");
    }
}

/// 进程入口：初始化日志和 AI 通道，然后进入 winit backend 主循环。
fn main() -> Result<(), NormaError> {
    init_tracing();

    let (ai_nexus, _ai_handle) = AiNexus::channel();
    run_winit(ai_nexus)
}

/// 初始化全局 tracing subscriber。
///
/// 默认打开 `normawm` 和 `smithay` 的日志，便于在早期阶段直接观察协议与渲染行为。
fn init_tracing() {
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
fn run_winit(ai_nexus: AiNexus) -> Result<(), NormaError> {
    let mut display: Display<NormaApp> =
        Display::new().map_err(|error| NormaError::DisplayInit(error.to_string()))?;
    let dh = display.handle();
    let (mut backend, mut winit) = backend_winit::init::<GlesRenderer>()
        .map_err(|error| NormaError::WinitBackend(error.to_string()))?;
    let initial_size = to_logical_size(backend.window_size());

    let compositor_state = CompositorState::new::<NormaApp>(&dh);
    let shm_state = ShmState::new::<NormaApp>(&dh, vec![]);
    let mut seat_state = SeatState::new();
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
        data_device_state: DataDeviceState::new::<NormaApp>(&dh),
        seat,
        ai_nexus,
        clear_color: DEEP_GRAY,
        socket_name: socket_name.clone(),
        shutdown_requested: false,
        wm_state: TilingState::new(initial_size),
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
fn to_logical_size(size: Size<i32, smithay::utils::Physical>) -> Size<i32, Logical> {
    Size::from((size.w, size.h))
}

/// 把 frame callback 发送给整棵 surface tree。
///
/// 这是 Wayland 客户端继续下一帧渲染的重要节奏信号。
fn send_frames_surface_tree(surface: &wl_surface::WlSurface, time: u32) {
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

delegate_xdg_shell!(NormaApp);
delegate_compositor!(NormaApp);
delegate_shm!(NormaApp);
delegate_seat!(NormaApp);
delegate_data_device!(NormaApp);
