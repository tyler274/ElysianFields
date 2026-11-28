#![cfg(not(target_arch = "wasm32"))]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use ef_core::{ShutdownSignal, assert_zero_phi};
use ef_mobile_server::{MobileService, TcpMobileServer};

fn send_raw(addr: std::net::SocketAddr, raw: &str) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect mobile server");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set read timeout");
    stream.write_all(raw.as_bytes()).expect("write request");
    stream.flush().expect("flush");
    let mut buf = String::new();
    let _ = stream.read_to_string(&mut buf);
    buf
}

fn extract_header(resp: &str, name: &str) -> Option<String> {
    let head = resp.split("\r\n\r\n").next().unwrap_or("");
    for line in head.lines().skip(1) {
        if let Some((k, v)) = line.split_once(':')
            && k.trim().eq_ignore_ascii_case(name)
        {
            return Some(v.trim().to_owned());
        }
    }
    None
}

#[test]
fn mobile_propagates_request_id_records_zero_phi_access_logs_and_drains_on_shutdown() {
    let service = MobileService::new_in_memory(1_760_000_000);
    let shutdown = ShutdownSignal::new();
    let server = TcpMobileServer::bind_with_shutdown("127.0.0.1:0", service, shutdown.clone())
        .expect("bind mobile server");
    let addr = server.local_addr();

    let traced = send_raw(
        addr,
        "GET /elysianfields/mobile-api/healthz HTTP/1.1\r\nHost: localhost\r\nX-Request-Id: ef-compose-trace-42\r\n\r\n",
    );
    assert!(traced.starts_with("HTTP/1.1 200 OK"), "resp={traced}");
    assert_eq!(
        extract_header(&traced, "X-Request-Id").as_deref(),
        Some("ef-compose-trace-42")
    );

    let queried = send_raw(
        addr,
        "GET /elysianfields/mobile-api/connected-care/status?email=patient@example.org HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    let gen_id = extract_header(&queried, "X-Request-Id").expect("generated X-Request-Id");
    assert!(
        gen_id.starts_with("ef-mobile-"),
        "expected ef-mobile-* generated ID, got {gen_id}"
    );

    let logs = server.access_logs();
    assert_eq!(logs.len(), 2);
    assert_eq!(logs[0].service, "ef-mobile-server");
    assert_eq!(logs[0].request_id.as_str(), "ef-compose-trace-42");
    assert_eq!(
        logs[0].route_template.as_str(),
        "/elysianfields/mobile-api/healthz"
    );
    assert_eq!(
        logs[1].route_template.as_str(),
        "/elysianfields/mobile-api/connected-care/status"
    );
    for ev in &logs {
        let line = ev.to_json_line();
        assert_eq!(assert_zero_phi(&line), Ok(()), "line={line}");
        assert!(
            !line.contains("patient@example.org"),
            "email leaked: {line}"
        );
    }

    shutdown.trigger();
    let deadline = Instant::now() + Duration::from_secs(2);
    while server.is_running() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !server.is_running(),
        "TcpMobileServer accept loop should exit promptly after ShutdownSignal::trigger()"
    );
}
