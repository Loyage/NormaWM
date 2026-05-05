//! 最小测试客户端。
//!
//! 这个二进制不是 compositor 本体，而是一个专门用来连接 `NormaWM`
//! 的 Wayland 客户端。它提供：
//! - 一个简单顶层窗口
//! - 纯色背景绘制
//! - 周期性标题更新
//! - 手动按键触发颜色/标题变化
//!
//! 目标是帮助人工验证：
//! 1. `xdg_toplevel` 生命周期是否正常
//! 2. redraw / resize 路径是否正常
//! 3. `NormaWM` 的 AI 预览是否会随窗口元数据变化而刷新

use std::{
    num::NonZeroU32,
    rc::Rc,
    time::{Duration, Instant},
};

use softbuffer::{Context, Surface};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, NamedKey},
    window::{Window, WindowAttributes, WindowId},
};

const WINDOW_TITLE: &str = "NormaWM Test Window";
const WINDOW_SIZE: (f64, f64) = (640.0, 360.0);
const TITLE_INTERVAL: Duration = Duration::from_millis(900);
const TITLE_VARIANTS: [&str; 4] = ["teal", "amber", "cobalt", "mint"];
const BACKGROUND_COLORS: [u32; 4] = [0x001c7c72, 0x00c98c20, 0x002d5bd1, 0x0029a36a];

/// 测试窗口的全部本地状态。
///
/// 这里不追求抽象复杂度，而是尽量把窗口、渲染 surface、标题状态和颜色状态
/// 明确地放在一起，方便手工调试。
#[derive(Default)]
struct TestWindowApp {
    window: Option<Rc<Window>>,
    context: Option<Context<Rc<Window>>>,
    surface: Option<Surface<Rc<Window>, Rc<Window>>>,
    title_index: usize,
    color_index: usize,
    next_title_change: Option<Instant>,
}

impl ApplicationHandler for TestWindowApp {
    /// 当事件循环恢复时创建窗口和软件渲染 surface。
    ///
    /// `winit` 在 0.30 以后推荐通过 `ApplicationHandler` 驱动应用生命周期，
    /// 所以窗口创建也放在 `resumed` 里，而不是旧式的同步 `Window::new`。
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + TITLE_INTERVAL));

        let attributes: WindowAttributes = Window::default_attributes()
            .with_title(WINDOW_TITLE)
            .with_resizable(true)
            .with_inner_size(LogicalSize::new(WINDOW_SIZE.0, WINDOW_SIZE.1))
            .with_min_inner_size(LogicalSize::new(320.0, 180.0));

        let window = Rc::new(
            event_loop
                .create_window(attributes)
                .expect("failed to create test window"),
        );
        let context =
            Context::new(window.clone()).expect("failed to create softbuffer display context");
        let mut surface =
            Surface::new(&context, window.clone()).expect("failed to create softbuffer surface");

        resize_surface(
            &mut surface,
            window.inner_size().width,
            window.inner_size().height,
        );
        window.set_title(&format_title(self.title_index));
        window.request_redraw();

        self.next_title_change = Some(Instant::now() + TITLE_INTERVAL);
        self.surface = Some(surface);
        self.context = Some(context);
        self.window = Some(window);
    }

    /// 在没有新事件时推进定时标题变化。
    ///
    /// 这样测试窗口即使没有用户输入，也会周期性改变 title，
    /// 便于观察 `NormaWM` 是否把 title 更新传播到了 AI 预览。
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(window) = self.window.as_ref() else {
            return;
        };

        let now = Instant::now();
        let next_change = self
            .next_title_change
            .get_or_insert_with(|| now + TITLE_INTERVAL);

        if now >= *next_change {
            advance_title(window, &mut self.title_index);
            *next_change = now + TITLE_INTERVAL;
        }

        event_loop.set_control_flow(ControlFlow::WaitUntil(*next_change));
    }

    /// 处理窗口关闭、尺寸变化、快捷键和重绘请求。
    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self.window.as_ref() else {
            return;
        };

        if window.id() != window_id {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                // 宿主窗口尺寸变化后，先同步软件缓冲区，再请求重绘，
                // 否则新的窗口大小下可能仍然显示旧尺寸内容。
                if let Some(surface) = self.surface.as_mut() {
                    resize_surface(surface, size.width, size.height);
                }
                window.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state.is_pressed() {
                    match &event.logical_key {
                        // `Esc` 直接退出，方便快速关闭测试窗口。
                        Key::Named(NamedKey::Escape) => event_loop.exit(),
                        Key::Named(NamedKey::Space) => {
                            // `Space` 手动切换背景色，用于肉眼验证：
                            // 1. 客户端是否真的请求了 redraw
                            // 2. compositor 是否把新的内容渲染了出来
                            self.color_index = (self.color_index + 1) % BACKGROUND_COLORS.len();
                            window.request_redraw();
                        }
                        Key::Character(ch) if ch.eq_ignore_ascii_case("t") => {
                            // `T` 立即推进标题状态。
                            // 这条路径主要用于验证 `title_changed` 是否会反映到
                            // `NormaWM` 的 AI 预览输出里。
                            advance_title(window, &mut self.title_index);
                        }
                        _ => {}
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                // 只要收到 redraw 请求，就把当前选中的纯色重新刷到整个窗口。
                if let Some(surface) = self.surface.as_mut() {
                    render(surface, BACKGROUND_COLORS[self.color_index]);
                }
            }
            _ => {}
        }
    }
}

