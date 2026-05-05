//! `NormaWM` 的 compositor 状态与 Wayland handler 实现。
//!
//! 这个模块只关心“系统当前是什么状态”以及“收到协议/输入事件后如何更新状态”。
//! 它不负责驱动主循环本身；主循环放在 `runtime.rs`。

use std::{fs, os::unix::io::OwnedFd};

use crate::{
    ai::{format_ai_window_digest, ActionResult, AiEvent, AiNexus, CompositorSnapshot},
    wm::TilingState,
};
use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    delegate_compositor, delegate_data_device, delegate_seat, delegate_shm, delegate_xdg_shell,
    input::{pointer::CursorImageStatus, Seat, SeatHandler, SeatState},
    reexports::wayland_server::protocol::wl_seat,
    utils::Serial,
    wayland::{
        buffer::BufferHandler,
        compositor::{CompositorClientState, CompositorHandler, CompositorState},
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
    protocol::{wl_buffer, wl_surface::WlSurface},
    Client,
};

pub const DEEP_GRAY: [f32; 4] = [0.13, 0.13, 0.14, 1.0];
pub const AI_PREVIEW_PATH: &str = "target/ai-input-preview.txt";

/// 整个 compositor 在运行时的聚合状态。
///
/// 这是 Smithay 各个 handler trait 共享的中心对象：
/// 图形、输入、Wayland、AI 预览与窗口管理状态都从这里读取。
pub struct NormaApp {
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub seat_state: SeatState<Self>,
    pub data_device_state: DataDeviceState,
    pub seat: Seat<Self>,
    pub ai_nexus: AiNexus,
    pub clear_color: [f32; 4],
    pub socket_name: String,
    pub shutdown_requested: bool,
    pub wm_state: TilingState,
}

impl NormaApp {
    /// 生成一份面向外部观察者的轻量快照。
    ///
    /// 它不包含窗口逐项信息，主要用来响应简单状态查询。
    pub fn snapshot(&self) -> CompositorSnapshot {
        CompositorSnapshot {
            backend: "winit",
            socket_name: self.socket_name.clone(),
            workspace: "main",
            toplevel_count: self.wm_state.len(),
            clear_color: self.clear_color,
        }
    }

    /// 基于当前窗口管理状态生成一段完整的 AI 输入预览。
    pub fn build_ai_preview(&self) -> String {
        let digest = self.wm_state.build_ai_window_digest("main");
        format_ai_window_digest(&digest)
    }

    /// 在窗口状态变化时向“AI 前向输入”观察面发布最新预览。
    ///
    /// 当前有两个观察出口：
    /// 1. 通过 `AiEvent::PromptPreview` 发给外部接入端
    /// 2. 同步打印到终端并写入固定文件，便于人工检查
    pub fn publish_ai_preview(&self, reason: &str) {
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
pub struct ClientState {
    pub compositor_state: CompositorClientState,
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

delegate_xdg_shell!(NormaApp);
delegate_compositor!(NormaApp);
delegate_shm!(NormaApp);
delegate_seat!(NormaApp);
delegate_data_device!(NormaApp);
