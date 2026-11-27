//! Native HTTP/1.1 TCP server binding [`std::net::TcpListener`] to [`ClinicService`].
//!
//! Gated behind `#[cfg(not(target_arch = "wasm32"))]` so workspace `wasm32-unknown-unknown`
//! Clippy checks pass cleanly without pulling in native socket runtime crates.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::http::{
    AUTH_MAX_BODY_BYTES, COMMANDS_MAX_BODY_BYTES, HttpMethod, HttpRequest, HttpResponse,
};
use crate::service::ClinicService;

/// Native TCP HTTP/1.1 + SSE server hosting [`ClinicService`].
pub struct TcpClinicServer {
    addr: SocketAddr,
    shutdown_flag: Arc<AtomicBool>,
    service: Arc<Mutex<ClinicService>>,
    accept_thread: Option<JoinHandle<()>>,
}

impl TcpClinicServer {
    /// Binds a real TCP listener on `bind_addr` (e.g. `"127.0.0.1:0"`) and starts serving.
    pub fn bind(bind_addr: &str, service: ClinicService) -> std::io::Result<Self> {
        let listener = TcpListener::bind(bind_addr)?;
        let addr = listener.local_addr()?;
        let shutdown_flag = Arc::new(AtomicBool::new(false));
        let shared_service = Arc::new(Mutex::new(service));

        let thread_shutdown = Arc::clone(&shutdown_flag);
        let thread_service = Arc::clone(&shared_service);
        let accept_thread = thread::spawn(move || {
            for stream_res in listener.incoming() {
                if thread_shutdown.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream_res else {
                    continue;
                };
                handle_tcp_connection(stream, &thread_service);
            }
        });

        Ok(Self {
            addr,
            shutdown_flag,
            service: shared_service,
            accept_thread: Some(accept_thread),
        })
    }

    /// Returns the actual bound local socket address.
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Returns a handle to the shared [`ClinicService`] (for test fixtures / clock control).
    #[must_use]
    pub fn service(&self) -> Arc<Mutex<ClinicService>> {
        Arc::clone(&self.service)
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

impl Drop for TcpClinicServer {
    fn drop(&mut self) {
        self.shutdown_flag.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(handle) = self.accept_thread.take() {
            let _ = handle.join();
        }
    }
}

enum ParseOutcome {
    Request(HttpRequest),
    ImmediateResponse(HttpResponse),
    Closed,
}

fn handle_tcp_connection(mut stream: TcpStream, service: &Arc<Mutex<ClinicService>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));

    let parse_result = read_http_request(&stream);
    let (response, is_head) = match parse_result {
        ParseOutcome::Closed => return,
        ParseOutcome::ImmediateResponse(resp) => (resp, false),
        ParseOutcome::Request(req) => {
            let is_head = req.method == HttpMethod::Head;
            let resp = match service.lock() {
                Ok(mut guard) => guard.handle_request(&req),
                Err(_) => HttpResponse::json_clinical(
                    500,
                    &serde_json::json!({ "error": "service_lock_poisoned" }),
                ),
            };
            (resp, is_head)
        }
    };

    let wire_bytes = response.to_http1_bytes(is_head);
    let _ = stream.write_all(&wire_bytes);
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Write);
    // Drain any unread request bytes (e.g. rejected 413 payload) so closing the socket
    // sends a clean FIN rather than a TCP RST that discards the HTTP response.
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

fn read_http_request(stream: &TcpStream) -> ParseOutcome {
    let Ok(cloned) = stream.try_clone() else {
        return ParseOutcome::Closed;
    };
    let mut reader = BufReader::new(cloned);

    let mut request_line = String::new();
    match reader.read_line(&mut request_line) {
        Ok(0) | Err(_) => return ParseOutcome::Closed,
        Ok(_) => {}
    }

    let trimmed = request_line.trim_end_matches(['\r', '\n']);
    let mut parts = trimmed.split_whitespace();
    let Some(raw_method) = parts.next() else {
        return ParseOutcome::ImmediateResponse(HttpResponse::bad_request("missing HTTP method"));
    };
    let Some(target) = parts.next() else {
        return ParseOutcome::ImmediateResponse(HttpResponse::bad_request("missing HTTP target"));
    };
    let Some(method) = HttpMethod::parse(raw_method) else {
        return ParseOutcome::ImmediateResponse(HttpResponse::method_not_allowed());
    };

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
        if let Some((name, value)) = header_line.split_once(':') {
            let lower = name.trim().to_ascii_lowercase();
            // Strip incoming Nix-store mtime validators per Chapter 4 §4.1.
            if lower == "if-modified-since" || lower == "if-unmodified-since" {
                continue;
            }
            headers.insert(lower, value.trim().to_owned());
        }
    }

    let content_length = match headers.get("content-length") {
        None => 0usize,
        Some(raw) => match raw.parse::<usize>() {
            Ok(len) => len,
            Err(_) => {
                return ParseOutcome::ImmediateResponse(HttpResponse::bad_request(
                    "invalid Content-Length header",
                ));
            }
        },
    };

    let path_only = target.split_once('?').map_or(target, |(p, _)| p);
    let max_body = if path_only.starts_with("/elysianfields/api/auth/") {
        AUTH_MAX_BODY_BYTES
    } else {
        COMMANDS_MAX_BODY_BYTES
    };
    if content_length > max_body {
        return ParseOutcome::ImmediateResponse(HttpResponse::payload_too_large(max_body));
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 && reader.read_exact(&mut body).is_err() {
        return ParseOutcome::ImmediateResponse(HttpResponse::bad_request(
            "truncated HTTP request body",
        ));
    }

    ParseOutcome::Request(HttpRequest {
        method,
        target: target.to_owned(),
        headers,
        body,
    })
}
