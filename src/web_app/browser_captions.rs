//! Local bridge for a Chrome extension reading standard HTML video text tracks.

use serde::Deserialize;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const ADDRESS: &str = "127.0.0.1:47836";

#[derive(Default)]
pub(super) struct BrowserCaptionState {
    pub(super) text: String,
    pub(super) has_track: bool,
    pub(super) updated: Option<Instant>,
}

impl BrowserCaptionState {
    pub(super) fn active(&self) -> bool {
        self.has_track
            && self
                .updated
                .is_some_and(|time| time.elapsed() < Duration::from_secs(3))
    }
}

#[derive(Deserialize)]
struct CaptionPayload {
    text: String,
    has_track: bool,
    playing: bool,
}

pub(super) fn start_bridge(state: Arc<Mutex<BrowserCaptionState>>) {
    std::thread::spawn(move || {
        let listener = match TcpListener::bind(ADDRESS) {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("Chrome 字幕连接无法监听 {ADDRESS}：{error}");
                return;
            }
        };
        for connection in listener.incoming() {
            match connection {
                Ok(mut stream) => handle_connection(&mut stream, &state),
                Err(error) => eprintln!("Chrome 字幕连接失败：{error}"),
            }
        }
    });
}

fn handle_connection(stream: &mut TcpStream, state: &Arc<Mutex<BrowserCaptionState>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let mut data = Vec::with_capacity(1024);
    let mut buffer = [0_u8; 1024];
    while data.len() < 8192 && !data.windows(4).any(|part| part == b"\r\n\r\n") {
        let Ok(count) = stream.read(&mut buffer) else {
            return;
        };
        if count == 0 {
            return;
        }
        data.extend_from_slice(&buffer[..count]);
    }
    let Some(header_end) = data.windows(4).position(|part| part == b"\r\n\r\n") else {
        return;
    };
    let Ok(header) = std::str::from_utf8(&data[..header_end]) else {
        return;
    };
    let header = header.to_owned();
    let origin = header
        .lines()
        .find_map(|line| {
            line.strip_prefix("Origin: ")
                .or_else(|| line.strip_prefix("origin: "))
        })
        .unwrap_or("")
        .trim();
    // Browser pages cannot forge a chrome-extension Origin. Restrict the bridge
    // to extensions and never expose the listener beyond loopback.
    if !origin.starts_with("chrome-extension://") || origin.len() > 100 {
        respond(stream, "403 Forbidden", "", "");
        return;
    }
    if header.starts_with("OPTIONS /caption ") {
        respond(stream, "204 No Content", origin, "");
        return;
    }
    if !header.starts_with("POST /caption ") {
        respond(stream, "404 Not Found", origin, "");
        return;
    }
    let length = header
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length: ")?
                .trim()
                .parse::<usize>()
                .ok()
        })
        .unwrap_or(0);
    if length == 0 || length > 4096 {
        respond(stream, "413 Payload Too Large", origin, "");
        return;
    }
    let body_start = header_end + 4;
    while data.len() - body_start < length {
        let Ok(count) = stream.read(&mut buffer) else {
            return;
        };
        if count == 0 {
            return;
        }
        data.extend_from_slice(&buffer[..count]);
    }
    let Ok(payload) =
        serde_json::from_slice::<CaptionPayload>(&data[body_start..body_start + length])
    else {
        respond(stream, "400 Bad Request", origin, "");
        return;
    };
    if payload.text.len() > 2000 {
        respond(stream, "413 Payload Too Large", origin, "");
        return;
    }
    if let Ok(mut current) = state.lock() {
        current.text = payload.text;
        current.has_track = payload.has_track && payload.playing;
        current.updated = Some(Instant::now());
    }
    respond(stream, "200 OK", origin, "ok");
}

fn respond(stream: &mut TcpStream, status: &str, origin: &str, body: &str) {
    let reply = format!(
        "HTTP/1.1 {status}\r\nAccess-Control-Allow-Origin: {origin}\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(reply.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::{BrowserCaptionState, handle_connection};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    #[test]
    fn stale_browser_captions_do_not_override_audio_recognition() {
        let state = BrowserCaptionState::default();
        assert!(!state.active());
    }

    #[test]
    fn accepts_extension_caption_and_rejects_webpage_origin() {
        let state = Arc::new(Mutex::new(BrowserCaptionState::default()));
        for (origin, accepted) in [
            ("chrome-extension://test", true),
            ("https://example.com", false),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let shared = state.clone();
            let worker = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                handle_connection(&mut stream, &shared);
            });
            let mut stream = TcpStream::connect(address).unwrap();
            let body = r#"{"text":"你好","has_track":true,"playing":true}"#;
            write!(stream, "POST /caption HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: {origin}\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
            let mut reply = String::new();
            // Windows may reset the socket after a rejected request whose body
            // remains unread; any response bytes already received are enough.
            let _ = stream.read_to_string(&mut reply);
            worker.join().unwrap();
            assert_eq!(reply.starts_with("HTTP/1.1 200"), accepted);
        }
        assert!(state.lock().unwrap().active());
        assert_eq!(state.lock().unwrap().text, "你好");
    }
}
