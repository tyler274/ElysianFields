#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use ef_clinic_domain::auth::{
    AccountStanding, EmailAddress, HOSTED_ANTI_ENUMERATION_MESSAGE, PhysicianAccount,
    PhysicianRole, SignInMode,
};
use ef_clinic_domain::chat::RetryBackoffMode;
use ef_clinic_domain::workspace::{
    CitationOrigin, CitedSourcePayload, ReadablePart, classify_record_status,
    synthetic_5000_row_lab_source,
};
use ef_clinic_server::{AUTH_MAX_BODY_BYTES, ClinicService, STRICT_CSP, TcpClinicServer};
use ef_core::{Locale, PhysicianId, PracticeId, SourceKey, WorkspaceId};
use ef_harness::StubProvider;
use serde_json::Value;

struct RawHttpResponse {
    status: u16,
    headers: BTreeMap<String, String>,
    body: String,
}

impl RawHttpResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

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
    let mut stream = TcpStream::connect(addr).expect("connect to local TCP clinic server");
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

fn fresh_temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!(
        "ef-clinic-server-{label}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("create temp durable WAL dir");
    dir
}

fn synthetic_progress_note(workspace_id: WorkspaceId, source_key: SourceKey) -> CitedSourcePayload {
    CitedSourcePayload {
        source_key,
        workspace_id,
        number: 1,
        source_label: "Synthetic Nephrology Progress Note [1]".to_owned(),
        title: "Synthetic Outpatient Follow-Up Note".to_owned(),
        record_type: "progress_note".to_owned(),
        recorded_at: "2026-01-12T09:15:00Z".to_owned(),
        record_status: "final".to_owned(),
        status_class: classify_record_status("final"),
        origin: CitationOrigin::SourceRecord,
        original_language: Locale::En,
        quote: "Creatinine 1.02 mg/dL, stable eGFR, continue current regimen.".to_owned(),
        body: "Synthetic clinical note body verifying verbatim citation inspection.".to_owned(),
        readable_parts: vec![ReadablePart {
            label: "Assessment & Plan".to_owned(),
            text: "Creatinine 1.02 mg/dL, stable eGFR, continue current regimen.".to_owned(),
            unreadable_notice: None,
        }],
        fields: Vec::new(),
        raw_content: None,
        external_url: None,
    }
}

fn seed_vertical_slice(service: &mut ClinicService) {
    let practice_a = PracticeId::new("practice-a").expect("practice-a");
    let practice_b = PracticeId::new("practice-b").expect("practice-b");
    let doc_a = PhysicianId::new("phys-a1").expect("phys-a1");
    let doc_deactivated = PhysicianId::new("phys-deactivated").expect("phys-deactivated");
    let doc_b = PhysicianId::new("phys-b1").expect("phys-b1");

    service
        .seed_account(PhysicianAccount {
            physician_id: doc_a.clone(),
            practice_id: practice_a.clone(),
            email: EmailAddress::parse("clinician.a@example.invalid").expect("email"),
            display_name: "Dr. Synthetic A".to_owned(),
            role: PhysicianRole::Standard,
            standing: AccountStanding::Active,
            sessions_ended_before: None,
        })
        .expect("seed active account");

    service
        .seed_account(PhysicianAccount {
            physician_id: doc_deactivated,
            practice_id: practice_a.clone(),
            email: EmailAddress::parse("deactivated.a@example.invalid").expect("email"),
            display_name: "Dr. Former A".to_owned(),
            role: PhysicianRole::Standard,
            standing: AccountStanding::Deactivated,
            sessions_ended_before: None,
        })
        .expect("seed deactivated account");

    service
        .seed_account(PhysicianAccount {
            physician_id: doc_b.clone(),
            practice_id: practice_b.clone(),
            email: EmailAddress::parse("clinician.b@example.invalid").expect("email"),
            display_name: "Dr. Synthetic B".to_owned(),
            role: PhysicianRole::Standard,
            standing: AccountStanding::Active,
            sessions_ended_before: None,
        })
        .expect("seed practice B account");

    let ws_a1 = WorkspaceId::new("ws-a1").expect("ws-a1");
    let ws_a2 = WorkspaceId::new("ws-a2").expect("ws-a2");
    let ws_b1 = WorkspaceId::new("ws-b1").expect("ws-b1");

    service
        .seed_workspace(
            ws_a1.clone(),
            practice_a.clone(),
            "Synthetic Patient A1",
            vec![doc_a],
        )
        .expect("seed ws-a1");
    service
        .seed_workspace(
            ws_a2,
            practice_a,
            "Synthetic Patient A2 (Demographic)",
            Vec::new(),
        )
        .expect("seed ws-a2");
    service
        .seed_workspace(
            ws_b1.clone(),
            practice_b,
            "Synthetic Patient B1 (Other Practice)",
            vec![doc_b],
        )
        .expect("seed ws-b1");

    service
        .seed_source(synthetic_progress_note(
            ws_a1.clone(),
            SourceKey::new("src-note-1").expect("src-note-1"),
        ))
        .expect("seed citation [1]");
    service
        .seed_source(synthetic_5000_row_lab_source(
            ws_a1.clone(),
            SourceKey::new("src-lab-12").expect("src-lab-12"),
        ))
        .expect("seed citation [12] 5000-row table");
    service
        .seed_source(synthetic_progress_note(
            ws_a1,
            SourceKey::new("src-withdrawn-9").expect("src-withdrawn-9"),
        ))
        .expect("seed withdrawable citation");
    service
        .seed_source(synthetic_progress_note(
            ws_b1,
            SourceKey::new("src-other-b1").expect("src-other-b1"),
        ))
        .expect("seed cross-patient citation");
}

