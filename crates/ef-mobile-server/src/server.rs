//! Native HTTP/1.1 TCP server hosting [`MobileService`].

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ef_core::{
    AccessLogRingBuffer, DEFAULT_ACCESS_LOG_CAPACITY, HttpRequestId, REQUEST_ID_HEADER,
    ShutdownSignal, ZeroPhiAccessEvent, ZeroPhiRouteTemplate,
};

use crate::http::{MOBILE_MAX_BODY_BYTES, MobileHttpMethod, MobileHttpRequest, MobileHttpResponse};
use crate::service::MobileService;

pub struct TcpMobileServer {
    addr: SocketAddr,
    shutdown: ShutdownSignal,
    service: Arc<Mutex<MobileService>>,
    access_logs: AccessLogRingBuffer,
    accept_thread: Option<JoinHandle<()>>,
}

impl TcpMobileServer {
    pub fn bind(bind_addr: &str, service: MobileService) -> std::io::Result<Self> {
        Self::bind_with_shutdown(bind_addr, service, ShutdownSignal::new())
    }

    /// Binds a real TCP listener on `bind_addr` controlled by a caller-supplied [`ShutdownSignal`].
    pub fn bind_with_shutdown(
        bind_addr: &str,
        service: MobileService,
        shutdown: ShutdownSignal,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind(bind_addr)?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let shared = Arc::new(Mutex::new(service));
        let access_logs = AccessLogRingBuffer::from_env(DEFAULT_ACCESS_LOG_CAPACITY);

        let flag = shutdown.clone();
        let svc = Arc::clone(&shared);
        let logs = access_logs.clone();
        let accept_thread = thread::spawn(move || {
            while !flag.is_triggered() {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        if flag.is_triggered() {
                            break;
                        }
                        handle_conn(stream, &svc, &logs);
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
            service: shared,
            access_logs,
            accept_thread: Some(accept_thread),
        })
    }

    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    #[must_use]
    pub fn service(&self) -> Arc<Mutex<MobileService>> {
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

    pub fn shutdown(mut self) {
        self.shutdown.trigger();
        if let Some(handle) = self.accept_thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for TcpMobileServer {
    fn drop(&mut self) {
        self.shutdown.trigger();
        if let Some(handle) = self.accept_thread.take() {
            let _ = handle.join();
        }
    }
}

fn handle_conn(
    mut stream: TcpStream,
    service: &Arc<Mutex<MobileService>>,
    access_logs: &AccessLogRingBuffer,
) {
    let started = Instant::now();
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));

    let Some(req) = parse_request(&stream) else {
        return;
    };
    let incoming_id = req.header(REQUEST_ID_HEADER).map(str::to_owned);
    let method_label = req.method.as_str().to_owned();
    let raw_target = req.target.clone();
    let mut resp = match service.lock() {
        Ok(mut guard) => guard.handle_request(&req),
        Err(_) => MobileHttpResponse::json(500, &serde_json::json!({ "error": "lock_poisoned" })),
    };
    let request_id = resp
        .header(REQUEST_ID_HEADER)
        .and_then(HttpRequestId::parse)
        .unwrap_or_else(|| {
            let id = HttpRequestId::parse_or_generate(incoming_id.as_deref(), "mobile");
            resp.set_header(REQUEST_ID_HEADER, id.as_str());
            id
        });
    let status = resp.status;

    let _ = stream.write_all(&resp.to_http1_bytes());
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Write);

    let timestamp_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let latency_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    access_logs.record(ZeroPhiAccessEvent {
        timestamp_secs,
        service: "ef-mobile-server".to_owned(),
        request_id,
        method: method_label,
        route_template: ZeroPhiRouteTemplate::from_target(&raw_target),
        status,
        latency_us,
    });
}

fn parse_request(stream: &TcpStream) -> Option<MobileHttpRequest> {
    let cloned = stream.try_clone().ok()?;
    let mut reader = BufReader::new(cloned);
    let mut line = String::new();
    if reader.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let mut parts = line.trim_end_matches(['\r', '\n']).split_whitespace();
    let method = MobileHttpMethod::parse(parts.next()?)?;
    let target = parts.next()?.to_owned();

    let mut headers = BTreeMap::new();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).ok()? == 0 {
            break;
        }
        let trimmed = h.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }

    let len = headers
        .get("content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0)
        .min(MOBILE_MAX_BODY_BYTES);
    let mut body = vec![0u8; len];
    if len > 0 {
        reader.read_exact(&mut body).ok()?;
    }

    Some(MobileHttpRequest {
        method,
        target,
        headers,
        body,
    })
}
