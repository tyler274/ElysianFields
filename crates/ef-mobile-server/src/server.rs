//! Native HTTP/1.1 TCP server hosting [`MobileService`].

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::http::{MOBILE_MAX_BODY_BYTES, MobileHttpMethod, MobileHttpRequest, MobileHttpResponse};
use crate::service::MobileService;

pub struct TcpMobileServer {
    addr: SocketAddr,
    shutdown_flag: Arc<AtomicBool>,
    service: Arc<Mutex<MobileService>>,
    accept_thread: Option<JoinHandle<()>>,
}

impl TcpMobileServer {
    pub fn bind(bind_addr: &str, service: MobileService) -> std::io::Result<Self> {
        let listener = TcpListener::bind(bind_addr)?;
        let addr = listener.local_addr()?;
        let shutdown_flag = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Mutex::new(service));

        let flag = Arc::clone(&shutdown_flag);
        let svc = Arc::clone(&shared);
        let accept_thread = thread::spawn(move || {
            for stream_res in listener.incoming() {
                if flag.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream_res else {
                    continue;
                };
                handle_conn(stream, &svc);
            }
        });

        Ok(Self {
            addr,
            shutdown_flag,
            service: shared,
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

    pub fn shutdown(mut self) {
        self.shutdown_flag.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(handle) = self.accept_thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for TcpMobileServer {
    fn drop(&mut self) {
        self.shutdown_flag.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(handle) = self.accept_thread.take() {
            let _ = handle.join();
        }
    }
}

fn handle_conn(mut stream: TcpStream, service: &Arc<Mutex<MobileService>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));

    let Some(req) = parse_request(&stream) else {
        return;
    };
    let resp = match service.lock() {
        Ok(mut guard) => guard.handle_request(&req),
        Err(_) => MobileHttpResponse::json(500, &serde_json::json!({ "error": "lock_poisoned" })),
    };

    let _ = stream.write_all(&resp.to_http1_bytes());
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Write);
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
