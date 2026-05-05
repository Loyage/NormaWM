//! 最小窗口管理层。
//!
//! 这个模块目前实现的是一个非常保守的纵向平铺布局：
//! - 维护受管 `xdg_toplevel` 列表
//! - 维护当前输出尺寸和焦点索引
//! - 负责把几何与激活状态写回到客户端
//! - 负责把窗口状态整理成 AI 可消费的摘要
//!
//! 它的目标不是一开始就做完整 WM，而是先把“布局状态”和“渲染/协议层”分开。

use crate::ai::{AiWindowDigest, AiWindowRecord};
use smithay::{
    utils::{Logical, Rectangle, Size},
    wayland::{
        compositor::with_states,
        shell::xdg::{ToplevelSurface, XdgToplevelSurfaceData},
    },
};
use wayland_server::protocol::wl_surface::WlSurface;

const OUTER_GAP: i32 = 24;
const INNER_GAP: i32 = 16;
const MIN_TILE_SIZE: i32 = 96;

/// 已经接入布局系统的一个顶层窗口。
///
/// 这里保存的是：
/// - Wayland 层的 surface 句柄
/// - 布局计算出的逻辑坐标系几何
#[derive(Debug, Clone)]
pub struct ManagedToplevel {
    pub surface: ToplevelSurface,
    pub geometry: Rectangle<i32, Logical>,
}

/// 当前最小平铺窗口管理器的内部状态。
///
/// `focused` 存的是索引而不是 surface 句柄，是为了让布局重排和摘要导出更直接。
#[derive(Debug)]
pub struct TilingState {
    output_size: Size<i32, Logical>,
    windows: Vec<ManagedToplevel>,
    focused: Option<usize>,
}

/// 与 Wayland 资源弱耦合的窗口布局快照。
///
/// 它是 `ToplevelSurface` 到 AI 摘要之间的桥梁，
/// 可以被测试代码手工构造，也可以由真实运行时导出。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowLayoutSnapshot {
    pub window_id: String,
    pub role: &'static str,
    pub title: Option<String>,
    pub app_id: Option<String>,
    pub focused: bool,
    pub geometry: Rectangle<i32, Logical>,
}

/// 把窗口管理层的布局快照转换成 AI 可消费的摘要数据。
///
/// 这个函数只依赖纯几何和焦点信息，不依赖真实 Wayland client，
/// 因此既适合单元/集成测试，也适合作为后续接入 AI 模块前的中间层。
pub fn build_ai_window_digest_from_layout(
    workspace: impl Into<String>,
    output_size: Size<i32, Logical>,
    windows: &[WindowLayoutSnapshot],
) -> AiWindowDigest {
    AiWindowDigest {
        workspace: workspace.into(),
        output_width: output_size.w,
        output_height: output_size.h,
        window_count: windows.len(),
        windows: windows
            .iter()
            .map(|window| AiWindowRecord {
                window_id: window.window_id.clone(),
                role: window.role,
                title: window.title.clone(),
                app_id: window.app_id.clone(),
                focused: window.focused,
                x: window.geometry.loc.x,
                y: window.geometry.loc.y,
                width: window.geometry.size.w,
                height: window.geometry.size.h,
            })
            .collect(),
    }
}

impl TilingState {
    /// 用初始输出尺寸创建窗口管理状态。
    pub fn new(output_size: Size<i32, Logical>) -> Self {
        Self {
            output_size,
            windows: Vec::new(),
            focused: None,
        }
    }

    /// 当前受管窗口数量。
    pub fn len(&self) -> usize {
        self.windows.len()
    }

    /// 只读访问当前受管窗口集合，主要给渲染层使用。
    pub fn windows(&self) -> &[ManagedToplevel] {
        &self.windows
    }

