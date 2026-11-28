//! Native HTTP/1.1 TCP server binding [`std::net::TcpListener`] to [`ClinicService`].
//!
//! Gated behind `#[cfg(not(target_arch = "wasm32"))]` so workspace `wasm32-unknown-unknown`
//! Clippy checks pass cleanly without pulling in native socket runtime crates.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ef_core::{
    AccessLogRingBuffer, DEFAULT_ACCESS_LOG_CAPACITY, HttpRequestId, REQUEST_ID_HEADER,
    ShutdownSignal, ZeroPhiAccessEvent, ZeroPhiRouteTemplate,
};

use crate::http::{
    AUTH_MAX_BODY_BYTES, COMMANDS_MAX_BODY_BYTES, HttpMethod, HttpRequest, HttpResponse,
};
use crate::service::ClinicService;

/// Native TCP HTTP/1.1 + SSE server hosting [`ClinicService`].
pub struct TcpClinicServer {
    addr: SocketAddr,
    shutdown: ShutdownSignal,
    service: Arc<Mutex<ClinicService>>,
    access_logs: AccessLogRingBuffer,
    accept_thread: Option<JoinHandle<()>>,
}

impl TcpClinicServer {
    /// Binds a real TCP listener on `bind_addr` (e.g. `"127.0.0.1:0"`) and starts serving.
    pub fn bind(bind_addr: &str, service: ClinicService) -> std::io::Result<Self> {
        Self::bind_with_shutdown(bind_addr, service, ShutdownSignal::new())
    }

    /// Binds a real TCP listener on `bind_addr` controlled by a caller-supplied [`ShutdownSignal`].
    pub fn bind_with_shutdown(
        bind_addr: &str,
        service: ClinicService,
        shutdown: ShutdownSignal,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind(bind_addr)?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let shared_service = Arc::new(Mutex::new(service));
        let access_logs = AccessLogRingBuffer::from_env(DEFAULT_ACCESS_LOG_CAPACITY);

        let thread_shutdown = shutdown.clone();
        let thread_service = Arc::clone(&shared_service);
        let thread_logs = access_logs.clone();
        let accept_thread = thread::spawn(move || {
            while !thread_shutdown.is_triggered() {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        if thread_shutdown.is_triggered() {
                            break;
                        }
                        handle_tcp_connection(stream, &thread_service, &thread_logs);
                    }
                    Err(err) if err.kind() == ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => {
                        thread::sleep(Duration::from_millis(5));
                    }
                }
            }
        });

        Ok(Self {
            addr,
            shutdown,
            service: shared_service,
            access_logs,
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

    /// Returns a cloneable [`ShutdownSignal`] that gracefully stops this server's accept loop.
    #[must_use]
    pub fn shutdown_signal(&self) -> ShutdownSignal {
        self.shutdown.clone()
    }

    /// Returns a snapshot of recorded zero-PHI structured JSON access events.
    #[must_use]
    pub fn access_logs(&self) -> Vec<ZeroPhiAccessEvent> {
        self.access_logs.snapshot()
    }

    /// Returns `true` while the accept thread is still running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.accept_thread
            .as_ref()
            .is_some_and(|handle| !handle.is_finished())
    }

    /// Gracefully stops the listener loop and joins the accept thread.
    pub fn shutdown(mut self) {
        self.shutdown.trigger();
        if let Some(handle) = self.accept_thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for TcpClinicServer {
    fn drop(&mut self) {
        self.shutdown.trigger();
        if let Some(handle) = self.accept_thread.take() {
            let _ = handle.join();
        }
    }
}

enum ParseOutcome {
    Request(HttpRequest),
    ImmediateResponse {
        resp: HttpResponse,
        method: String,
        target: String,
    },
    Closed,
}

fn handle_tcp_connection(
    mut stream: TcpStream,
    service: &Arc<Mutex<ClinicService>>,
    access_logs: &AccessLogRingBuffer,
) {
    let started = Instant::now();
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let peer_ip = stream
        .peer_addr()
        .map_or_else(|_| "127.0.0.1".to_owned(), |addr| addr.ip().to_string());

    let parse_result = read_http_request(&stream);
    let (response, is_head, method_label, raw_target, request_id) = match parse_result {
        ParseOutcome::Closed => return,
        ParseOutcome::ImmediateResponse {
            mut resp,
            method,
            target,
        } => {
            let req_id = HttpRequestId::parse_or_generate(None, "clinic");
            resp.set_header(REQUEST_ID_HEADER, req_id.as_str());
            (resp, false, method, target, req_id)
        }
        ParseOutcome::Request(req) => {
            let is_head = req.method == HttpMethod::Head;
            let method_label = req.method.as_str().to_owned();
            let raw_target = req.target.clone();
            let incoming_id = req.header(REQUEST_ID_HEADER).map(str::to_owned);
            let mut resp = match service.lock() {
                Ok(mut guard) => guard.handle_request_from_ip(&req, &peer_ip),
                Err(_) => HttpResponse::json_clinical(
                    500,
                    &serde_json::json!({ "error": "service_lock_poisoned" }),
                ),
            };
            let req_id = resp
                .header(REQUEST_ID_HEADER)
                .and_then(HttpRequestId::parse)
                .unwrap_or_else(|| {
                    let id = HttpRequestId::parse_or_generate(incoming_id.as_deref(), "clinic");
                    resp.set_header(REQUEST_ID_HEADER, id.as_str());
                    id
                });
            (resp, is_head, method_label, raw_target, req_id)
        }
    };

    let status = response.status;
    let wire_bytes = response.to_http1_bytes(is_head);
    let _ = stream.write_all(&wire_bytes);
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Write);

    let timestamp_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let latency_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    access_logs.record(ZeroPhiAccessEvent {
        timestamp_secs,
        service: "ef-clinic-server".to_owned(),
        request_id,
        method: method_label,
        route_template: ZeroPhiRouteTemplate::from_target(&raw_target),
        status,
        latency_us,
    });

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
        return ParseOutcome::ImmediateResponse {
            resp: HttpResponse::bad_request("missing HTTP method"),
            method: "UNKNOWN".to_owned(),
            target: "/".to_owned(),
        };
    };
    let Some(target) = parts.next() else {
        return ParseOutcome::ImmediateResponse {
            resp: HttpResponse::bad_request("missing HTTP target"),
            method: raw_method.to_owned(),
            target: "/".to_owned(),
        };
    };
    let Some(method) = HttpMethod::parse(raw_method) else {
        return ParseOutcome::ImmediateResponse {
            resp: HttpResponse::method_not_allowed(),
            method: raw_method.to_owned(),
            target: target.to_owned(),
        };
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
                return ParseOutcome::ImmediateResponse {
                    resp: HttpResponse::bad_request("invalid Content-Length header"),
                    method: method.as_str().to_owned(),
                    target: target.to_owned(),
                };
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
        return ParseOutcome::ImmediateResponse {
            resp: HttpResponse::payload_too_large(max_body),
            method: method.as_str().to_owned(),
            target: target.to_owned(),
        };
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 && reader.read_exact(&mut body).is_err() {
        return ParseOutcome::ImmediateResponse {
            resp: HttpResponse::bad_request("truncated HTTP request body"),
            method: method.as_str().to_owned(),
            target: target.to_owned(),
        };
    }

    ParseOutcome::Request(HttpRequest {
        method,
        target: target.to_owned(),
        headers,
        body,
    })
}
