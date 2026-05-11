//! NormaWM command line control client.

use std::{
    env,
    io::{self, ErrorKind, Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    process,
    time::Duration,
};

const CONTROL_SOCKET_NAME: &str = "normawm-control.sock";

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() {
        return Err(usage());
    }

    let request = build_request(&args)?;
    let response = send_request(&request)?;
    print_response(&response)
}

struct Request {
    header: String,
    payload: Option<String>,
}

fn build_request(args: &[String]) -> Result<Request, String> {
    match args.first().map(String::as_str) {
        Some("msg") => build_msg_request(&args[1..]),
        Some("ctl") => build_ctl_request(&args[1..]),
        _ => Err(usage()),
    }
}

fn build_msg_request(args: &[String]) -> Result<Request, String> {
    match args.first().map(String::as_str) {
        Some("status") => line_request("MSG_STATUS"),
        Some("windows") => line_request("MSG_WINDOWS"),
        Some("workspaces") => line_request("MSG_WORKSPACES"),
        Some("focused-window") => line_request("MSG_FOCUSED_WINDOW"),
        _ => Err("usage: norma msg <status|windows|workspaces|focused-window>".to_string()),
    }
}

fn build_ctl_request(args: &[String]) -> Result<Request, String> {
    match args.first().map(String::as_str) {
        Some("focus") => {
            let window = parse_window_arg(&args[1..])?
                .ok_or_else(|| "usage: norma ctl focus --window <window-id>".to_string())?;
            line_request(format!("FOCUS_WINDOW {window}"))
        }
        Some("workspace") => {
            let Some(workspace) = args.get(1) else {
                return Err("usage: norma ctl workspace <0..9>".to_string());
            };
            line_request(format!("WORKSPACE {workspace}"))
        }
        Some("launch") => {
            if args.len() < 2 {
                return Err("usage: norma ctl launch <program> [args...]".to_string());
            }
            line_request(format!("LAUNCH {}", args[1..].join(" ")))
        }
        Some("ai") => match args.get(1).map(String::as_str) {
            Some("pause") => line_request("AI_PAUSE"),
            Some("resume") => line_request("AI_RESUME"),
            Some("cancel") => line_request("AI_CANCEL"),
            _ => Err("usage: norma ctl ai <pause|resume|cancel>".to_string()),
        },
        Some("shutdown") => line_request("SHUTDOWN"),
        Some("input") => build_input_request(&args[1..]),
        _ => Err("usage: norma ctl <focus|workspace|launch|ai|shutdown|input> ...".to_string()),
    }
}

fn build_input_request(args: &[String]) -> Result<Request, String> {
    let mut target = None;
    let mut use_stdin = false;
    let mut text_parts = Vec::new();
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--window" => {
                let Some(window) = args.get(index + 1) else {
                    return Err("--window requires a window id".to_string());
                };
                target = Some(window.clone());
                index += 2;
            }
            "--stdin" => {
                use_stdin = true;
                index += 1;
            }
            value => {
                text_parts.push(value.to_string());
                index += 1;
            }
        }
    }

    let text = if use_stdin {
        let mut text = String::new();
        io::stdin()
            .read_to_string(&mut text)
            .map_err(|error| format!("failed to read stdin: {error}"))?;
        text
    } else {
        text_parts.join(" ")
    };

    if text.is_empty() {
        return Err("input text cannot be empty".to_string());
    }

    let target = target.unwrap_or_else(|| "focused".to_string());
    Ok(Request {
        header: format!("INPUT_TEXT {} {}", target, text.len()),
        payload: Some(text),
    })
}

fn parse_window_arg(args: &[String]) -> Result<Option<String>, String> {
    if args.is_empty() {
        return Ok(None);
    }

    if args.first().map(String::as_str) != Some("--window") || args.len() != 2 {
        return Err("expected --window <window-id>".to_string());
    }

    Ok(args.get(1).cloned())
}

fn line_request(line: impl Into<String>) -> Result<Request, String> {
    Ok(Request {
        header: line.into(),
        payload: None,
    })
}

fn send_request(request: &Request) -> Result<String, String> {
    let path = control_socket_path();
    let mut stream = UnixStream::connect(&path)
        .map_err(|error| format!("failed to connect {}: {error}", path.display()))?;
    stream
        .set_read_timeout(Some(Duration::from_millis(800)))
        .map_err(|error| format!("failed to configure socket timeout: {error}"))?;

    writeln!(stream, "{}", request.header)
        .map_err(|error| format!("failed to write request: {error}"))?;
    if let Some(payload) = &request.payload {
        stream
            .write_all(payload.as_bytes())
            .map_err(|error| format!("failed to write request payload: {error}"))?;
    }
    stream
        .flush()
        .map_err(|error| format!("failed to flush request: {error}"))?;

    let mut response = Vec::new();
    let mut buffer = [0; 4096];

    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(bytes) => {
                response.extend_from_slice(&buffer[..bytes]);
                if response_complete(&response) {
                    break;
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) =>
            {
                if response.is_empty() {
                    return Err("timed out waiting for compositor response".to_string());
                }
                break;
            }
            Err(error) => return Err(format!("failed to read response: {error}")),
        }
    }

    String::from_utf8(response).map_err(|_| "compositor returned invalid UTF-8".to_string())
}

fn control_socket_path() -> PathBuf {
    env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join(CONTROL_SOCKET_NAME)
}

fn response_complete(response: &[u8]) -> bool {
    let text = String::from_utf8_lossy(response);
    text.starts_with("RESULT ") || text.contains("\nEND_STATUS\n") || text.contains("\nEND_TEXT\n")
}

fn print_response(response: &str) -> Result<(), String> {
    let response = response.trim_end();

    if let Some(message) = response.strip_prefix("RESULT ok ") {
        println!("{message}");
        Ok(())
    } else if let Some(message) = response.strip_prefix("RESULT err ") {
        Err(message.to_string())
    } else if let Some(text) = response
        .strip_prefix("BEGIN_TEXT\n")
        .and_then(|text| text.strip_suffix("\nEND_TEXT"))
    {
        println!("{text}");
        Ok(())
    } else {
        println!("{response}");
        Ok(())
    }
}

fn usage() -> String {
    [
        "usage:",
        "  norma msg status",
        "  norma msg windows",
        "  norma msg workspaces",
        "  norma msg focused-window",
        "  norma ctl focus --window <window-id>",
        "  norma ctl workspace <0..9>",
        "  norma ctl launch <program> [args...]",
        "  norma ctl ai <pause|resume|cancel>",
        "  norma ctl input [--window <window-id>] <text>",
        "  norma ctl input [--window <window-id>] --stdin",
        "  norma ctl shutdown",
    ]
    .join("\n")
}
