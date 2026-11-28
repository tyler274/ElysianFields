#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};

use ef_public_site::{
    DeploymentMode, PUBLIC_MAX_BODY_BYTES, PublicIngressGuard, PublicSiteRouter,
    TcpPublicSiteServer, crisis_lines_appear_before_listings,
};

struct RawResp {
    status: u16,
    headers: BTreeMap<String, String>,
    body: String,
}

impl RawResp {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

fn tcp_request(addr: SocketAddr, method: &str, target: &str, body: &str) -> RawResp {
    let mut stream = TcpStream::connect(addr).expect("connect to ef-public-site");
    let mut raw = format!("{method} {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n");
    if !body.is_empty() {
        raw.push_str("Content-Type: application/json\r\n");
        raw.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    raw.push_str("Connection: close\r\n\r\n");
    raw.push_str(body);

    stream.write_all(raw.as_bytes()).expect("write request");
    stream.flush().expect("flush request");
    let _ = stream.shutdown(Shutdown::Write);

    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).expect("read response");
    let text = String::from_utf8(bytes).expect("utf8 response");
    let (head, body_part) = text.split_once("\r\n\r\n").expect("header/body split");
    let mut lines = head.lines();
    let status: u16 = lines
        .next()
        .expect("status line")
        .split_whitespace()
        .nth(1)
        .expect("status code")
        .parse()
        .expect("numeric status");

    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }

    RawResp {
        status,
        headers,
        body: body_part.to_owned(),
    }
}

#[test]
fn public_server_serves_semantic_html_dei_crisis_lines_first_health_and_guards() {
    let router = PublicSiteRouter::new(DeploymentMode::Production);
    let server = TcpPublicSiteServer::bind("127.0.0.1:0", router).expect("bind public site");
    let addr = server.local_addr();

    // 1. Core semantic HTML routes (zero JS required, zero Bevy canvas).
    for route in [
        "/",
        "/mobile",
        "/privacy",
        "/terms",
        "/clinical-transparency",
        "/demo",
    ] {
        let resp = tcp_request(addr, "GET", route, "");
        assert_eq!(resp.status, 200, "route={route}");
        assert!(resp.body.contains("<!DOCTYPE html>"), "route={route}");
        assert!(!resp.body.contains("<canvas"), "route={route}");
    }

    // 2. `/invite` emits `noindex, nofollow` and `no-store`.
    let invite = tcp_request(addr, "GET", "/invite", "");
    assert_eq!(invite.status, 200);
    assert_eq!(invite.header("x-robots-tag"), Some("noindex, nofollow"));
    assert!(
        invite
            .header("cache-control")
            .unwrap_or_default()
            .contains("no-store")
    );

    // 3. `/dei` and `/dei/search` render crisis lines 988/911/SAMHSA strictly BEFORE listings.
    let dei = tcp_request(addr, "GET", "/dei", "");
    assert_eq!(dei.status, 200);
    assert!(dei.body.contains("988"));
    assert!(dei.body.contains("911"));
    assert!(dei.body.contains("SAMHSA"));
    assert!(crisis_lines_appear_before_listings(&dei.body));

    let dei_search = tcp_request(addr, "GET", "/dei/search?q=Outpatient", "");
    assert_eq!(dei_search.status, 200);
    assert!(crisis_lines_appear_before_listings(&dei_search.body));

    // 4. `/robots.txt`, `/sitemap.xml`, `/health`, `/readyz`, and `/grafana/` -> 404 + no-store.
    let robots = tcp_request(addr, "GET", "/robots.txt", "");
    assert_eq!(robots.status, 200);
    assert!(robots.body.contains("Disallow: /invite"));

    let sitemap = tcp_request(addr, "GET", "/sitemap.xml", "");
    assert_eq!(sitemap.status, 200);
    assert!(sitemap.body.contains("<urlset"));

    for probe in ["/health", "/readyz"] {
        let resp = tcp_request(addr, "GET", probe, "");
        assert_eq!(resp.status, 200, "probe={probe}");
        assert!(resp.body.contains("\"status\":\"ok\""), "probe={probe}");
        assert!(
            resp.header("cache-control")
                .unwrap_or_default()
                .contains("no-store")
        );
    }

    let grafana = tcp_request(addr, "GET", "/grafana/", "");
    assert_eq!(grafana.status, 404);
    assert!(
        grafana
            .header("cache-control")
            .unwrap_or_default()
            .contains("no-store")
    );

    // 5. Interactive synthetic demo POST works over TCP.
    let demo_turn = tcp_request(
        addr,
        "POST",
        "/demo/turn",
        r#"{"visitor_id":"v-tcp-1","prompt":"Summarize renal labs"}"#,
    );
    assert_eq!(demo_turn.status, 200);
    assert!(demo_turn.body.contains("\"Answered\""));

    // 6. Oversized body (> 64 KiB) returns 413 Payload Too Large with `no-store`.
    let oversized = "a".repeat(PUBLIC_MAX_BODY_BYTES + 256);
    let resp_413 = tcp_request(addr, "POST", "/demo/turn", &oversized);
    assert_eq!(resp_413.status, 413);
    assert!(
        resp_413
            .header("cache-control")
            .unwrap_or_default()
            .contains("no-store")
    );

    server.shutdown();
}