#[test]
fn full_vertical_slice_e2e_over_real_tcp_with_restart_durability() {
    let wal_dir = fresh_temp_dir("vertical-slice");
    let default_src = SourceKey::new("src-note-1").expect("src-note-1");

    let mut service1 = ClinicService::open_durable(
        &wal_dir,
        SignInMode::Hosted,
        RetryBackoffMode::DeterministicTest,
        default_src.clone(),
        1_760_000_000,
    )
    .expect("open durable clinic store");
    seed_vertical_slice(&mut service1);
    service1
        .withdraw_source(
            &WorkspaceId::new("ws-a1").expect("ws-a1"),
            &SourceKey::new("src-withdrawn-9").expect("src-withdrawn-9"),
        )
        .expect("withdraw source");

    let server1 = TcpClinicServer::bind("127.0.0.1:0", service1).expect("bind TCP server 1");
    let addr1 = server1.local_addr();

    // -------------------------------------------------------------------------
    // 1. Subpath mounting (`308`/`404`), strict CSP, and Nix cache hygiene
    // -------------------------------------------------------------------------
    let root_404 = send_tcp_request(addr1, "GET", "/", &[], "");
    assert_eq!(root_404.status, 404);
    assert!(
        root_404
            .header("cache-control")
            .unwrap_or_default()
            .contains("no-store")
    );

    let evil_prefix_404 = send_tcp_request(addr1, "GET", "/elysianfields-evil/", &[], "");
    assert_eq!(evil_prefix_404.status, 404);

    let bare_redirect = send_tcp_request(addr1, "GET", "/elysianfields?patient=ws-a1", &[], "");
    assert_eq!(bare_redirect.status, 308);
    assert_eq!(
        bare_redirect.header("location"),
        Some("/elysianfields/?patient=ws-a1")
    );

    let entry_html = send_tcp_request(
        addr1,
        "GET",
        "/elysianfields/",
        &[("If-Modified-Since", "Thu, 01 Jan 1970 00:00:00 GMT")],
        "",
    );
    assert_eq!(entry_html.status, 200);
    assert_eq!(entry_html.header("cache-control"), Some("no-cache"));
    assert_eq!(entry_html.header("last-modified"), None);
    assert_eq!(
        entry_html.header("content-security-policy"),
        Some(STRICT_CSP)
    );
    assert_eq!(entry_html.header("x-frame-options"), Some("DENY"));
    assert_eq!(entry_html.header("x-content-type-options"), Some("nosniff"));

    // -------------------------------------------------------------------------
    // 2. Invite-only sign-in anti-enumeration, single-use verify, session check
    // -------------------------------------------------------------------------
    let no_session = send_tcp_request(addr1, "GET", "/elysianfields/api/auth/session", &[], "");
    assert_eq!(no_session.status, 401);
    assert_eq!(no_session.json()["reason"], "no_session");
    assert!(
        no_session
            .header("cache-control")
            .unwrap_or_default()
            .contains("no-store")
    );

    let unknown_req = send_tcp_request(
        addr1,
        "POST",
        "/elysianfields/api/auth/request-link",
        &[],
        r#"{"email":"nobody@example.invalid"}"#,
    );
    let deactivated_req = send_tcp_request(
        addr1,
        "POST",
        "/elysianfields/api/auth/request-link",
        &[],
        r#"{"email":"deactivated.a@example.invalid"}"#,
    );
    let active_req = send_tcp_request(
        addr1,
        "POST",
        "/elysianfields/api/auth/request-link",
        &[],
        r#"{"email":"clinician.a@example.invalid"}"#,
    );
    assert_eq!(unknown_req.status, 200);
    assert_eq!(deactivated_req.status, 200);
    assert_eq!(active_req.status, 200);
    // Identical hosted anti-enumeration payload across unknown, deactivated, and active emails.
    assert_eq!(unknown_req.body, deactivated_req.body);
    assert_eq!(deactivated_req.body, active_req.body);
    assert_eq!(
        active_req.json()["message"],
        HOSTED_ANTI_ENUMERATION_MESSAGE
    );

    // Outbound mail emitted strictly for the active account.
    let issued_token = {
        let svc_arc = server1.service();
        let guard = svc_arc.lock().expect("lock service 1");
        assert_eq!(guard.auth().sent_mail().len(), 1);
        guard.auth().sent_mail()[0].token.clone()
    };

    let verify_ok = send_tcp_request(
        addr1,
        "POST",
        "/elysianfields/api/auth/verify",
        &[],
        &format!(r#"{{"email":"clinician.a@example.invalid","token":"{issued_token}"}}"#),
    );
    assert_eq!(verify_ok.status, 200);
    let session_id = verify_ok.json()["session"]
        .as_str()
        .expect("minted session id")
        .to_owned();

    // Replay of single-use verification token fails closed with `session_ended`.
    let verify_replay = send_tcp_request(
        addr1,
        "POST",
        "/elysianfields/api/auth/verify",
        &[],
        &format!(r#"{{"email":"clinician.a@example.invalid","token":"{issued_token}"}}"#),
    );
    assert_eq!(verify_replay.status, 401);
    assert_eq!(verify_replay.json()["reason"], "session_ended");

    let auth_header = format!("Bearer {session_id}");
    let auth_headers = [("Authorization", auth_header.as_str())];

    let session_ok = send_tcp_request(
        addr1,
        "GET",
        "/elysianfields/api/auth/session",
        &auth_headers,
        "",
    );
    assert_eq!(session_ok.status, 200);
    assert_eq!(session_ok.json()["physician_id"], "phys-a1");

    // -------------------------------------------------------------------------
    // 3. Three-tier patient access + citations [1] & [12] (5,000-row table)
    // -------------------------------------------------------------------------
    let patients = send_tcp_request(
        addr1,
        "GET",
        "/elysianfields/api/patients",
        &auth_headers,
        "",
    );
    assert_eq!(patients.status, 200);
    let roster_json = patients.json();
    assert_eq!(roster_json["kind"], "loaded");
    assert_eq!(
        roster_json["patients"].as_array().expect("patients").len(),
        2
    );

    // Cross-practice workspace returns 404 (non-disclosure).
    let cross_practice_ws = send_tcp_request(
        addr1,
        "GET",
        "/elysianfields/api/patients/ws-b1/workspace",
        &auth_headers,
        "",
    );
    assert_eq!(cross_practice_ws.status, 404);

    // ClinicDemographic workspace returns demographic summary only (no clinical citations).
    let demo_ws = send_tcp_request(
        addr1,
        "GET",
        "/elysianfields/api/patients/ws-a2/workspace",
        &auth_headers,
        "",
    );
    assert_eq!(demo_ws.status, 200);
    assert_eq!(demo_ws.json()["membership"], "clinic_demographic");
    assert!(demo_ws.json().get("chats").is_none());

    let demo_cite_403 = send_tcp_request(
        addr1,
        "GET",
        "/elysianfields/api/patients/ws-a2/citations/src-note-1",
        &auth_headers,
        "",
    );
    assert_eq!(demo_cite_403.status, 403);

    // Citation [1] progress note + citation [12] 5,000-row structured lab table.
    let cite_1 = send_tcp_request(
        addr1,
        "GET",
        "/elysianfields/api/patients/ws-a1/citations/src-note-1",
        &auth_headers,
        "",
    );
    assert_eq!(cite_1.status, 200);
    assert_eq!(cite_1.json()["number"], 1);
    assert_eq!(cite_1.json()["status_class"], "settled");

    let cite_12 = send_tcp_request(
        addr1,
        "GET",
        "/elysianfields/api/patients/ws-a1/citations/src-lab-12",
        &auth_headers,
        "",
    );
    assert_eq!(cite_12.status, 200);
    assert_eq!(cite_12.json()["number"], 12);
    assert_eq!(
        cite_12.json()["fields"].as_array().expect("lab rows").len(),
        5_000
    );

    // Cross-patient citation isolation (404) & withdrawn citation redaction (404, zero clinical text).
    let cross_cite = send_tcp_request(
        addr1,
        "GET",
        "/elysianfields/api/patients/ws-a1/citations/src-other-b1",
        &auth_headers,
        "",
    );
    assert_eq!(cross_cite.status, 404);

    let withdrawn_cite = send_tcp_request(
        addr1,
        "GET",
        "/elysianfields/api/patients/ws-a1/citations/src-withdrawn-9",
        &auth_headers,
        "",
    );
    assert_eq!(withdrawn_cite.status, 404);
    assert_eq!(withdrawn_cite.json()["status"], "unavailable");
    assert!(!withdrawn_cite.body.contains("Creatinine 1.02"));

    // -------------------------------------------------------------------------
    // 4. Streamed SSE clinical answer + idempotent replay
    // -------------------------------------------------------------------------
    let submit_body = r#"{
        "type": "message.submit",
        "idempotency_key": "idem-turn-1",
        "payload": {
            "patient_workspace_id": "ws-a1",
            "chat_id": "chat-1",
            "text": "Review renal clearance trend."
        }
    }"#;
    let submit1 = send_tcp_request(
        addr1,
        "POST",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-1/commands",
        &auth_headers,
        submit_body,
    );
    assert_eq!(submit1.status, 200);
    assert_eq!(submit1.json()["deduplicated"], false);
    let wi_1 = submit1.json()["work_item_id"]
        .as_str()
        .expect("wi-1")
        .to_owned();

    let submit1_replay = send_tcp_request(
        addr1,
        "POST",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-1/commands",
        &auth_headers,
        submit_body,
    );
    assert_eq!(submit1_replay.status, 200);
    assert_eq!(submit1_replay.json()["deduplicated"], true);
    assert_eq!(submit1_replay.json()["work_item_id"], wi_1);

    let sse1 = send_tcp_request(
        addr1,
        "GET",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-1/stream",
        &auth_headers,
        "",
    );
    assert_eq!(sse1.status, 200);
    assert_eq!(
        sse1.header("content-type"),
        Some("text/event-stream; charset=utf-8")
    );
    assert!(
        sse1.header("cache-control")
            .unwrap_or_default()
            .contains("no-store")
    );
    assert!(sse1.body.contains("event: step_recorded"));
    assert!(sse1.body.contains("event: delta_chunk"));
    assert!(sse1.body.contains("event: completed"));
    assert!(sse1.body.contains(": keep-alive"));

    // -------------------------------------------------------------------------
    // 5. Mid-stream cancel (`work.stop`) preserving partial text
    // -------------------------------------------------------------------------
    {
        let mut cancel_provider = StubProvider::clinical_two_round_fixture(default_src.clone());
        cancel_provider.cancel_after_chunks = Some(2);
        let svc_arc = server1.service();
        svc_arc
            .lock()
            .expect("lock")
            .enqueue_provider_script(cancel_provider);
    }

    let submit_cancel = send_tcp_request(
        addr1,
        "POST",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-1/commands",
        &auth_headers,
        r#"{
            "type": "message.submit",
            "idempotency_key": "idem-turn-2-cancel",
            "payload": {
                "patient_workspace_id": "ws-a1",
                "chat_id": "chat-1",
                "text": "Draft prolonged summary to cancel mid-stream."
            }
        }"#,
    );
    assert_eq!(submit_cancel.status, 200);
    let wi_2 = submit_cancel.json()["work_item_id"]
        .as_str()
        .expect("wi-2")
        .to_owned();

    let stop_resp = send_tcp_request(
        addr1,
        "POST",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-1/commands",
        &auth_headers,
        &format!(
            r#"{{
                "type": "work.stop",
                "idempotency_key": "idem-stop-2",
                "payload": {{
                    "patient_workspace_id": "ws-a1",
                    "chat_id": "chat-1",
                    "work_item_id": "{wi_2}"
                }}
            }}"#
        ),
    );
    assert_eq!(stop_resp.status, 200);

    let snap_after_stop = send_tcp_request(
        addr1,
        "GET",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-1",
        &auth_headers,
        "",
    );
    assert_eq!(snap_after_stop.status, 200);
    let ex2 = &snap_after_stop.json()["exchanges"][1];
    assert_eq!(ex2["answer"]["kind"], "stopped");
    assert!(
        !ex2["kept_partial_text"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );

    // -------------------------------------------------------------------------
    // 6. Bounded 5-failure retry exhaustion -> `CouldntFinish` -> `Try again`
    // -------------------------------------------------------------------------
    {
        let mut failing_provider = StubProvider::clinical_two_round_fixture(default_src.clone());
        failing_provider.remaining_failures = 5;
        let svc_arc = server1.service();
        svc_arc
            .lock()
            .expect("lock")
            .enqueue_provider_script(failing_provider);
    }

    let submit_fault = send_tcp_request(
        addr1,
        "POST",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-1/commands",
        &auth_headers,
        r#"{
            "type": "message.submit",
            "idempotency_key": "idem-turn-3-fault",
            "payload": {
                "patient_workspace_id": "ws-a1",
                "chat_id": "chat-1",
                "text": "Trigger 5 transient upstream failures then recover."
            }
        }"#,
    );
    assert_eq!(submit_fault.status, 200);
    let wi_3 = submit_fault.json()["work_item_id"]
        .as_str()
        .expect("wi-3")
        .to_owned();

    let snap_after_fault = send_tcp_request(
        addr1,
        "GET",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-1",
        &auth_headers,
        "",
    );
    let ex3 = &snap_after_fault.json()["exchanges"][2];
    assert_eq!(ex3["failed_attempts"], 5);
    assert_eq!(ex3["answer"]["kind"], "couldnt_finish");
    let expected_run_id = ex3["answer"]["try_again"]["expected_run_id"]
        .as_str()
        .expect("try_again expected_run_id")
        .to_owned();

    // Stale `expected_run_id` returns 409 Conflict.
    let stale_resume = send_tcp_request(
        addr1,
        "POST",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-1/commands",
        &auth_headers,
        &format!(
            r#"{{
                "type": "work.resume",
                "idempotency_key": "idem-resume-3-stale",
                "payload": {{
                    "patient_workspace_id": "ws-a1",
                    "chat_id": "chat-1",
                    "work_item_id": "{wi_3}",
                    "expected_run_id": "run-stale-999"
                }}
            }}"#
        ),
    );
    assert_eq!(stale_resume.status, 409);

    // Explicit `Try again` (`work.resume` with live `expected_run_id`) succeeds on attempt 6.
    let try_again_ok = send_tcp_request(
        addr1,
        "POST",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-1/commands",
        &auth_headers,
        &format!(
            r#"{{
                "type": "work.resume",
                "idempotency_key": "idem-resume-3-ok",
                "payload": {{
                    "patient_workspace_id": "ws-a1",
                    "chat_id": "chat-1",
                    "work_item_id": "{wi_3}",
                    "expected_run_id": "{expected_run_id}"
                }}
            }}"#
        ),
    );
    assert_eq!(try_again_ok.status, 200);

    let snap_recovered = send_tcp_request(
        addr1,
        "GET",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-1",
        &auth_headers,
        "",
    );
    assert_eq!(
        snap_recovered.json()["exchanges"][2]["answer"]["kind"],
        "finished"
    );

    // SSE stream carries `cancelled`, `couldnt_finish`, and `?resync=true` event.
    let sse_resync = send_tcp_request(
        addr1,
        "GET",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-1/stream?resync=true",
        &auth_headers,
        "",
    );
    assert_eq!(sse_resync.status, 200);
    assert!(sse_resync.body.contains("event: cancelled"));
    assert!(sse_resync.body.contains("event: couldnt_finish"));
    assert!(sse_resync.body.contains("event: resync"));

    // -------------------------------------------------------------------------
    // 7. Optimistic-concurrency draft autosave & 409 Conflict
    // -------------------------------------------------------------------------
    let draft_v1 = send_tcp_request(
        addr1,
        "PUT",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-1/draft",
        &auth_headers,
        r#"{"text":"Follow up repeat BMP in 2 weeks.","expected_revision":0}"#,
    );
    assert_eq!(draft_v1.status, 200);
    assert_eq!(draft_v1.json()["revision"], 1);

    let draft_conflict = send_tcp_request(
        addr1,
        "PUT",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-1/draft",
        &auth_headers,
        r#"{"text":"Stale tab overwrite attempt","expected_revision":0}"#,
    );
    assert_eq!(draft_conflict.status, 409);
    assert_eq!(draft_conflict.json()["current"]["revision"], 1);
    assert_eq!(
        draft_conflict.json()["current"]["text"],
        "Follow up repeat BMP in 2 weeks."
    );

    let draft_v2 = send_tcp_request(
        addr1,
        "PUT",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-1/draft",
        &auth_headers,
        r#"{"text":"Follow up repeat BMP + potassium in 2 weeks.","expected_revision":1}"#,
    );
    assert_eq!(draft_v2.status, 200);
    assert_eq!(draft_v2.json()["revision"], 2);

    // Oversized auth body (> 64 KiB) returns 413 Payload Too Large with `no-store`.
    let huge_auth = "x".repeat(AUTH_MAX_BODY_BYTES + 128);
    let too_large = send_tcp_request(
        addr1,
        "POST",
        "/elysianfields/api/auth/request-link",
        &[],
        &huge_auth,
    );
    assert_eq!(too_large.status, 413);
    assert!(
        too_large
            .header("cache-control")
            .unwrap_or_default()
            .contains("no-store")
    );

    // -------------------------------------------------------------------------
    // 8. Full server shutdown -> boot fresh server on same durable dir -> exact readback
    // -------------------------------------------------------------------------
    server1.shutdown();

    let service2 = ClinicService::open_durable(
        &wal_dir,
        SignInMode::Hosted,
        RetryBackoffMode::DeterministicTest,
        default_src,
        1_760_000_120,
    )
    .expect("reopen durable clinic store after restart");
    let server2 = TcpClinicServer::bind("127.0.0.1:0", service2).expect("bind TCP server 2");
    let addr2 = server2.local_addr();

    // Existing session survived full restart!
    let session_after_restart = send_tcp_request(
        addr2,
        "GET",
        "/elysianfields/api/auth/session",
        &auth_headers,
        "",
    );
    assert_eq!(session_after_restart.status, 200);
    assert_eq!(session_after_restart.json()["physician_id"], "phys-a1");

    // All 3 exchanges, steps, answers, and revision-2 draft read back identically!
    let snap_after_restart = send_tcp_request(
        addr2,
        "GET",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-1",
        &auth_headers,
        "",
    );
    assert_eq!(snap_after_restart.status, 200);
    let snap_json = snap_after_restart.json();
    let exchanges = snap_json["exchanges"]
        .as_array()
        .expect("3 persisted turns");
    assert_eq!(exchanges.len(), 3);
    assert_eq!(exchanges[0]["answer"]["kind"], "finished");
    assert_eq!(exchanges[1]["answer"]["kind"], "stopped");
    assert_eq!(exchanges[2]["answer"]["kind"], "finished");
    assert_eq!(exchanges[2]["failed_attempts"], 5);
    assert_eq!(snap_json["draft"]["revision"], 2);
    assert_eq!(
        snap_json["draft"]["text"],
        "Follow up repeat BMP + potassium in 2 weeks."
    );

    // Idempotency key persisted across restart!
    let replay_after_restart = send_tcp_request(
        addr2,
        "POST",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-1/commands",
        &auth_headers,
        submit_body,
    );
    assert_eq!(replay_after_restart.status, 200);
    assert_eq!(replay_after_restart.json()["deduplicated"], true);
    assert_eq!(replay_after_restart.json()["work_item_id"], wi_1);

    // Sign-out revokes session immediately and refuses subsequent requests.
    let sign_out = send_tcp_request(
        addr2,
        "POST",
        "/elysianfields/api/auth/sign-out",
        &auth_headers,
        "",
    );
    assert_eq!(sign_out.status, 200);
    let post_sign_out = send_tcp_request(
        addr2,
        "GET",
        "/elysianfields/api/auth/session",
        &auth_headers,
        "",
    );
    assert_eq!(post_sign_out.status, 401);
    assert_eq!(post_sign_out.json()["reason"], "session_ended");

    server2.shutdown();
    let _ = fs::remove_dir_all(&wal_dir);
}

