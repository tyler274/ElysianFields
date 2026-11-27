//! Native HTTP/1.1 TCP server hosting [`PublicSiteRouter`].
//!
//! Gated behind `#[cfg(not(target_arch = "wasm32"))]` so workspace `wasm32-unknown-unknown`
//! Clippy checks stay cleanly cross-compilable with zero new external dependencies.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::router::{PUBLIC_MAX_BODY_BYTES, PublicRequest, PublicResponse, PublicSiteRouter};

/// Standalone native TCP HTTP/1.1 server for [`PublicSiteRouter`].
pub struct TcpPublicSiteServer {
    addr: SocketAddr,
    shutdown_flag: Arc<AtomicBool>,
    router: Arc<Mutex<PublicSiteRouter>>,
    accept_thread: Option<JoinHandle<()>>,
}

impl TcpPublicSiteServer {
    /// Binds a real TCP listener on `bind_addr` (e.g. `"127.0.0.1:8082"` or `"127.0.0.1:0"`).
    pub fn bind(bind_addr: &str, router: PublicSiteRouter) -> std::io::Result<Self> {
        let listener = TcpListener::bind(bind_addr)?;
        let addr = listener.local_addr()?;
        let shutdown_flag = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Mutex::new(router));

        let flag = Arc::clone(&shutdown_flag);
        let shared_router = Arc::clone(&shared);
        let accept_thread = thread::spawn(move || {
            for stream_res in listener.incoming() {
                if flag.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream_res else {
                    continue;
                };
                handle_tcp_connection(stream, &shared_router);
            }
        });

        Ok(Self {
            addr,
            shutdown_flag,
            router: shared,
            accept_thread: Some(accept_thread),
        })
    }

    /// Returns the actual bound local socket address.
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Returns a handle to the shared [`PublicSiteRouter`].
    #[must_use]
    pub fn router(&self) -> Arc<Mutex<PublicSiteRouter>> {
        Arc::clone(&self.router)
    }

    /// Gracefully stops the listener loop and joins the accept thread.
    pub fn shutdown(mut self) {
        self.shutdown_flag.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(handle) = self.accept_thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for TcpPublicSiteServer {
    fn drop(&mut self) {
        self.shutdown_flag.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(handle) = self.accept_thread.take() {
            let _ = handle.join();
        }
    }
}

enum ReadOutcome {
    Request { req: PublicRequest, is_head: bool },
    PayloadTooLarge,
    BadRequest,
    Closed,
}

fn handle_tcp_connection(mut stream: TcpStream, router: &Arc<Mutex<PublicSiteRouter>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));

    let client_ip = stream
        .peer_addr()
        .map_or_else(|_| "127.0.0.1".to_owned(), |a| a.ip().to_string());

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));

    let (response, is_head) = match read_request(&stream, client_ip) {
        ReadOutcome::Closed => return,
        ReadOutcome::BadRequest => (simple_text_response(400, "Bad Request"), false),
        ReadOutcome::PayloadTooLarge => (
            simple_text_response(413, "Payload Too Large (64 KiB limit exceeded)"),
            false,
        ),
        ReadOutcome::Request { req, is_head } => {
            let resp = match router.lock() {
                Ok(mut guard) => guard.handle(&req, now_ms),
                Err(_) => simple_text_response(500, "Internal Server Error"),
            };
            (resp, is_head)
        }
    };

    let wire = serialize_http1(&response, is_head);
    let _ = stream.write_all(&wire);
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Write);

    // Drain any unread request bytes (e.g., rejected 413 body) so Linux closes cleanly with FIN.
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let mut drain = [0u8; 4096];
    let mut drained = 0usize;
    while drained < 256 * 1024 {
        match stream.read(&mut drain) {
            Ok(0) | Err(_) => break,
            Ok(n) => drained = drained.saturating_add(n),
        }
    }
}

fn read_request(stream: &TcpStream, client_ip: String) -> ReadOutcome {
    let Ok(cloned) = stream.try_clone() else {
        return ReadOutcome::Closed;
    };
    let mut reader = BufReader::new(cloned);

    let mut request_line = String::new();
    match reader.read_line(&mut request_line) {
        Ok(0) | Err(_) => return ReadOutcome::Closed,
        Ok(_) => {}
    }

    let trimmed = request_line.trim_end_matches(['\r', '\n']);
    let mut parts = trimmed.split_whitespace();
    let Some(raw_method) = parts.next() else {
        return ReadOutcome::BadRequest;
    };
    let Some(target) = parts.next() else {
        return ReadOutcome::BadRequest;
    };
    let is_head = raw_method == "HEAD";
    let method = if is_head { "GET" } else { raw_method };

    let mut headers = BTreeMap::new();
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let header_line = line.trim_end_matches(['\r', '\n']);
        if header_line.is_empty() {
            break;
        }
        if let Some((k, v)) = header_line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }

    let content_length = match headers.get("content-length") {
        None => 0usize,
        Some(raw) => match raw.parse::<usize>() {
            Ok(n) => n,
            Err(_) => return ReadOutcome::BadRequest,
        },
    };

    if content_length > PUBLIC_MAX_BODY_BYTES {
        return ReadOutcome::PayloadTooLarge;
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 && reader.read_exact(&mut body).is_err() {
        return ReadOutcome::BadRequest;
    }

    ReadOutcome::Request {
        req: PublicRequest {
            method: method.to_owned(),
            path_and_query: target.to_owned(),
            client_ip,
            body,
        },
        is_head,
    }
}

fn simple_text_response(status: u16, body: &str) -> PublicResponse {
    let mut headers = BTreeMap::new();
    headers.insert("Cache-Control".to_owned(), "no-store".to_owned());
    headers.insert("CDN-Cache-Control".to_owned(), "no-store".to_owned());
    headers.insert("X-Content-Type-Options".to_owned(), "nosniff".to_owned());
    PublicResponse {
        status,
        content_type: "text/plain; charset=utf-8",
        headers,
        body: body.to_owned(),
    }
}

fn serialize_http1(resp: &PublicResponse, suppress_body: bool) -> Vec<u8> {
    let body_bytes = resp.body.as_bytes();
    let mut head = format!(
        "HTTP/1.1 {} {}\r\n",
        resp.status,
        status_reason(resp.status)
    );
    let _ = write!(head, "Content-Type: {}\r\n", resp.content_type);
    for (k, v) in &resp.headers {
        let _ = write!(head, "{k}: {v}\r\n");
    }
    let _ = write!(
        head,
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body_bytes.len()
    );
    let mut out = head.into_bytes();
    if !suppress_body {
        out.extend_from_slice(body_bytes);
    }
    out
}

const fn status_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        _ => "Unknown",
    }
}
