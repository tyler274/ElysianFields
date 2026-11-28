#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use ef_mobile_domain::{
    AccountFencedHealthBatch, AccountIncarnation, ClinicalRecordBucket, ConfidentialityTag,
    FileReceiptJournal, HealthAdapterPlatform, HealthMetricKind, HealthSampleEntry,
    ImportSourceKind, IncomingBatchItem, MobileAccountId, PatientRecord,
};
use ef_mobile_server::{MobileService, TcpMobileServer};
use serde_json::Value;

struct Resp {
    status: u16,
    headers: BTreeMap<String, String>,
    body: String,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).expect("JSON body")
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

fn tcp_req(addr: SocketAddr, method: &str, target: &str, auth: Option<&str>, body: &str) -> Resp {
    let mut stream = TcpStream::connect(addr).expect("connect mobile tcp");
    let mut raw = format!("{method} {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n");
    if let Some(bearer) = auth {
        raw.push_str(&format!("Authorization: Bearer {bearer}\r\n"));
    }
    if !body.is_empty() {
        raw.push_str("Content-Type: application/json\r\n");
        raw.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    raw.push_str("Connection: close\r\n\r\n");
    raw.push_str(body);
    stream.write_all(raw.as_bytes()).expect("write");
    stream.flush().expect("flush");
    let _ = stream.shutdown(Shutdown::Write);

    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).expect("read");
    let text = String::from_utf8(bytes).expect("utf8");
    let (head, body_part) = text.split_once("\r\n\r\n").expect("split");
    let mut lines = head.lines();
    let status: u16 = lines
        .next()
        .expect("status line")
        .split_whitespace()
        .nth(1)
        .expect("code")
        .parse()
        .expect("u16");
    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }
    Resp {
        status,
        headers,
        body: body_part.to_owned(),
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!(
        "ef-mobile-server-{name}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("mkdir");
    dir
}

#[test]
fn mobile_server_tcp_wal_independent_journal_and_restore_replay_readyz_gate() {
    let db_dir = temp_dir("db");
    let backup_dir = temp_dir("backup");
    let journal_dir = temp_dir("journal");

    // Verify SQL schema file defines standalone Curie Mobile tables.
    let sql = include_str!("../migrations/0001_m3_mobile.sql");
    for table in [
        "schema_migrations",
        "mobile_accounts",
        "mobile_deleted_incarnation_fences",
        "patient_records",
        "health_samples",
        "for_my_doctor_sends",
    ] {
        assert!(sql.contains(table), "missing SQL table {table}");
    }

    let journal = FileReceiptJournal::open(&journal_dir).expect("open file journal");
    let svc1 = MobileService::open_durable(&db_dir, Box::new(journal), 1_760_000_000, false)
        .expect("open mobile service");
    let srv1 = TcpMobileServer::bind("127.0.0.1:0", svc1).expect("bind mobile srv 1");
    let addr1 = srv1.local_addr();

    // Readyz passes & returns no-store.
    let ready = tcp_req(addr1, "GET", "/elysianfields/mobile-api/readyz", None, "");
    assert_eq!(ready.status, 200);
    assert!(ready.headers["cache-control"].contains("no-store"));

    // Sign in standalone patient account.
    let link = tcp_req(
        addr1,
        "POST",
        "/elysianfields/mobile-api/auth/request-link",
        None,
        r#"{"email":"mobile.e2e@example.invalid"}"#,
    );
    assert_eq!(link.status, 200);
    let token = link.json()["token"].as_str().expect("token").to_owned();

    let verify = tcp_req(
        addr1,
        "POST",
        "/elysianfields/mobile-api/auth/verify",
        None,
        &format!(r#"{{"email":"mobile.e2e@example.invalid","token":"{token}"}}"#),
    );
    assert_eq!(verify.status, 200);
    let sess_id = verify.json()["session_id"]
        .as_str()
        .expect("session_id")
        .to_owned();
    let acct_id = verify.json()["account_id"]
        .as_str()
        .expect("account_id")
        .to_owned();

    // Ingest records with per-record savepoints + upload health samples idempotently.
    let ingest_body = serde_json::to_string(&serde_json::json!({
        "items": [
            IncomingBatchItem::Valid(PatientRecord {
                record_id: "m-rec-1".to_owned(),
                version: 1,
                account_id: MobileAccountId::new(&acct_id).expect("acct"),
                incarnation: AccountIncarnation::first(),
                source_kind: ImportSourceKind::SmartHealthLink,
                portal_id: None,
                bucket: ClinicalRecordBucket::Vitals,
                confidentiality: ConfidentialityTag::Standard,
                part2_notice: None,
                original_date: "2026-01-12".to_owned(),
                title: "Synthetic BP".to_owned(),
                summary: "118/76 mmHg".to_owned(),
                native_bytes: b"{}".to_vec(),
                removed_upstream: false,
            }),
            IncomingBatchItem::Malformed {
                external_id: "m-bad-2".to_owned(),
                reason: "invalid XML".to_owned(),
            }
        ]
    }))
    .expect("json");
    let ing = tcp_req(
        addr1,
        "POST",
        "/elysianfields/mobile-api/records/ingest",
        Some(&sess_id),
        &ingest_body,
    );
    assert_eq!(ing.status, 200);
    assert_eq!(
        ing.json()["committed_record_ids"],
        serde_json::json!(["m-rec-1"])
    );
    assert_eq!(
        ing.json()["failed_items"].as_array().expect("failed").len(),
        1
    );

    let health_body = serde_json::to_string(&AccountFencedHealthBatch {
        account_id: MobileAccountId::new(&acct_id).expect("acct"),
        account_incarnation: AccountIncarnation::first(),
        idempotency_key: "hc-batch-1".to_owned(),
        samples: vec![HealthSampleEntry {
            sample_id: "hc-1".to_owned(),
            metric: HealthMetricKind::Steps,
            value_milli: 7_500_000,
            unit: "count".to_owned(),
            recorded_at_secs: 1_760_000_050,
            platform: HealthAdapterPlatform::AndroidHealthConnect,
        }],
    })
    .expect("health json");
    let h_up = tcp_req(
        addr1,
        "POST",
        "/elysianfields/mobile-api/health/upload",
        Some(&sess_id),
        &health_body,
    );
    assert_eq!(h_up.status, 200);
    assert_eq!(h_up.json()["stored_count"], 1);

    // Copy pre-deletion WAL into isolated backup directory BEFORE deleting account!
    fs::copy(
        db_dir.join("mobile-wal.jsonl"),
        backup_dir.join("mobile-wal.jsonl"),
    )
    .expect("snapshot pre-deletion WAL backup");

    // Export works before deletion.
    let exp = tcp_req(
        addr1,
        "GET",
        "/elysianfields/mobile-api/export",
        Some(&sess_id),
        "",
    );
    assert_eq!(exp.status, 200);
    assert_eq!(exp.json()["records"].as_array().expect("records").len(), 1);

    // Delete account -> writes immutable receipt to independent FileReceiptJournal.
    let del = tcp_req(
        addr1,
        "POST",
        "/elysianfields/mobile-api/account/delete",
        Some(&sess_id),
        r#"{"event_id":"evt-tcp-del-1","occurred_at":"2026-01-12T13:00:00Z"}"#,
    );
    assert_eq!(del.status, 200);
    assert_eq!(del.json()["deleted"], true);
    srv1.shutdown();

    // Boot fresh server from PRE-DELETION backup with `require_restore_replay = true`.
    let journal2 = FileReceiptJournal::open(&journal_dir).expect("reopen journal");
    let svc2 = MobileService::open_durable(&backup_dir, Box::new(journal2), 1_760_000_200, true)
        .expect("open restored backup");
    let srv2 = TcpMobileServer::bind("127.0.0.1:0", svc2).expect("bind restored server");
    let addr2 = srv2.local_addr();

    // Before replay runs, `/readyz` and API traffic fail closed with `503`!
    let not_ready = tcp_req(addr2, "GET", "/elysianfields/mobile-api/readyz", None, "");
    assert_eq!(not_ready.status, 503);

    // Execute mandatory restore replay gate against independent journal!
    {
        let svc_arc = srv2.service();
        let report = svc_arc
            .lock()
            .expect("lock")
            .execute_restore_replay_gate(1_000)
            .expect("restore replay succeeds");
        assert_eq!(report.applied, 1);
    }

    // Now `/readyz` is `200`, and the deleted account CANNOT resurrect from the restored backup!
    let ready2 = tcp_req(addr2, "GET", "/elysianfields/mobile-api/readyz", None, "");
    assert_eq!(ready2.status, 200);
    let resurrected_session = tcp_req(
        addr2,
        "GET",
        "/elysianfields/mobile-api/auth/session",
        Some(&sess_id),
        "",
    );
    assert_eq!(resurrected_session.status, 401);

    let resurrected_sign_in = tcp_req(
        addr2,
        "POST",
        "/elysianfields/mobile-api/auth/request-link",
        None,
        r#"{"email":"mobile.e2e@example.invalid"}"#,
    );
    assert_eq!(resurrected_sign_in.status, 403);

    srv2.shutdown();
    let _ = fs::remove_dir_all(&db_dir);
    let _ = fs::remove_dir_all(&backup_dir);
    let _ = fs::remove_dir_all(&journal_dir);
}

fn mobile_prometheus_value(body: &str, prefix: &str) -> u64 {
    body.lines()
        .find_map(|line| {
            line.strip_prefix(prefix)
                .and_then(|rest| rest.trim().parse::<u64>().ok())
        })
        .unwrap_or_else(|| panic!("missing Prometheus metric `{prefix}` in:\n{body}"))
}

#[test]
fn mobile_prometheus_metrics_increment_counters_and_emit_zero_phi() {
    let svc = MobileService::new_in_memory(1_760_200_000);
    let srv = TcpMobileServer::bind("127.0.0.1:0", svc).expect("bind mobile server");
    let addr = srv.local_addr();

    // Generate 4xx (unauthenticated), 2xx (auth + WAL mutation) with synthetic email/token/patient data.
    let unauth = tcp_req(addr, "GET", "/elysianfields/mobile-api/records", None, "");
    assert_eq!(unauth.status, 401);

    let link = tcp_req(
        addr,
        "POST",
        "/elysianfields/mobile-api/auth/request-link",
        None,
        r#"{"email":"patient.phi.dob.19910314@example.invalid"}"#,
    );
    assert_eq!(link.status, 200);
    let token = link.json()["token"].as_str().expect("token").to_owned();

    let verify = tcp_req(
        addr,
        "POST",
        "/elysianfields/mobile-api/auth/verify",
        None,
        &format!(r#"{{"email":"patient.phi.dob.19910314@example.invalid","token":"{token}"}}"#),
    );
    assert_eq!(verify.status, 200);
    let sess_id = verify.json()["session_id"]
        .as_str()
        .expect("session_id")
        .to_owned();

    let metrics_1 = tcp_req(addr, "GET", "/elysianfields/mobile-api/metrics", None, "");
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
            .contains("# TYPE ef_mobile_http_requests_total counter")
    );
    assert!(metrics_1.body.contains("# TYPE ef_mobile_wal_seq gauge"));
    assert!(
        metrics_1
            .body
            .contains("# TYPE ef_mobile_uptime_seconds gauge")
    );
    assert!(metrics_1.body.contains("# TYPE ef_mobile_db_ready gauge"));

    let c2xx_1 = mobile_prometheus_value(
        &metrics_1.body,
        "ef_mobile_http_requests_total{status_class=\"2xx\"} ",
    );
    let c4xx_1 = mobile_prometheus_value(
        &metrics_1.body,
        "ef_mobile_http_requests_total{status_class=\"4xx\"} ",
    );
    let wal_seq = mobile_prometheus_value(&metrics_1.body, "ef_mobile_wal_seq ");
    let db_ready = mobile_prometheus_value(&metrics_1.body, "ef_mobile_db_ready ");
    let _uptime = mobile_prometheus_value(&metrics_1.body, "ef_mobile_uptime_seconds ");

    assert!(c2xx_1 >= 2, "expected >= 2 2xx requests, got {c2xx_1}");
    assert!(c4xx_1 >= 1, "expected >= 1 4xx request, got {c4xx_1}");
    assert!(wal_seq >= 2, "expected >= 2 WAL entries, got {wal_seq}");
    assert_eq!(db_ready, 1);

    // Second scrape increments 2xx counter.
    let metrics_2 = tcp_req(addr, "GET", "/elysianfields/mobile-api/metrics", None, "");
    assert_eq!(metrics_2.status, 200);
    let c2xx_2 = mobile_prometheus_value(
        &metrics_2.body,
        "ef_mobile_http_requests_total{status_class=\"2xx\"} ",
    );
    assert!(c2xx_2 > c2xx_1);

    for forbidden in [
        "patient.phi.dob.19910314@example.invalid",
        "19910314",
        token.as_str(),
        sess_id.as_str(),
    ] {
        assert!(
            !metrics_1.body.contains(forbidden) && !metrics_2.body.contains(forbidden),
            "Mobile Prometheus output leaked forbidden string `{forbidden}`"
        );
    }

    srv.shutdown();
}
