//! Native HTTP/1.1 TCP server hosting [`PublicSiteRouter`].
//!
//! Gated behind `#[cfg(not(target_arch = "wasm32"))]` so workspace `wasm32-unknown-unknown`
//! Clippy checks stay cleanly cross-compilable with zero new external dependencies.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ef_core::{
    AccessLogRingBuffer, DEFAULT_ACCESS_LOG_CAPACITY, HttpRequestId, REQUEST_ID_HEADER,
    ShutdownSignal, ZeroPhiAccessEvent, ZeroPhiRouteTemplate,
};

use crate::router::{PUBLIC_MAX_BODY_BYTES, PublicRequest, PublicResponse, PublicSiteRouter};

/// Standalone native TCP HTTP/1.1 server for [`PublicSiteRouter`].
pub struct TcpPublicSiteServer {
    addr: SocketAddr,
    shutdown: ShutdownSignal,
    router: Arc<Mutex<PublicSiteRouter>>,
    access_logs: AccessLogRingBuffer,
    accept_thread: Option<JoinHandle<()>>,
}

impl TcpPublicSiteServer {
    /// Binds a real TCP listener on `bind_addr` (e.g. `"127.0.0.1:8082"` or `"127.0.0.1:0"`).
    pub fn bind(bind_addr: &str, router: PublicSiteRouter) -> std::io::Result<Self> {
        Self::bind_with_shutdown(bind_addr, router, ShutdownSignal::new())
    }

    /// Binds a real TCP listener on `bind_addr` controlled by a caller-supplied [`ShutdownSignal`].
    pub fn bind_with_shutdown(
        bind_addr: &str,
        router: PublicSiteRouter,
        shutdown: ShutdownSignal,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind(bind_addr)?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let shared = Arc::new(Mutex::new(router));
        let access_logs = AccessLogRingBuffer::from_env(DEFAULT_ACCESS_LOG_CAPACITY);

        let flag = shutdown.clone();
        let shared_router = Arc::clone(&shared);
        let logs = access_logs.clone();
        let accept_thread = thread::spawn(move || {
            while !flag.is_triggered() {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        if flag.is_triggered() {
                            break;
                        }
                        handle_tcp_connection(stream, &shared_router, &logs);
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
            router: shared,
            access_logs,
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

impl Drop for TcpPublicSiteServer {
    fn drop(&mut self) {
        self.shutdown.trigger();
        if let Some(handle) = self.accept_thread.take() {
            let _ = handle.join();
        }
    }
}

enum ReadOutcome {
    Request { req: PublicRequest, is_head: bool },
    PayloadTooLarge { method: String, target: String },
    BadRequest { method: String, target: String },
    Closed,
}

fn handle_tcp_connection(
    mut stream: TcpStream,
    router: &Arc<Mutex<PublicSiteRouter>>,
    access_logs: &AccessLogRingBuffer,
) {
    let started = Instant::now();
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));

    let client_ip = stream
        .peer_addr()
        .map_or_else(|_| "127.0.0.1".to_owned(), |a| a.ip().to_string());

    let now_duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let now_ms = u64::try_from(now_duration.as_millis()).unwrap_or(u64::MAX);
    let timestamp_secs = now_duration.as_secs();

    let (response, is_head, method_label, raw_target, request_id) =
        match read_request(&stream, client_ip) {
            ReadOutcome::Closed => return,
            ReadOutcome::BadRequest { method, target } => {
                let req_id = HttpRequestId::parse_or_generate(None, "public");
                let mut resp = simple_text_response(400, "Bad Request");
                resp.headers
                    .insert(REQUEST_ID_HEADER.to_owned(), req_id.as_str().to_owned());
                (resp, false, method, target, req_id)
            }
            ReadOutcome::PayloadTooLarge { method, target } => {
                let req_id = HttpRequestId::parse_or_generate(None, "public");
                let mut resp =
                    simple_text_response(413, "Payload Too Large (64 KiB limit exceeded)");
                resp.headers
                    .insert(REQUEST_ID_HEADER.to_owned(), req_id.as_str().to_owned());
                (resp, false, method, target, req_id)
            }
            ReadOutcome::Request { req, is_head } => {
                let method_label = if is_head {
                    "HEAD".to_owned()
                } else {
                    req.method.clone()
                };
                let raw_target = req.path_and_query.clone();
                let incoming_id = req.header(REQUEST_ID_HEADER).map(str::to_owned);
                let mut resp = match router.lock() {
                    Ok(mut guard) => guard.handle(&req, now_ms),
                    Err(_) => simple_text_response(500, "Internal Server Error"),
                };
                let req_id = resp
                    .headers
                    .get(REQUEST_ID_HEADER)
                    .and_then(|v| HttpRequestId::parse(v))
                    .unwrap_or_else(|| {
                        let id = HttpRequestId::parse_or_generate(incoming_id.as_deref(), "public");
                        resp.headers
                            .insert(REQUEST_ID_HEADER.to_owned(), id.as_str().to_owned());
                        id
                    });
                (resp, is_head, method_label, raw_target, req_id)
            }
        };

    let status = response.status;
    let wire = serialize_http1(&response, is_head);
    let _ = stream.write_all(&wire);
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Write);

    let latency_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    access_logs.record(ZeroPhiAccessEvent {
        timestamp_secs,
        service: "ef-public-site".to_owned(),
        request_id,
        method: method_label,
        route_template: ZeroPhiRouteTemplate::from_target(&raw_target),
        status,
        latency_us,
    });

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
        return ReadOutcome::BadRequest {
            method: "UNKNOWN".to_owned(),
            target: "/".to_owned(),
        };
    };
    let Some(target) = parts.next() else {
        return ReadOutcome::BadRequest {
            method: raw_method.to_owned(),
            target: "/".to_owned(),
        };
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
            Err(_) => {
                return ReadOutcome::BadRequest {
                    method: raw_method.to_owned(),
                    target: target.to_owned(),
                };
            }
        },
    };

    if content_length > PUBLIC_MAX_BODY_BYTES {
        return ReadOutcome::PayloadTooLarge {
            method: raw_method.to_owned(),
            target: target.to_owned(),
        };
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 && reader.read_exact(&mut body).is_err() {
        return ReadOutcome::BadRequest {
            method: raw_method.to_owned(),
            target: target.to_owned(),
        };
    }

    ReadOutcome::Request {
        req: PublicRequest {
            method: method.to_owned(),
            path_and_query: target.to_owned(),
            client_ip,
            headers,
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
