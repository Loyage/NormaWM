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

#[derive(Debug, Clone)]
pub struct ManagedToplevel {
    pub surface: ToplevelSurface,
    pub geometry: Rectangle<i32, Logical>,
}

#[derive(Debug)]
pub struct TilingState {
    output_size: Size<i32, Logical>,
    windows: Vec<ManagedToplevel>,
    focused: Option<usize>,
}

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
    pub fn new(output_size: Size<i32, Logical>) -> Self {
        Self {
            output_size,
            windows: Vec::new(),
            focused: None,
        }
    }

    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn windows(&self) -> &[ManagedToplevel] {
        &self.windows
    }

    pub fn focused_surface(&self) -> Option<WlSurface> {
        self.focused
            .and_then(|index| self.windows.get(index))
            .map(|window| window.surface.wl_surface().clone())
    }

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

    pub fn set_output_size(&mut self, output_size: Size<i32, Logical>) -> bool {
        if self.output_size == output_size {
            return false;
        }

        self.output_size = output_size;
        self.relayout();
        self.configure_windows();
        true
    }

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

fn window_title(surface: &ToplevelSurface) -> Option<String> {
    with_states(surface.wl_surface(), |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .and_then(|attributes| attributes.lock().ok().and_then(|guard| guard.title.clone()))
    })
}

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