/// 按窗口大小变化同步软件缓冲区尺寸。
///
/// `softbuffer` 的 surface 大小需要手动更新，否则重绘时会使用旧尺寸。
fn resize_surface<D, W>(surface: &mut Surface<D, W>, width: u32, height: u32)
where
    D: raw_window_handle::HasDisplayHandle,
    W: raw_window_handle::HasWindowHandle,
{
    let width = NonZeroU32::new(width.max(1)).expect("non-zero width");
    let height = NonZeroU32::new(height.max(1)).expect("non-zero height");
    surface
        .resize(width, height)
        .expect("failed to resize softbuffer surface");
}

/// 用一个纯色填满整个窗口。
///
/// 这里故意不引入更复杂的 2D 绘图库，只保留最小可见变化，
/// 让“窗口是否重绘”和“颜色是否改变”一眼可见。
fn render<D, W>(surface: &mut Surface<D, W>, background_color: u32)
where
    D: raw_window_handle::HasDisplayHandle,
    W: raw_window_handle::HasWindowHandle,
{
    let mut buffer = surface
        .buffer_mut()
        .expect("failed to fetch mutable softbuffer frame");
    buffer.fill(background_color);
    buffer
        .present()
        .expect("failed to present softbuffer frame");
}

/// 手动推进一次标题状态，用于验证 compositor 是否接收到了 title 更新。
///
/// 和定时器驱动的标题轮换共用同一条逻辑，这样键盘快捷键与自动更新
/// 不会分叉出两套行为。
fn advance_title(window: &Window, title_index: &mut usize) {
    *title_index = (*title_index + 1) % TITLE_VARIANTS.len();
    window.set_title(&format_title(*title_index));
    window.request_redraw();
}

/// 把当前标题索引渲染成实际窗口标题字符串。
fn format_title(index: usize) -> String {
    format!(
        "{WINDOW_TITLE} • {} • tick {}",
        TITLE_VARIANTS[index],
        index + 1
    )
}

/// 测试客户端入口。
fn main() {
    let event_loop = EventLoop::new().expect("failed to create winit event loop");
    let mut app = TestWindowApp::default();
    // 这里把应用完全交给 `winit` 驱动。
    // 之后的所有窗口创建、定时标题更新和快捷键响应都从 handler 回调进入。
    event_loop
        .run_app(&mut app)
        .expect("test window exited cleanly");
}
