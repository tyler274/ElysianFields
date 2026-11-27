#![cfg(not(target_arch = "wasm32"))]

//! Two-instance horizontal-scale & failover integration tests for `ef-clinic-server`:
//! - Boots two concurrent `TcpClinicServer` instances (`Instance A` and `Instance B`) on `127.0.0.1:0`
//!   sharing a durable WAL directory.
//! - Proves cross-instance session & workspace readback (sign in on A -> authenticate/read on B).
//! - Proves `Instance A` shutdown -> `Instance B` `Last-Event-ID` SSE resume with zero duplicate
//!   and zero lost events.
//! - Proves concurrent idempotent command retries hitting BOTH live instances simultaneously
//!   execute at most once across the cluster.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use ef_clinic_domain::auth::{
    AccountStanding, EmailAddress, PhysicianAccount, PhysicianRole, SignInMode,
};
use ef_clinic_domain::chat::RetryBackoffMode;
use ef_clinic_domain::workspace::{
    CitationOrigin, CitedSourcePayload, ReadablePart, classify_record_status,
};
use ef_clinic_server::{ClinicService, TcpClinicServer};
use ef_core::{Locale, PhysicianId, PracticeId, SourceKey, WorkspaceId};
use serde_json::Value;

struct RawHttpResponse {
    status: u16,
    headers: BTreeMap<String, String>,
    body: String,
}

impl RawHttpResponse {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|err| panic!("expected JSON body, got `{}`: {err}", self.body))
    }
}

fn send_tcp_request(
    addr: SocketAddr,
    method: &str,
    target: &str,
    extra_headers: &[(&str, &str)],
    body: &str,
) -> RawHttpResponse {
    let mut stream = TcpStream::connect(addr).expect("connect to live TCP clinic server");
    let mut raw = format!("{method} {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n");
    for (k, v) in extra_headers {
        raw.push_str(&format!("{k}: {v}\r\n"));
    }
    if !body.is_empty() {
        raw.push_str("Content-Type: application/json\r\n");
        raw.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    raw.push_str("Connection: close\r\n\r\n");
    raw.push_str(body);

    stream
        .write_all(raw.as_bytes())
        .expect("write HTTP request");
    stream.flush().expect("flush HTTP request");

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).expect("read HTTP response");
    let text = String::from_utf8(buf).expect("UTF-8 HTTP response");
    let (head, body_part) = text
        .split_once("\r\n\r\n")
        .expect("HTTP header/body separator");

    let mut lines = head.lines();
    let status_line = lines.next().expect("HTTP status line");
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .expect("HTTP status code")
        .parse()
        .expect("numeric HTTP status");

    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }

    RawHttpResponse {
        status,
        headers,
        body: body_part.to_owned(),
    }
}

fn parse_sse_event_ids(body: &str) -> Vec<u64> {
    let mut ids = Vec::new();
    for block in body.replace("\r\n", "\n").split("\n\n") {
        let trimmed = block.trim();
        if trimmed.is_empty() || trimmed.starts_with(':') {
            continue;
        }
        for line in trimmed.lines() {
            if let Some(raw_id) = line.strip_prefix("id:")
                && let Ok(id) = raw_id.trim().parse::<u64>()
            {
                ids.push(id);
            }
        }
    }
    ids
}

fn fresh_temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!(
        "ef-clinic-scale-{label}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("create temp shared WAL dir");
    dir
}

fn synthetic_note(workspace_id: WorkspaceId, source_key: SourceKey) -> CitedSourcePayload {
    CitedSourcePayload {
        source_key,
        workspace_id,
        number: 1,
        source_label: "Synthetic Renal Progress Note [1]".to_owned(),
        title: "Synthetic Multi-Instance Follow-Up Note".to_owned(),
        record_type: "progress_note".to_owned(),
        recorded_at: "2026-02-14T10:00:00Z".to_owned(),
        record_status: "final".to_owned(),
        status_class: classify_record_status("final"),
        origin: CitationOrigin::SourceRecord,
        original_language: Locale::En,
        quote: "Creatinine 1.02 mg/dL, stable eGFR across clinic nodes.".to_owned(),
        body: "Synthetic clinical note for multi-instance scale proof.".to_owned(),
        readable_parts: vec![ReadablePart {
            label: "Assessment".to_owned(),
            text: "Creatinine 1.02 mg/dL, stable eGFR across clinic nodes.".to_owned(),
            unreadable_notice: None,
        }],
        fields: Vec::new(),
        raw_content: None,
        external_url: None,
    }
}

