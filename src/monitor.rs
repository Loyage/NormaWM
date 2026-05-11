//! 独立的人类控制面窗口。
//!
//! 这个窗口运行在 `normawm-control` 进程里，连接 compositor 暴露的
//! Unix socket。它不是 NormaWM 的 Wayland client，因此不会进入普通
//! tiling/focus/AI 管理路径。

use std::{
    io::{ErrorKind, Read, Write},
    num::NonZeroU32,
    os::unix::net::UnixStream,
    rc::Rc,
    time::{Duration, Instant},
};

use font8x8::{UnicodeFonts, BASIC_FONTS};
use softbuffer::{Context, Surface};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, NamedKey},
    window::{Window, WindowAttributes, WindowId},
};

use crate::control::control_socket_path;

const CONTROL_TITLE: &str = "NormaWM Human Control";
const CONTROL_SIZE: (f64, f64) = (760.0, 600.0);
const BACKGROUND: u32 = 0x00161b22;
const PANEL_TEXT: u32 = 0x00e6edf3;
const EMPHASIS_TEXT: u32 = 0x0066d9ef;
const RESULT_TEXT: u32 = 0x00a7f3d0;
const ERROR_TEXT: u32 = 0x00ff7b72;
const LINE_HEIGHT: usize = 12;
const CHAR_WIDTH: usize = 8;
const PADDING_X: usize = 16;
const PADDING_Y: usize = 16;

pub fn run_control_panel() -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|error| error.to_string())?;
    let mut app = ControlPanelApp::new();
    event_loop
        .run_app(&mut app)
        .map_err(|error| error.to_string())
}

struct ControlPanelApp {
    window: Option<Rc<Window>>,
    context: Option<Context<Rc<Window>>>,
    surface: Option<Surface<Rc<Window>, Rc<Window>>>,
    stream: Option<UnixStream>,
    incoming: String,
    status_lines: Vec<String>,
    result_line: String,
    collecting_status: bool,
    last_connect_attempt: Instant,
    last_status_request: Instant,
    ai_paused: bool,
}

impl ControlPanelApp {
    fn new() -> Self {
        Self {
            window: None,
            context: None,
            surface: None,
            stream: None,
            incoming: String::new(),
            status_lines: vec!["waiting for compositor control socket...".to_string()],
            result_line: String::new(),
            collecting_status: false,
            last_connect_attempt: Instant::now() - Duration::from_secs(5),
            last_status_request: Instant::now() - Duration::from_secs(5),
            ai_paused: false,
        }
    }

    fn tick(&mut self) -> bool {
        let mut changed = false;

        if self.stream.is_none() && self.last_connect_attempt.elapsed() >= Duration::from_secs(1) {
            self.last_connect_attempt = Instant::now();
            changed |= self.connect();
        }

        if self.stream.is_some() && self.last_status_request.elapsed() >= Duration::from_secs(1) {
            self.last_status_request = Instant::now();
            self.send_command("STATUS");
        }

        changed | self.drain_socket()
    }

    fn connect(&mut self) -> bool {
        match UnixStream::connect(control_socket_path()) {
            Ok(stream) => {
                if stream.set_nonblocking(true).is_err() {
                    self.result_line = "failed to make control socket nonblocking".to_string();
                    return true;
                }

                self.stream = Some(stream);
                self.result_line = "connected to NormaWM control socket".to_string();
                self.send_command("STATUS");
                true
            }
            Err(error) => {
                self.status_lines = vec![
                    "NormaWM control socket is not available.".to_string(),
                    format!("path: {}", control_socket_path().display()),
                    format!("last error: {error}"),
                ];
                true
            }
        }
    }

    fn send_command(&mut self, command: &str) {
        let Some(stream) = self.stream.as_mut() else {
            self.result_line = "not connected to compositor".to_string();
            return;
        };

        if writeln!(stream, "{command}")
            .and_then(|_| stream.flush())
            .is_err()
        {
            self.stream = None;
            self.result_line = "lost compositor control connection".to_string();
        }
    }