#[test]
fn deactivated_account_and_sse_last_event_id_filtering_over_tcp() {
    let wal_dir = fresh_temp_dir("auth-and-sse");
    let default_src = SourceKey::new("src-note-1").expect("src-note-1");

    let mut service = ClinicService::open_durable(
        &wal_dir,
        SignInMode::LocalProof,
        RetryBackoffMode::DeterministicTest,
        default_src,
        1_760_001_000,
    )
    .expect("open durable store");
    seed_vertical_slice(&mut service);

    let server = TcpClinicServer::bind("127.0.0.1:0", service).expect("bind TCP server");
    let addr = server.local_addr();

    // LocalProof mode issues a one-time token directly for an active clinician.
    let init = send_tcp_request(
        addr,
        "POST",
        "/elysianfields/api/auth/request-link",
        &[],
        r#"{"email":"clinician.a@example.invalid"}"#,
    );
    assert_eq!(init.status, 200);
    let token = init.json()["token"].as_str().expect("token").to_owned();

    let verify = send_tcp_request(
        addr,
        "POST",
        "/elysianfields/api/auth/verify",
        &[],
        &format!(r#"{{"email":"clinician.a@example.invalid","token":"{token}"}}"#),
    );
    assert_eq!(verify.status, 200);
    let session_id = verify.json()["session"]
        .as_str()
        .expect("session")
        .to_owned();
    let auth_header = format!("Bearer {session_id}");
    let auth_headers = [("Authorization", auth_header.as_str())];

    // Submit one turn and verify `Last-Event-ID` filters earlier SSE frames.
    let submit = send_tcp_request(
        addr,
        "POST",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-sse/commands",
        &auth_headers,
        r#"{
            "type": "message.submit",
            "idempotency_key": "idem-sse-1",
            "payload": {
                "patient_workspace_id": "ws-a1",
                "chat_id": "chat-sse",
                "text": "Check SSE Last-Event-ID filtering."
            }
        }"#,
    );
    assert_eq!(submit.status, 200);

    let full_sse = send_tcp_request(
        addr,
        "GET",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-sse/stream",
        &auth_headers,
        "",
    );
    assert!(full_sse.body.contains("id: 1\n"));
    assert!(full_sse.body.contains("event: completed"));

    let tail_sse = send_tcp_request(
        addr,
        "GET",
        "/elysianfields/api/workspaces/ws-a1/chats/chat-sse/stream",
        &[
            ("Authorization", auth_header.as_str()),
            ("Last-Event-ID", "4"),
        ],
        "",
    );
    assert_eq!(tail_sse.status, 200);
    assert!(!tail_sse.body.contains("id: 1\n"));
    assert!(!tail_sse.body.contains("id: 4\n"));
    assert!(tail_sse.body.contains("event: completed"));

    // Live account deactivation immediately refuses requests with 403 `access_removed`.
    {
        let svc_arc = server.service();
        let email = EmailAddress::parse("clinician.a@example.invalid").expect("email");
        svc_arc
            .lock()
            .expect("lock")
            .deactivate_account(&email)
            .expect("deactivate");
    }

    let refused = send_tcp_request(
        addr,
        "GET",
        "/elysianfields/api/auth/session",
        &auth_headers,
        "",
    );
    assert_eq!(refused.status, 403);
    assert_eq!(refused.json()["reason"], "access_removed");

    // Verify Postgres SQL migration file exists and defines core M2 tables.
    let sql = include_str!("../migrations/0001_m2_clinic_slice.sql");
    for required_table in [
        "physician_accounts",
        "sessions",
        "patient_workspaces",
        "cited_sources",
        "chat_exchanges",
        "composer_drafts",
    ] {
        assert!(
            sql.contains(required_table),
            "migration missing table `{required_table}`"
        );
    }

    server.shutdown();
    let _ = fs::remove_dir_all(&wal_dir);
}