#[test]
fn two_concurrent_instances_share_wal_resume_sse_on_failover_and_deduplicate_concurrent_retries() {
    let wal_dir = fresh_temp_dir("two-node");
    let now_secs = 1_760_000_000u64;
    let default_src = SourceKey::new("src-note-1").expect("src-note-1");
    let ws_a1 = WorkspaceId::new("ws-a1").expect("ws-a1");
    let practice_a = PracticeId::new("practice-a").expect("practice-a");
    let doc_a = PhysicianId::new("phys-a1").expect("phys-a1");

    // Seed durable state into shared WAL via Instance A, then boot Instance B concurrently against the exact same WAL directory.
    let mut service_a = ClinicService::open_durable(
        &wal_dir,
        SignInMode::LocalProof,
        RetryBackoffMode::DeterministicTest,
        default_src.clone(),
        now_secs,
    )
    .expect("open Instance A durable WAL");
    service_a
        .seed_account(PhysicianAccount {
            physician_id: doc_a.clone(),
            practice_id: practice_a.clone(),
            email: EmailAddress::parse("scale.doc@example.invalid").expect("email"),
            display_name: "Dr. Scale A".to_owned(),
            role: PhysicianRole::Standard,
            standing: AccountStanding::Active,
            sessions_ended_before: None,
        })
        .expect("seed physician");
    service_a
        .seed_workspace(
            ws_a1.clone(),
            practice_a,
            "Synthetic Scale Patient A1",
            vec![doc_a],
        )
        .expect("seed workspace");
    service_a
        .seed_source(synthetic_note(ws_a1, default_src.clone()))
        .expect("seed note");

    let service_b = ClinicService::open_durable(
        &wal_dir,
        SignInMode::LocalProof,
        RetryBackoffMode::DeterministicTest,
        default_src,
        now_secs,
    )
    .expect("open Instance B durable WAL");

    let server_a = TcpClinicServer::bind("127.0.0.1:0", service_a).expect("bind Instance A");
    let server_b = TcpClinicServer::bind("127.0.0.1:0", service_b).expect("bind Instance B");
    let addr_a = server_a.local_addr();
    let addr_b = server_b.local_addr();
    assert_ne!(addr_a, addr_b);

    // -------------------------------------------------------------------------
    // 1. Cross-instance session, workspace, draft, and memory readback
    // -------------------------------------------------------------------------
    let req_link = send_tcp_request(
        addr_a,
        "POST",
        "/elysianfields/api/auth/sign-in",
        &[],
        r#"{"email":"scale.doc@example.invalid"}"#,
    );
    assert_eq!(req_link.status, 200);
    let token = req_link.json()["token"]
        .as_str()
        .expect("LocalProof token")
        .to_owned();

    let verify = send_tcp_request(
        addr_a,
        "POST",
        "/elysianfields/api/auth/verify",
        &[],
        &format!(r#"{{"email":"scale.doc@example.invalid","token":"{token}"}}"#),
    );
    assert_eq!(verify.status, 200);
    let session_id = verify.json()["session"]
        .as_str()
        .expect("session id")
        .to_owned();
    let auth_header = format!("Bearer {session_id}");
    let auth_headers = [("Authorization", auth_header.as_str())];

    // Authenticate and read patient workspace + citation directly on Instance B!
    let whoami_b = send_tcp_request(
        addr_b,
        "GET",
        "/elysianfields/api/auth/whoami",
        &auth_headers,
        "",
    );
    assert_eq!(whoami_b.status, 200);
    assert_eq!(whoami_b.json()["physician_id"], "phys-a1");

    let roster_b = send_tcp_request(
        addr_b,
        "GET",
        "/elysianfields/api/patients",
        &auth_headers,
        "",
    );
    assert_eq!(roster_b.status, 200);
    assert_eq!(
        roster_b.json()["patients"]
            .as_array()
            .expect("patients")
            .len(),
        1
    );

    let cite_b = send_tcp_request(
        addr_b,
        "GET",
        "/elysianfields/api/patients/ws-a1/citations/src-note-1",
        &auth_headers,
        "",
    );
    assert_eq!(cite_b.status, 200);
    assert!(
        cite_b.json()["quote"]
            .as_str()
            .unwrap_or_default()
            .contains("Creatinine 1.02")
    );

    // Save draft on Instance A -> read back on Instance B -> stale revision returns 409 on B.
    let put_draft_a = send_tcp_request(
        addr_a,
        "PUT",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-scale/draft",
        &auth_headers,
        r#"{"text":"Cross-node draft note","expected_revision":0}"#,
    );
    assert_eq!(put_draft_a.status, 200);
    assert_eq!(put_draft_a.json()["revision"], 1);

    let get_draft_b = send_tcp_request(
        addr_b,
        "GET",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-scale/draft",
        &auth_headers,
        "",
    );
    assert_eq!(get_draft_b.status, 200);
    assert_eq!(get_draft_b.json()["text"], "Cross-node draft note");
    assert_eq!(get_draft_b.json()["revision"], 1);

    let stale_draft_b = send_tcp_request(
        addr_b,
        "PUT",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-scale/draft",
        &auth_headers,
        r#"{"text":"Stale overwrite","expected_revision":0}"#,
    );
    assert_eq!(stale_draft_b.status, 409);

    // -------------------------------------------------------------------------
    // 2. Concurrent idempotent command retries across BOTH live instances (at-most-once)
    // -------------------------------------------------------------------------
    let worker_count = 8usize;
    let barrier = Arc::new(Barrier::new(worker_count));
    let mut handles = Vec::with_capacity(worker_count);

    for idx in 0..worker_count {
        let target_addr = if idx.is_multiple_of(2) {
            addr_a
        } else {
            addr_b
        };
        let bar = Arc::clone(&barrier);
        let auth_val = auth_header.clone();
        handles.push(thread::spawn(move || {
            let body = r#"{
                "type": "message.submit",
                "idempotency_key": "idem-concurrent-scale-1",
                "payload": {
                    "patient_workspace_id": "ws-a1",
                    "chat_id": "chat-concurrent",
                    "text": "Concurrent retry deduplication check."
                }
            }"#;
            bar.wait();
            let resp = send_tcp_request(
                target_addr,
                "POST",
                "/elysianfields/api/workspaces/ws-a1/chats/chat-concurrent/commands",
                &[("Authorization", auth_val.as_str())],
                body,
            );
            assert_eq!(resp.status, 200, "concurrent submit failed: {}", resp.body);
            let json = resp.json();
            let wi = json["work_item_id"]
                .as_str()
                .expect("work_item_id")
                .to_owned();
            let dedup = json["deduplicated"].as_bool().expect("deduplicated bool");
            (wi, dedup)
        }));
    }

    let mut work_item_ids = Vec::with_capacity(worker_count);
    let mut fresh_executions = 0usize;
    let mut deduplicated_hits = 0usize;
    for handle in handles {
        let (wi, dedup) = handle.join().expect("thread finished");
        work_item_ids.push(wi);
        if dedup {
            deduplicated_hits += 1;
        } else {
            fresh_executions += 1;
        }
    }

    assert_eq!(
        fresh_executions, 1,
        "concurrent idempotent retries across A and B must execute strictly once"
    );
    assert_eq!(deduplicated_hits, worker_count - 1);
    let canonical_wi = &work_item_ids[0];
    assert!(work_item_ids.iter().all(|id| id == canonical_wi));

    // Verify both Instance A and Instance B observe exactly 1 exchange for `chat-concurrent`.
    let snap_conc_a = send_tcp_request(
        addr_a,
        "GET",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-concurrent",
        &auth_headers,
        "",
    );
    let snap_conc_b = send_tcp_request(
        addr_b,
        "GET",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-concurrent",
        &auth_headers,
        "",
    );
    assert_eq!(
        snap_conc_a.json()["exchanges"]
            .as_array()
            .expect("ex A")
            .len(),
        1
    );
    assert_eq!(
        snap_conc_b.json()["exchanges"]
            .as_array()
            .expect("ex B")
            .len(),
        1
    );

    // -------------------------------------------------------------------------
    // 3. Instance A shutdown -> Instance B `Last-Event-ID` SSE resume (0 dup / 0 loss)
    // -------------------------------------------------------------------------
    let turn1_a = send_tcp_request(
        addr_a,
        "POST",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-scale/commands",
        &auth_headers,
        r#"{
            "type": "message.submit",
            "idempotency_key": "idem-failover-turn-1",
            "payload": {
                "patient_workspace_id": "ws-a1",
                "chat_id": "chat-scale",
                "text": "Turn 1 executed on Instance A."
            }
        }"#,
    );
    assert_eq!(turn1_a.status, 200);

    let sse_a_1 = send_tcp_request(
        addr_a,
        "GET",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-scale/stream",
        &auth_headers,
        "",
    );
    assert_eq!(sse_a_1.status, 200);
    assert!(
        sse_a_1
            .headers
            .get("content-type")
            .is_some_and(|v| v.starts_with("text/event-stream"))
    );
    let ids_before_failover = parse_sse_event_ids(&sse_a_1.body);
    assert!(
        ids_before_failover.len() >= 3,
        "expected step + delta + completed events on Instance A, got {ids_before_failover:?}"
    );
    let last_seen_id = *ids_before_failover.last().expect("non-empty event ids");

    // Execute turn 2 on Instance A (appending new events > last_seen_id), then shut Instance A down!
    let turn2_a = send_tcp_request(
        addr_a,
        "POST",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-scale/commands",
        &auth_headers,
        r#"{
            "type": "message.submit",
            "idempotency_key": "idem-failover-turn-2",
            "payload": {
                "patient_workspace_id": "ws-a1",
                "chat_id": "chat-scale",
                "text": "Turn 2 committed right before Instance A shutdown."
            }
        }"#,
    );
    assert_eq!(turn2_a.status, 200);

    // Simulate Instance A crash / shutdown.
    server_a.shutdown();

    // Client reconnects to Instance B presenting `Last-Event-ID: <last_seen_id>`.
    let last_id_header = last_seen_id.to_string();
    let sse_b_resume = send_tcp_request(
        addr_b,
        "GET",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-scale/stream",
        &[
            ("Authorization", auth_header.as_str()),
            ("Last-Event-ID", last_id_header.as_str()),
        ],
        "",
    );
    assert_eq!(sse_b_resume.status, 200);
    assert!(sse_b_resume.body.contains("event: delta_chunk"));
    assert!(sse_b_resume.body.contains("event: completed"));

    let resumed_ids = parse_sse_event_ids(&sse_b_resume.body);
    assert!(
        resumed_ids.len() >= 3,
        "expected full turn 2 SSE stream resumed on Instance B, got {resumed_ids:?}"
    );
    // Zero duplicates: every resumed event ID must be strictly greater than `last_seen_id`.
    assert!(
        resumed_ids.iter().all(|&id| id > last_seen_id),
        "duplicate event emitted after failover: last_seen={last_seen_id}, resumed={resumed_ids:?}"
    );
    // Zero lost events: combined sequence across A -> B must be strictly contiguous 1..=N!
    let mut combined = ids_before_failover;
    combined.extend_from_slice(&resumed_ids);
    let expected: Vec<u64> = (1..=(combined.len() as u64)).collect();
    assert_eq!(
        combined, expected,
        "combined A->B SSE event sequence must be strictly contiguous with zero gaps or duplicates"
    );

    // Full durable transcript on Instance B reflects both turn 1 and turn 2 cleanly.
    let snap_after_failover = send_tcp_request(
        addr_b,
        "GET",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-scale",
        &auth_headers,
        "",
    );
    assert_eq!(snap_after_failover.status, 200);
    let exchanges = snap_after_failover.json()["exchanges"]
        .as_array()
        .expect("exchanges")
        .clone();
    assert_eq!(exchanges.len(), 2);
    assert_eq!(exchanges[0]["answer"]["kind"], "finished");
    assert_eq!(exchanges[1]["answer"]["kind"], "finished");

    server_b.shutdown();
    let _ = fs::remove_dir_all(&wal_dir);
}
