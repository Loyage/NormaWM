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
                if let Some(surface) = self.surface.as_mut() {
                    resize_surface(surface, size.width, size.height);
                }
                window.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state.is_pressed() {
                    match &event.logical_key {
                        Key::Named(NamedKey::Escape) => event_loop.exit(),
                        Key::Named(NamedKey::Space) => {
                            self.color_index = (self.color_index + 1) % BACKGROUND_COLORS.len();
                            window.request_redraw();
                        }
                        Key::Character(ch) if ch.eq_ignore_ascii_case("t") => {
                            advance_title(window, &mut self.title_index);
                        }
                        _ => {}
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(surface) = self.surface.as_mut() {
                    render(surface, BACKGROUND_COLORS[self.color_index]);
                }
            }
            _ => {}
        }
    }
}

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

fn format_title(index: usize) -> String {
    format!(
        "{WINDOW_TITLE} • {} • tick {}",
        TITLE_VARIANTS[index],
        index + 1
    )
}

fn main() {
    let event_loop = EventLoop::new().expect("failed to create winit event loop");
    let mut app = TestWindowApp::default();
    event_loop
        .run_app(&mut app)
        .expect("test window exited cleanly");
}
