//! 内建监控窗口。
//!
//! 这个窗口不是 Wayland client，也不参与普通窗口管理。
//! 它运行在宿主环境的独立 `winit` 事件循环里，负责显示：
//! - AI 接入层当前是否在线
//! - 最近一次状态更新原因
//! - 当前普通窗口数量与窗口摘要
//!
//! 由于它是 compositor 自己的诊断窗口，因此不计入 `wm_state`，
//! 也不会参与普通 tiling/focus 逻辑。

use std::{
    num::NonZeroU32,
    rc::Rc,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread,
    time::Duration,
};

use font8x8::{UnicodeFonts, BASIC_FONTS};
use softbuffer::{Context, Surface};
use tracing::warn;
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, NamedKey},
    window::{Window, WindowAttributes, WindowId},
};

const MONITOR_TITLE: &str = "NormaWM Monitor";
const MONITOR_SIZE: (f64, f64) = (720.0, 560.0);
const BACKGROUND: u32 = 0x00161b22;
const PANEL_TEXT: u32 = 0x00e6edf3;
const EMPHASIS_TEXT: u32 = 0x0066d9ef;
const LINE_HEIGHT: usize = 12;
const CHAR_WIDTH: usize = 8;
const PADDING_X: usize = 16;
const PADDING_Y: usize = 16;

#[derive(Debug, Clone)]
pub struct MonitorSnapshot {
    pub title: String,
    pub body: String,
}

#[derive(Clone)]
pub struct MonitorHandle {
    tx: Sender<MonitorSnapshot>,
}

impl MonitorHandle {
    /// 向监控窗口推送一份最新状态。
    ///
    /// 如果监控窗口已经关闭，这里静默忽略，避免影响 compositor 主循环。
    pub fn update(&self, snapshot: MonitorSnapshot) {
        let _ = self.tx.send(snapshot);
    }
}

/// 启动一个独立的宿主监控窗口线程，并返回更新句柄。
pub fn spawn_monitor_window() -> MonitorHandle {
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        if let Err(error) = run_monitor_window(rx) {
            warn!(%error, "failed to run monitor window");
        }
    });

    MonitorHandle { tx }
}

fn run_monitor_window(rx: Receiver<MonitorSnapshot>) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|error| error.to_string())?;
    let mut app = MonitorApp::new(rx);
    event_loop
        .run_app(&mut app)
        .map_err(|error| error.to_string())
}

struct MonitorApp {
    rx: Receiver<MonitorSnapshot>,
    window: Option<Rc<Window>>,
    context: Option<Context<Rc<Window>>>,
    surface: Option<Surface<Rc<Window>, Rc<Window>>>,
    current_snapshot: MonitorSnapshot,
}

impl MonitorApp {
    fn new(rx: Receiver<MonitorSnapshot>) -> Self {
        Self {
            rx,
            window: None,
            context: None,
            surface: None,
            current_snapshot: MonitorSnapshot {
                title: MONITOR_TITLE.to_string(),
                body: "waiting for compositor state...".to_string(),
            },
        }
    }

    fn drain_updates(&mut self) -> bool {
        let mut changed = false;

        loop {
            match self.rx.try_recv() {
                Ok(snapshot) => {
                    self.current_snapshot = snapshot;
                    changed = true;
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }

        changed
    }
}

impl ApplicationHandler for MonitorApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        event_loop.set_control_flow(ControlFlow::WaitUntil(
            std::time::Instant::now() + Duration::from_millis(120),
        ));

        let attributes: WindowAttributes = Window::default_attributes()
            .with_title(MONITOR_TITLE)
            .with_resizable(true)
            .with_inner_size(LogicalSize::new(MONITOR_SIZE.0, MONITOR_SIZE.1))
            .with_min_inner_size(LogicalSize::new(480.0, 320.0));

        let window = Rc::new(
            event_loop
                .create_window(attributes)
                .expect("failed to create monitor window"),
        );
        let context =
            Context::new(window.clone()).expect("failed to create monitor display context");
        let mut surface =
            Surface::new(&context, window.clone()).expect("failed to create monitor surface");

        resize_surface(
            &mut surface,
            window.inner_size().width,
            window.inner_size().height,
        );
        window.request_redraw();

        self.surface = Some(surface);
        self.context = Some(context);
        self.window = Some(window);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let changed = self.drain_updates();

        if let Some(window) = self.window.as_ref() {
            if changed {
                window.request_redraw();
            }
        }

        event_loop.set_control_flow(ControlFlow::WaitUntil(
            std::time::Instant::now() + Duration::from_millis(120),
        ));
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
                if event.state.is_pressed()
                    && matches!(event.logical_key, Key::Named(NamedKey::Escape))
                {
                    event_loop.exit();
                }
            }
            WindowEvent::RedrawRequested => {
                window.set_title(&self.current_snapshot.title);
                if let Some(surface) = self.surface.as_mut() {
                    render_monitor(surface, &self.current_snapshot);
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
        .expect("failed to resize monitor surface");
}

fn render_monitor<D, W>(surface: &mut Surface<D, W>, snapshot: &MonitorSnapshot)
where
    D: raw_window_handle::HasDisplayHandle,
    W: raw_window_handle::HasWindowHandle,
{
    let mut buffer = surface
        .buffer_mut()
        .expect("failed to acquire monitor frame buffer");

    let width = buffer.width().get() as usize;
    let height = buffer.height().get() as usize;
    buffer.fill(BACKGROUND);

    draw_text_line(
        &mut buffer,
        width,
        height,
        PADDING_X,
        PADDING_Y,
        &snapshot.title,
        EMPHASIS_TEXT,
    );

    for (index, line) in snapshot.body.lines().enumerate() {
        draw_text_line(
            &mut buffer,
            width,
            height,
            PADDING_X,
            PADDING_Y + LINE_HEIGHT * (index + 2),
            line,
            PANEL_TEXT,
        );
    }

    buffer.present().expect("failed to present monitor frame");
}

fn draw_text_line(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    start_x: usize,
    start_y: usize,
    text: &str,
    color: u32,
) {
    for (index, ch) in text.chars().enumerate() {
        draw_char(
            buffer,
            width,
            height,
            start_x + index * CHAR_WIDTH,
            start_y,
            ch,
            color,
        );
    }
}

fn draw_char(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    origin_x: usize,
    origin_y: usize,
    ch: char,
    color: u32,
) {
    let glyph = BASIC_FONTS.get(ch).unwrap_or([0; 8]);

    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..8 {
            if bits & (1 << col) == 0 {
                continue;
            }

            let x = origin_x + col;
            let y = origin_y + row;
            if x >= width || y >= height {
                continue;
            }

            buffer[y * width + x] = color;
        }
    }
}