    /// 取出当前焦点窗口的 `wl_surface`，方便输入系统同步键盘焦点。
    pub fn focused_surface(&self) -> Option<WlSurface> {
        self.focused
            .and_then(|index| self.windows.get(index))
            .map(|window| window.surface.wl_surface().clone())
    }

    /// 将焦点移动到第一个窗口。
    ///
    /// 这个操作会同时刷新客户端的 activated 状态，因此不仅仅是改一个索引。
    pub fn focus_first(&mut self) -> Option<WlSurface> {
        if self.windows.is_empty() {
            self.focused = None;
            return None;
        }

        if self.focused == Some(0) {
            return self.focused_surface();
        }

        self.focused = Some(0);
        self.configure_windows();
        self.focused_surface()
    }

    /// 把某个已有的 toplevel 设为当前焦点窗口。
    ///
    /// 找不到对应窗口时返回 `false`，这样调用方可以决定是否需要继续上报或重试。
    pub fn focus_toplevel(&mut self, surface: &ToplevelSurface) -> bool {
        let Some(index) = self
            .windows
            .iter()
            .position(|window| &window.surface == surface)
        else {
            return false;
        };

        if self.focused == Some(index) {
            return false;
        }

        self.focused = Some(index);
        self.configure_windows();
        true
    }

    /// 更新输出尺寸，并在尺寸变化时触发布局重排。
    pub fn set_output_size(&mut self, output_size: Size<i32, Logical>) -> bool {
        if self.output_size == output_size {
            return false;
        }

        self.output_size = output_size;
        self.relayout();
        self.configure_windows();
        true
    }

    /// 把一个新创建的 `xdg_toplevel` 纳入窗口管理。
    ///
    /// 当前策略很简单：新窗口加入末尾，并默认抢占焦点。
    pub fn insert_toplevel(&mut self, surface: ToplevelSurface) {
        if self.windows.iter().any(|window| window.surface == surface) {
            return;
        }

        self.windows.push(ManagedToplevel {
            surface,
            geometry: Rectangle::from_size((0, 0).into()),
        });
        self.focused = Some(self.windows.len() - 1);
        self.relayout();
        self.configure_windows();
    }

    /// 从窗口管理状态中移除一个已销毁或不再受管的窗口。
    pub fn remove_toplevel(&mut self, surface: &ToplevelSurface) -> bool {
        let old_len = self.windows.len();
        self.windows.retain(|window| &window.surface != surface);

        if self.windows.len() == old_len {
            return false;
        }

        self.normalize_focus();
        self.relayout();
        self.configure_windows();
        true
    }

    /// 清理已经失效的 Wayland 窗口句柄。
    ///
    /// 之所以单独保留这个步骤，是因为客户端可能先断开，再等 compositor 下一轮主循环处理。
    pub fn prune_dead_windows(&mut self) -> bool {
        let focused_surface = self.focused_surface();
        let old_len = self.windows.len();
        self.windows.retain(|window| window.surface.alive());

        if self.windows.len() == old_len {
            return false;
        }

        self.focused = focused_surface.and_then(|focused| {
            self.windows
                .iter()
                .position(|window| window.surface.wl_surface() == &focused)
        });
        self.normalize_focus();
        self.relayout();
        self.configure_windows();
        true
    }

    /// 强制重排并重新下发 configure。
    ///
    /// 适合处理“布局语义变化但窗口集合本身没变”的场景，例如 focus 或输出变化。
    pub fn refresh(&mut self) {
        self.relayout();
        self.configure_windows();
    }

    /// 从当前 `TilingState` 中导出一份 AI 摘要。
    ///
    /// 这里刻意不暴露底层 `ToplevelSurface` 给 AI，而是只输出窗口编号、
    /// 焦点和几何信息，保持 compositor 内部可变状态与 AI 输入边界解耦。
    pub fn build_ai_window_digest(&self, workspace: &str) -> AiWindowDigest {
        let windows = self
            .windows
            .iter()
            .enumerate()
            .map(|(index, window)| WindowLayoutSnapshot {
                window_id: format!("window-{}", index + 1),
                role: "xdg_toplevel",
                title: window_title(&window.surface),
                app_id: window_app_id(&window.surface),
                focused: self.focused == Some(index),
                geometry: window.geometry,
            })
            .collect::<Vec<_>>();

        build_ai_window_digest_from_layout(workspace, self.output_size, &windows)
    }

