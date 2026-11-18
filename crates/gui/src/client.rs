use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::state::{parse_status, UiState};

pub const CONTROL_PORT: u16 = 18904;

/// A raw probe because gpui's executor is not tokio; a blocking socket check
/// avoids dragging a runtime into the UI thread.
pub fn is_healthy(port: u16) -> bool {
    let Ok(addr) = format!("127.0.0.1:{port}").parse::<SocketAddr>() else {
        return false;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(400)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(1_200)));
    if stream
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut buffer = String::new();
    let _ = stream.read_to_string(&mut buffer);
    buffer.starts_with("HTTP/1.1 200") || buffer.starts_with("HTTP/1.0 200")
}

/// The daemon binary shipped next to the GUI inside the `.app`.
pub fn sibling_bin(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let candidate = exe.parent()?.join(name);
    if candidate.exists() {
        Some(candidate)
    } else {
        None
    }
}

/// Start the daemon if it is not already answering. The child is detached, so
/// closing the window never stops it.
pub fn ensure_daemon(bin: &Path, port: u16) -> bool {
    if is_healthy(port) {
        return true;
    }
    let _ = Command::new(bin)
        .arg("serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if is_healthy(port) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

pub fn poll_once(
    client: &reqwest::blocking::Client,
    base: &str,
    token: Option<&str>,
) -> UiState {
    let get = |path: &str| -> Option<String> {
        let mut request = client.get(format!("{base}{path}"));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        match request.send() {
            Ok(response) if response.status().is_success() => response.text().ok(),
            _ => None,
        }
    };

    let Some(status) = get("/api/status") else {
        return UiState {
            connected: false,
            error: Some("control API unreachable".to_string()),
            ..UiState::default()
        };
    };
    let keys = get("/api/keys").unwrap_or_else(|| "[]".to_string());
    parse_status(&status, &keys)
}

pub fn spawn_poller(state: Arc<Mutex<UiState>>, base: String, token: Option<String>) {
    std::thread::spawn(move || {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(4))
            .build()
            .ok();
        loop {
            if let Some(client) = client.as_ref() {
                let snapshot = poll_once(client, &base, token.as_deref());
                if let Ok(mut guard) = state.lock() {
                    *guard = snapshot;
                }
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn a_served_healthz_endpoint_is_healthy() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buffer = [0u8; 512];
                let _ = stream.read(&mut buffer);
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                );
            }
        });
        assert!(is_healthy(port));
    }

    #[test]
    fn a_closed_port_is_not_healthy() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert!(!is_healthy(port));
    }

    #[test]
    fn an_unreachable_control_api_reports_disconnected() {
        let client = reqwest::blocking::Client::new();
        let state = poll_once(&client, "http://127.0.0.1:1", None);
        assert!(!state.connected);
        assert!(state.error.is_some());
    }
}
