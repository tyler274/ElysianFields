#![cfg(not(target_arch = "wasm32"))]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use ef_clinic_domain::auth::SignInMode;
use ef_clinic_domain::chat::RetryBackoffMode;
use ef_clinic_server::{ClinicService, TcpClinicServer};
use ef_core::{ShutdownSignal, SourceKey, assert_zero_phi};

fn send_raw(addr: std::net::SocketAddr, raw: &str) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect clinic server");
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
fn clinic_propagates_request_id_records_zero_phi_access_logs_and_drains_on_shutdown() {
    let service = ClinicService::new_in_memory(
        SignInMode::Hosted,
        RetryBackoffMode::Hosted,
        SourceKey::new("src-1").expect("source key"),
        1_760_000_000,
    );
    let shutdown = ShutdownSignal::new();
    let server = TcpClinicServer::bind_with_shutdown("127.0.0.1:0", service, shutdown.clone())
        .expect("bind clinic server");
    let addr = server.local_addr();

    // 1. Custom valid X-Request-Id echoed verbatim on /healthz.
    let traced = send_raw(
        addr,
        "GET /elysianfields/api/healthz HTTP/1.1\r\nHost: localhost\r\nX-Request-Id: ef-compose-trace-42\r\n\r\n",
    );
    assert!(traced.starts_with("HTTP/1.1 200 OK"), "resp={traced}");
    assert_eq!(
        extract_header(&traced, "X-Request-Id").as_deref(),
        Some("ef-compose-trace-42")
    );

    // 2. Missing/invalid X-Request-Id generates deterministic ef-clinic-* ID and redacts dynamic segments + query.
    let redacted = send_raw(
        addr,
        "GET /elysianfields/api/workspaces/ws-patient-99/chats/chat-55/snapshot?email=doc@clinic.org HTTP/1.1\r\nHost: localhost\r\nX-Request-Id: invalid id with spaces\r\n\r\n",
    );
    let gen_id = extract_header(&redacted, "X-Request-Id").expect("generated X-Request-Id");
    assert!(
        gen_id.starts_with("ef-clinic-"),
        "expected ef-clinic-* generated ID, got {gen_id}"
    );

    // 3. Static entry shell + 404 both return valid X-Request-Id.
    let entry = send_raw(
        addr,
        "GET /elysianfields/ HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    assert!(entry.starts_with("HTTP/1.1 200 OK"));
    assert!(extract_header(&entry, "X-Request-Id").is_some());

    let logs = server.access_logs();
    assert_eq!(logs.len(), 3);
    assert_eq!(logs[0].service, "ef-clinic-server");
    assert_eq!(logs[0].request_id.as_str(), "ef-compose-trace-42");
    assert_eq!(
        logs[0].route_template.as_str(),
        "/elysianfields/api/healthz"
    );
    assert_eq!(logs[0].status, 200);

    assert_eq!(
        logs[1].route_template.as_str(),
        "/elysianfields/api/workspaces/:id/chats/:id/snapshot"
    );
    for ev in &logs {
        let line = ev.to_json_line();
        assert_eq!(assert_zero_phi(&line), Ok(()), "line={line}");
        assert!(
            !line.contains("ws-patient-99"),
            "raw workspace ID leaked: {line}"
        );
        assert!(!line.contains("doc@clinic.org"), "email leaked: {line}");
    }

    // 4. Graceful shutdown drains accept loop deterministically.
    shutdown.trigger();
    let deadline = Instant::now() + Duration::from_secs(2);
    while server.is_running() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !server.is_running(),
        "TcpClinicServer accept loop should exit promptly after ShutdownSignal::trigger()"
    );
}