#[test]
fn public_server_enforces_per_ip_429_rate_limit_over_tcp() {
    let mut router = PublicSiteRouter::new(DeploymentMode::Production);
    router.set_ingress_guard(PublicIngressGuard::new(2, 60_000));
    let server = TcpPublicSiteServer::bind("127.0.0.1:0", router).expect("bind rate-limited site");
    let addr = server.local_addr();

    assert_eq!(tcp_request(addr, "GET", "/health", "").status, 200);
    assert_eq!(tcp_request(addr, "GET", "/health", "").status, 200);

    let limited = tcp_request(addr, "GET", "/health", "");
    assert_eq!(limited.status, 429);
    assert!(
        limited
            .header("cache-control")
            .unwrap_or_default()
            .contains("no-store")
    );
    assert!(limited.header("retry-after").is_some());

    server.shutdown();
}

fn public_prometheus_value(body: &str, prefix: &str) -> u64 {
    body.lines()
        .find_map(|line| {
            line.strip_prefix(prefix)
                .and_then(|rest| rest.trim().parse::<u64>().ok())
        })
        .unwrap_or_else(|| panic!("missing Prometheus metric `{prefix}` in:\n{body}"))
}

#[test]
fn public_prometheus_metrics_increment_counters_and_emit_zero_phi() {
    let router = PublicSiteRouter::new(DeploymentMode::Production);
    let server = TcpPublicSiteServer::bind("127.0.0.1:0", router).expect("bind public site");
    let addr = server.local_addr();

    assert_eq!(tcp_request(addr, "GET", "/", "").status, 200);
    assert_eq!(tcp_request(addr, "GET", "/does-not-exist", "").status, 404);
    let demo = tcp_request(
        addr,
        "POST",
        "/demo/turn",
        r#"{"visitor_id":"visitor.phi@example.invalid","prompt":"Patient DOB 1979-11-05 token_secret_99"}"#,
    );
    assert_eq!(demo.status, 200);

    let metrics_1 = tcp_request(addr, "GET", "/metrics", "");
    assert_eq!(metrics_1.status, 200);
    assert_eq!(
        metrics_1.header("content-type"),
        Some("text/plain; version=0.0.4; charset=utf-8")
    );
    assert!(
        metrics_1
            .header("cache-control")
            .unwrap_or_default()
            .contains("no-store")
    );
    assert!(
        metrics_1
            .body
            .contains("# TYPE ef_public_http_requests_total counter")
    );
    assert!(
        metrics_1
            .body
            .contains("# TYPE ef_public_uptime_seconds gauge")
    );

    let c2xx_1 = public_prometheus_value(
        &metrics_1.body,
        "ef_public_http_requests_total{status_class=\"2xx\"} ",
    );
    let c4xx_1 = public_prometheus_value(
        &metrics_1.body,
        "ef_public_http_requests_total{status_class=\"4xx\"} ",
    );
    let _c5xx_1 = public_prometheus_value(
        &metrics_1.body,
        "ef_public_http_requests_total{status_class=\"5xx\"} ",
    );
    let _uptime = public_prometheus_value(&metrics_1.body, "ef_public_uptime_seconds ");

    assert!(c2xx_1 >= 2, "expected >= 2 2xx requests, got {c2xx_1}");
    assert!(c4xx_1 >= 1, "expected >= 1 4xx request, got {c4xx_1}");

    let metrics_2 = tcp_request(addr, "GET", "/metrics", "");
    assert_eq!(metrics_2.status, 200);
    let c2xx_2 = public_prometheus_value(
        &metrics_2.body,
        "ef_public_http_requests_total{status_class=\"2xx\"} ",
    );
    assert!(c2xx_2 > c2xx_1);

    for forbidden in [
        "visitor.phi@example.invalid",
        "1979-11-05",
        "token_secret_99",
    ] {
        assert!(
            !metrics_1.body.contains(forbidden) && !metrics_2.body.contains(forbidden),
            "Public Prometheus output leaked forbidden string `{forbidden}`"
        );
    }

    server.shutdown();
}
