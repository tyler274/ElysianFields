#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::time::Duration;

use ef_core::assert_zero_phi;
use ef_public_site::{
    DeploymentMode, PublicSiteRouter, TcpPublicSiteServer, crisis_lines_appear_before_listings,
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

fn send_get(addr: SocketAddr, target: &str, request_id: Option<&str>) -> RawResp {
    let mut stream = TcpStream::connect(addr).expect("connect to TcpPublicSiteServer");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set read timeout");
    let mut raw = format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n");
    if let Some(rid) = request_id {
        raw.push_str(&format!("X-Request-Id: {rid}\r\n"));
    }
    raw.push_str("Connection: close\r\n\r\n");
    stream.write_all(raw.as_bytes()).expect("write request");
    stream.flush().expect("flush");
    let _ = stream.shutdown(Shutdown::Write);

    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).expect("read response");
    let text = String::from_utf8(bytes).expect("utf8 response");
    let (head, body) = text.split_once("\r\n\r\n").expect("split HTTP response");
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
        body: body.to_owned(),
    }
}

#[test]
fn dei_finder_supports_zero_js_state_service_keyword_filtering_empty_state_xss_escaping_and_request_id()
 {
    let router = PublicSiteRouter::new(DeploymentMode::Production);
    let server = TcpPublicSiteServer::bind("127.0.0.1:0", router).expect("bind public site");
    let addr = server.local_addr();

    // 1. Unfiltered /dei renders accessible zero-JS GET form, crisis banner first, and all states.
    let base = send_get(addr, "/dei", Some("ef-dei-trace-7"));
    assert_eq!(base.status, 200);
    assert_eq!(base.header("x-request-id"), Some("ef-dei-trace-7"));
    assert!(crisis_lines_appear_before_listings(&base.body));
    assert!(base.body.contains("<form method=\"get\" action=\"/dei\">"));
    assert!(base.body.contains("name=\"state\""));
    assert!(base.body.contains("name=\"service\""));
    assert!(base.body.contains("name=\"q\""));
    assert!(!base.body.to_ascii_lowercase().contains("<script"));
    assert!(!base.body.to_ascii_lowercase().contains("<canvas"));

    // 2. State + service filtering (CA + crisis / detox / outpatient).
    let ca_crisis = send_get(addr, "/dei?state=CA&service=crisis", None);
    assert_eq!(ca_crisis.status, 200);
    assert!(
        ca_crisis
            .header("x-request-id")
            .is_some_and(|id| id.starts_with("ef-public-"))
    );
    assert!(crisis_lines_appear_before_listings(&ca_crisis.body));
    assert!(
        ca_crisis
            .body
            .contains("Bay Area Crisis Stabilization &amp; Detox")
    );
    assert!(
        ca_crisis
            .body
            .contains("Harborview Behavioral Crisis Center")
    );
    assert!(
        !ca_crisis
            .body
            .contains("Pacific Coast Outpatient Recovery Clinic")
    );
    assert!(
        !ca_crisis
            .body
            .contains("Cascade Outpatient Recovery Center")
    );

    let ca_detox = send_get(addr, "/dei?state=CA&service=detox", None);
    assert_eq!(ca_detox.status, 200);
    assert!(
        ca_detox
            .body
            .contains("Bay Area Crisis Stabilization &amp; Detox")
    );
    assert!(
        !ca_detox
            .body
            .contains("Harborview Behavioral Crisis Center")
    );
    assert!(!ca_detox.body.contains("Puget Sound Residential Wellness"));

    let ca_outpatient_la = send_get(addr, "/dei?state=CA&service=outpatient&q=Los+Angeles", None);
    assert_eq!(ca_outpatient_la.status, 200);
    assert!(crisis_lines_appear_before_listings(&ca_outpatient_la.body));
    assert!(
        ca_outpatient_la
            .body
            .contains("Pacific Coast Outpatient Recovery Clinic")
    );
    assert!(
        !ca_outpatient_la
            .body
            .contains("Bay Area Crisis Stabilization")
    );

    // 3. Helpful empty state with crisis banner ALWAYS pinned at top.
    let empty = send_get(
        addr,
        "/dei?state=CA&service=detox&q=NoSuchFacilityCityXYZ",
        Some("ef-dei-empty-1"),
    );
    assert_eq!(empty.status, 200);
    assert_eq!(empty.header("x-request-id"), Some("ef-dei-empty-1"));
    assert!(crisis_lines_appear_before_listings(&empty.body));
    assert!(empty.body.contains("id=\"empty-results\""));
    assert!(
        empty
            .body
            .contains("No recovery facilities matched your current filters")
    );
    assert!(empty.body.contains("988"));
    assert!(empty.body.contains("911"));
    assert!(empty.body.contains("SAMHSA National Helpline"));

    // 4. Strict XSS escaping across q, state, and service query parameters.
    let xss = send_get(
        addr,
        "/dei?q=%3Cscript%3Ealert(%22xss%22)%3C%2Fscript%3E&state=%22%3E%3Cimg+src%3Dx+onerror%3Dalert(1)%3E&service=%27%3E%3Csvg+onload%3Dalert(2)%3E",
        Some("ef-dei-xss-1"),
    );
    assert_eq!(xss.status, 200);
    assert_eq!(xss.header("x-request-id"), Some("ef-dei-xss-1"));
    assert!(crisis_lines_appear_before_listings(&xss.body));
    assert!(
        !xss.body.to_ascii_lowercase().contains("<script"),
        "unescaped <script> found in HTML: {}",
        xss.body
    );
    assert!(
        !xss.body.to_ascii_lowercase().contains("<img"),
        "unescaped <img> found in HTML: {}",
        xss.body
    );
    assert!(
        !xss.body.to_ascii_lowercase().contains("<svg"),
        "unescaped <svg> found in HTML: {}",
        xss.body
    );
    assert!(
        xss.body
            .contains("&lt;script&gt;alert(&quot;xss&quot;)&lt;/script&gt;")
    );
    assert!(xss.body.contains("&#39;&gt;&lt;svg"));

    // 5. Verify zero-PHI access logs stripped query strings cleanly to `/dei`.
    let logs = server.access_logs();
    assert_eq!(logs.len(), 6);
    for ev in &logs {
        assert_eq!(ev.route_template.as_str(), "/dei");
        let line = ev.to_json_line();
        assert_eq!(assert_zero_phi(&line), Ok(()), "line={line}");
        assert!(
            !line.contains("script"),
            "query leaked into telemetry: {line}"
        );
    }

    server.shutdown();
}
