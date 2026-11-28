#![cfg(not(target_arch = "wasm32"))]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use ef_core::{ShutdownSignal, assert_zero_phi};
use ef_public_site::{DeploymentMode, PublicSiteRouter, TcpPublicSiteServer};

fn send_raw(addr: std::net::SocketAddr, raw: &str) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect public server");
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
fn public_site_propagates_request_id_records_zero_phi_access_logs_and_drains_on_shutdown() {
    let router = PublicSiteRouter::new(DeploymentMode::Production);
    let shutdown = ShutdownSignal::new();
    let server = TcpPublicSiteServer::bind_with_shutdown("127.0.0.1:0", router, shutdown.clone())
        .expect("bind public site server");
    let addr = server.local_addr();

    let traced = send_raw(
        addr,
        "GET / HTTP/1.1\r\nHost: localhost\r\nX-Request-Id: ef-compose-trace-42\r\n\r\n",
    );
    assert!(traced.starts_with("HTTP/1.1 200 OK"), "resp={traced}");
    assert_eq!(
        extract_header(&traced, "X-Request-Id").as_deref(),
        Some("ef-compose-trace-42")
    );

    let healthz = send_raw(addr, "GET /healthz HTTP/1.1\r\nHost: localhost\r\n\r\n");
    assert!(healthz.starts_with("HTTP/1.1 200 OK"), "resp={healthz}");
    let gen_id = extract_header(&healthz, "X-Request-Id").expect("generated X-Request-Id");
    assert!(
        gen_id.starts_with("ef-public-"),
        "expected ef-public-* generated ID, got {gen_id}"
    );

    let search = send_raw(
        addr,
        "GET /dei/search?q=jane@example.org&state=CA HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    assert!(search.starts_with("HTTP/1.1 200 OK"), "resp={search}");

    let logs = server.access_logs();
    assert_eq!(logs.len(), 3);
    assert_eq!(logs[0].service, "ef-public-site");
    assert_eq!(logs[0].request_id.as_str(), "ef-compose-trace-42");
    assert_eq!(logs[0].route_template.as_str(), "/");
    assert_eq!(logs[1].route_template.as_str(), "/healthz");
    assert_eq!(logs[2].route_template.as_str(), "/dei/search");
    for ev in &logs {
        let line = ev.to_json_line();
        assert_eq!(assert_zero_phi(&line), Ok(()), "line={line}");
        assert!(!line.contains("jane@example.org"), "email leaked: {line}");
    }

    shutdown.trigger();
    let deadline = Instant::now() + Duration::from_secs(2);
    while server.is_running() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !server.is_running(),
        "TcpPublicSiteServer accept loop should exit promptly after ShutdownSignal::trigger()"
    );
}