    /// 在窗口集合变化后，把焦点索引修正到合法范围。
    fn normalize_focus(&mut self) {
        self.focused = match self.windows.len() {
            0 => None,
            len => self.focused.filter(|index| *index < len).or(Some(len - 1)),
        };
    }

    /// 依据当前输出尺寸，把所有窗口重新排成最小纵向平铺布局。
    ///
    /// 现在的策略是单列堆叠：所有窗口共享宽度，按顺序向下排列，
    /// 并保留 outer/inner gap。这样实现简单，但已经足够验证
    /// `xdg_toplevel` 的 configure、focus 和 render offset 路径。
    fn relayout(&mut self) {
        if self.windows.is_empty() {
            return;
        }

        let usable_width = (self.output_size.w - OUTER_GAP * 2).max(MIN_TILE_SIZE);
        let usable_height = (self.output_size.h - OUTER_GAP * 2).max(MIN_TILE_SIZE);
        let window_count = self.windows.len() as i32;
        let total_gaps = INNER_GAP.saturating_mul(window_count.saturating_sub(1));
        let tile_height = ((usable_height - total_gaps).max(MIN_TILE_SIZE)) / window_count.max(1);
        let tile_height = tile_height.max(MIN_TILE_SIZE);

        let mut next_y = OUTER_GAP;
        let remainder = (usable_height - total_gaps - tile_height * window_count).max(0);

        for (index, window) in self.windows.iter_mut().enumerate() {
            let extra_row = i32::from((index as i32) < remainder);
            let height = (tile_height + extra_row).max(MIN_TILE_SIZE);
            window.geometry =
                Rectangle::new((OUTER_GAP, next_y).into(), (usable_width, height).into());
            next_y += height + INNER_GAP;
        }
    }

    /// 将计算好的几何和焦点状态写回到每个 `xdg_toplevel` 的 pending state。
    ///
    /// 这一步本质上是在告诉客户端：“你应该按这个大小显示，并且谁处于激活态”。
    /// 真正下发时使用 `send_pending_configure()`，避免没有变化时重复发送 configure。
    fn configure_windows(&self) {
        let bounds = Some(self.output_size);

        for (index, window) in self.windows.iter().enumerate() {
            window.surface.with_pending_state(|state| {
                state.size = Some(window.geometry.size);
                state.bounds = bounds;

                if self.focused == Some(index) {
                    state
                        .states
                        .set(wayland_protocols::xdg::shell::server::xdg_toplevel::State::Activated);
                } else {
                    state.states.unset(
                        wayland_protocols::xdg::shell::server::xdg_toplevel::State::Activated,
                    );
                }
            });
            window.surface.send_pending_configure();
        }
    }
}

/// 从 `xdg_toplevel` 角色属性里读取客户端当前设置的标题。
///
/// 标题不属于几何状态，因此不能从 `current_state()` 拿到，需要直接访问 role data。
fn window_title(surface: &ToplevelSurface) -> Option<String> {
    with_states(surface.wl_surface(), |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .and_then(|attributes| attributes.lock().ok().and_then(|guard| guard.title.clone()))
    })
}

/// 从 `xdg_toplevel` 角色属性里读取客户端当前设置的 app_id。
fn window_app_id(surface: &ToplevelSurface) -> Option<String> {
    with_states(surface.wl_surface(), |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .and_then(|attributes| {
                attributes
                    .lock()
                    .ok()
                    .and_then(|guard| guard.app_id.clone())
            })
    })
}
