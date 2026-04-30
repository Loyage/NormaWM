use smithay::{
    utils::{Logical, Rectangle, Size},
    wayland::shell::xdg::ToplevelSurface,
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

    fn normalize_focus(&mut self) {
        self.focused = match self.windows.len() {
            0 => None,
            len => self.focused.filter(|index| *index < len).or(Some(len - 1)),
        };
    }

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