    fn drain_socket(&mut self) -> bool {
        let Some(stream) = self.stream.as_mut() else {
            return false;
        };

        let mut changed = false;
        let mut buffer = [0; 4096];
        let mut complete_lines = Vec::new();

        loop {
            match stream.read(&mut buffer) {
                Ok(0) => {
                    self.stream = None;
                    self.result_line = "compositor control socket closed".to_string();
                    return true;
                }
                Ok(bytes) => {
                    let chunk = String::from_utf8_lossy(&buffer[..bytes]);
                    self.incoming.push_str(&chunk);
                    changed = true;

                    while let Some(newline) = self.incoming.find('\n') {
                        let line = self.incoming[..newline].trim_end().to_string();
                        self.incoming.drain(..=newline);
                        complete_lines.push(line);
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) => {
                    self.stream = None;
                    self.result_line = format!("control socket error: {error}");
                    return true;
                }
            }
        }

        for line in complete_lines {
            self.handle_line(line);
        }

        changed
    }

    fn handle_line(&mut self, line: String) {
        match line.as_str() {
            "BEGIN_STATUS" => {
                self.status_lines.clear();
                self.collecting_status = true;
            }
            "END_STATUS" => {
                self.collecting_status = false;
                self.ai_paused = self
                    .status_lines
                    .iter()
                    .any(|line| line.trim() == "ai_paused: true");
            }
            _ if line.starts_with("RESULT ") => {
                self.result_line = line;
            }
            _ if self.collecting_status => {
                self.status_lines.push(line);
            }
            _ => {}
        }
    }

    fn render_snapshot(&self) -> RenderSnapshot {
        let mut body = vec![
            format!("control socket: {}", control_socket_path().display()),
            "keys: R refresh | F focus first | P pause/resume AI | C cancel AI | T test window | Q shutdown | Esc close panel".to_string(),
            String::new(),
        ];
        body.extend(self.status_lines.iter().cloned());

        RenderSnapshot {
            title: CONTROL_TITLE.to_string(),
            body,
            result: self.result_line.clone(),
            result_is_error: self.result_line.contains(" err ")
                || self.result_line.contains("error")
                || self.result_line.contains("failed")
                || self.result_line.contains("lost"),
        }
    }
}

impl ApplicationHandler for ControlPanelApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        event_loop.set_control_flow(ControlFlow::WaitUntil(
            std::time::Instant::now() + Duration::from_millis(120),
        ));

        let attributes: WindowAttributes = Window::default_attributes()
            .with_title(CONTROL_TITLE)
            .with_resizable(true)
            .with_inner_size(LogicalSize::new(CONTROL_SIZE.0, CONTROL_SIZE.1))
            .with_min_inner_size(LogicalSize::new(520.0, 360.0));

        let window = Rc::new(
            event_loop
                .create_window(attributes)
                .expect("failed to create control window"),
        );
        let context =
            Context::new(window.clone()).expect("failed to create control display context");
        let mut surface =
            Surface::new(&context, window.clone()).expect("failed to create control surface");

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
        let changed = self.tick();

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
        let Some(active_window_id) = self.window.as_ref().map(|window| window.id()) else {
            return;
        };

        if active_window_id != window_id {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(surface) = self.surface.as_mut() {
                    resize_surface(surface, size.width, size.height);
                }
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state.is_pressed() => {
                match event.logical_key {
                    Key::Named(NamedKey::Escape) => event_loop.exit(),
                    Key::Character(ref ch) if ch.eq_ignore_ascii_case("r") => {
                        self.send_command("STATUS")
                    }
                    Key::Character(ref ch) if ch.eq_ignore_ascii_case("f") => {
                        self.send_command("FOCUS_FIRST")
                    }
                    Key::Character(ref ch) if ch.eq_ignore_ascii_case("p") => {
                        if self.ai_paused {
                            self.send_command("AI_RESUME");
                        } else {
                            self.send_command("AI_PAUSE");
                        }
                    }
                    Key::Character(ref ch) if ch.eq_ignore_ascii_case("c") => {
                        self.send_command("AI_CANCEL")
                    }
                    Key::Character(ref ch) if ch.eq_ignore_ascii_case("t") => {
                        self.send_command("LAUNCH cargo run --bin test_window")
                    }
                    Key::Character(ref ch) if ch.eq_ignore_ascii_case("q") => {
                        self.send_command("SHUTDOWN")
                    }
                    _ => {}
                }
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(window) = self.window.as_ref() {
                    window.set_title(CONTROL_TITLE);
                }
                let snapshot = self.render_snapshot();
                if let Some(surface) = self.surface.as_mut() {
                    render_monitor(surface, &snapshot);
                }
            }
            _ => {}
        }
    }
}

struct RenderSnapshot {
    title: String,
    body: Vec<String>,
    result: String,
    result_is_error: bool,
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
        .expect("failed to resize control surface");
}

fn render_monitor<D, W>(surface: &mut Surface<D, W>, snapshot: &RenderSnapshot)
where
    D: raw_window_handle::HasDisplayHandle,
    W: raw_window_handle::HasWindowHandle,
{
    let mut buffer = surface
        .buffer_mut()
        .expect("failed to acquire control frame buffer");

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

    let result_color = if snapshot.result_is_error {
        ERROR_TEXT
    } else {
        RESULT_TEXT
    };
    draw_text_line(
        &mut buffer,
        width,
        height,
        PADDING_X,
        PADDING_Y + LINE_HEIGHT * 2,
        &snapshot.result,
        result_color,
    );

    for (index, line) in snapshot.body.iter().enumerate() {
        draw_text_line(
            &mut buffer,
            width,
            height,
            PADDING_X,
            PADDING_Y + LINE_HEIGHT * (index + 4),
            line,
            PANEL_TEXT,
        );
    }

    buffer.present().expect("failed to present control frame");
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
    let max_chars = width.saturating_sub(start_x) / CHAR_WIDTH;

    for (index, ch) in text.chars().take(max_chars).enumerate() {
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
