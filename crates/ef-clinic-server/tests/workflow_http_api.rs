#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::time::Duration;

use ef_clinic_domain::auth::{
    AccountStanding, EmailAddress, PhysicianAccount, PhysicianRole, SignInMode,
};
use ef_clinic_domain::chat::RetryBackoffMode;
use ef_clinic_server::{ClinicService, TcpClinicServer};
use ef_clinic_workflows::today::{TodayItemKind, TodayQueueItem, TodayUrgency};
use ef_core::{PhysicianId, PracticeId, SourceKey, WorkspaceId, assert_zero_phi};
use serde_json::{Value, json};

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

    fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|err| panic!("invalid JSON body ({err}): {}", self.body))
    }
}

fn send_http(
    addr: SocketAddr,
    method: &str,
    target: &str,
    token: Option<&str>,
    request_id: Option<&str>,
    body: Option<&Value>,
) -> RawResp {
    let mut stream = TcpStream::connect(addr).expect("connect to TcpClinicServer");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set read timeout");
    let body_bytes = body.map(|v| serde_json::to_vec(v).expect("serialize JSON body"));

    let mut raw = format!("{method} {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n");
    if let Some(tok) = token {
        raw.push_str(&format!("Authorization: Bearer {tok}\r\n"));
    }
    if let Some(rid) = request_id {
        raw.push_str(&format!("X-Request-Id: {rid}\r\n"));
    }
    if let Some(ref bytes) = body_bytes {
        raw.push_str("Content-Type: application/json\r\n");
        raw.push_str(&format!("Content-Length: {}\r\n", bytes.len()));
    }
    raw.push_str("Connection: close\r\n\r\n");

    stream.write_all(raw.as_bytes()).expect("write head");
    if let Some(ref bytes) = body_bytes {
        stream.write_all(bytes).expect("write body");
    }
    stream.flush().expect("flush");
    let _ = stream.shutdown(Shutdown::Write);

    let mut resp_bytes = Vec::new();
    stream.read_to_end(&mut resp_bytes).expect("read response");
    let text = String::from_utf8(resp_bytes).expect("utf8 response");
    let (head, body_str) = text.split_once("\r\n\r\n").expect("split HTTP response");
    let mut lines = head.lines();
    let status_line = lines.next().expect("status line");
    let status: u16 = status_line
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
        body: body_str.to_owned(),
    }
}

fn sign_in_physician(server: &TcpClinicServer, email: &str) -> String {
    let addr = server.local_addr();
    let req_link = send_http(
        addr,
        "POST",
        "/elysianfields/api/auth/request-link",
        None,
        None,
        Some(&json!({ "email": email })),
    );
    assert_eq!(
        req_link.status, 200,
        "request-link failed: {}",
        req_link.body
    );
    let token = {
        let svc = server.service();
        let guard = svc.lock().expect("lock service");
        guard
            .auth()
            .sent_mail()
            .last()
            .expect("issued verification mail")
            .token
            .clone()
    };

    let verify = send_http(
        addr,
        "POST",
        "/elysianfields/api/auth/verify",
        None,
        None,
        Some(&json!({ "email": email, "token": token })),
    );
    assert_eq!(verify.status, 200, "verify failed: {}", verify.body);
    verify.json()["session"]
        .as_str()
        .expect("session in verify response")
        .to_owned()
}

#[test]
fn clinic_workflow_http_apis_enforce_auth_isolation_standing_memory_invites_and_zero_phi_telemetry()
{
    let practice_id = PracticeId::new("prac-w18").expect("practice");
    let phys_member = PhysicianId::new("phys-member").expect("member physician");
    let phys_outsider = PhysicianId::new("phys-outsider").expect("outsider physician");
    let workspace_id = WorkspaceId::new("ws-patient-42").expect("workspace");

    let mut service = ClinicService::new_in_memory(
        SignInMode::Hosted,
        RetryBackoffMode::Hosted,
        SourceKey::new("src-lab-1").expect("source"),
        1_760_000_000,
    );
    service
        .seed_account(PhysicianAccount {
            physician_id: phys_member.clone(),
            practice_id: practice_id.clone(),
            email: EmailAddress::parse("member@clinic.example").expect("email"),
            display_name: "Dr. Member".to_owned(),
            role: PhysicianRole::Standard,
            standing: AccountStanding::Active,
            sessions_ended_before: None,
        })
        .expect("seed member");
    service
        .seed_account(PhysicianAccount {
            physician_id: phys_outsider.clone(),
            practice_id: practice_id.clone(),
            email: EmailAddress::parse("outsider@clinic.example").expect("email"),
            display_name: "Dr. Outsider".to_owned(),
            role: PhysicianRole::Standard,
            standing: AccountStanding::Active,
            sessions_ended_before: None,
        })
        .expect("seed outsider");
    service
        .seed_workspace(
            workspace_id.clone(),
            practice_id,
            "Elena Vance",
            vec![phys_member.clone()],
        )
        .expect("seed workspace");
    service
        .seed_demo_workflows(&workspace_id, "Elena Vance")
        .expect("seed demo workflows");

    let server = TcpClinicServer::bind("127.0.0.1:0", service).expect("bind clinic server");
    let addr = server.local_addr();

    // 1. Unauthenticated access to all 4 workflow endpoints must return 401 + ClinicalNoStore + X-Request-Id.
    for (method, path, body) in [
        ("GET", "/elysianfields/api/today", None),
        (
            "POST",
            "/elysianfields/api/today/standing",
            Some(
                json!({ "item_id": "today-urgent-ws-patient-42", "standing": "seen", "version_id": 1 }),
            ),
        ),
        (
            "GET",
            "/elysianfields/api/workspaces/ws-patient-42/memory",
            None,
        ),
        (
            "POST",
            "/elysianfields/api/workspaces/ws-patient-42/connected-care/invitations",
            Some(json!({ "patient_id": "pat-42", "expected_dob": "1985-04-12" })),
        ),
    ] {
        let unauth = send_http(addr, method, path, None, None, body.as_ref());
        assert_eq!(
            unauth.status, 401,
            "{method} {path} expected 401, got {}",
            unauth.status
        );
        assert_eq!(
            unauth.header("cache-control"),
            Some("private, no-store, max-age=0")
        );
        assert_eq!(unauth.header("cdn-cache-control"), Some("no-store"));
        assert!(unauth.header("x-request-id").is_some());
    }

    let member_token = sign_in_physician(&server, "member@clinic.example");
    let outsider_token = sign_in_physician(&server, "outsider@clinic.example");

    // 2. Membership isolation: non-member physician receives 403 on workspace memory & connected-care invite.
    let outsider_mem = send_http(
        addr,
        "GET",
        "/elysianfields/api/workspaces/ws-patient-42/memory",
        Some(&outsider_token),
        None,
        None,
    );
    assert_eq!(
        outsider_mem.status, 403,
        "outsider memory: {}",
        outsider_mem.body
    );

    let outsider_inv = send_http(
        addr,
        "POST",
        "/elysianfields/api/workspaces/ws-patient-42/connected-care/invitations",
        Some(&outsider_token),
        None,
        Some(&json!({ "patient_id": "pat-42", "expected_dob": "1985-04-12" })),
    );
    assert_eq!(
        outsider_inv.status, 403,
        "outsider invite: {}",
        outsider_inv.body
    );

    // 3. GET /elysianfields/api/today returns seeded items, scheduled_visits, channel_banners + echoes X-Request-Id.
    let today_initial = send_http(
        addr,
        "GET",
        "/elysianfields/api/today",
        Some(&member_token),
        Some("ef-w18-trace-99"),
        None,
    );
    assert_eq!(today_initial.status, 200);
    assert_eq!(
        today_initial.header("cache-control"),
        Some("private, no-store, max-age=0")
    );
    assert_eq!(
        today_initial.header("x-request-id"),
        Some("ef-w18-trace-99")
    );
    let today_json = today_initial.json();
    let items = today_json["items"].as_array().expect("items array");
    assert_eq!(items.len(), 2);
    assert_eq!(
        items[0]["item"]["item_id"].as_str(),
        Some("today-urgent-ws-patient-42")
    );
    assert_eq!(items[0]["seen"].as_bool(), Some(false));
    assert_eq!(items[0]["done"].as_bool(), Some(false));
    assert_eq!(
        today_json["scheduled_visits"]
            .as_array()
            .expect("scheduled_visits")
            .len(),
        1
    );
    assert!(today_json["channel_banners"].is_array());

    // 4. POST /elysianfields/api/today/standing: mark seen -> done -> bump version_id -> item resurfaces.
    let seen_resp = send_http(
        addr,
        "POST",
        "/elysianfields/api/today/standing",
        Some(&member_token),
        None,
        Some(&json!({
            "item_id": "today-urgent-ws-patient-42",
            "standing": "seen",
            "version_id": 1
        })),
    );
    assert_eq!(seen_resp.status, 200, "seen failed: {}", seen_resp.body);
    assert_eq!(seen_resp.json()["standing"].as_str(), Some("seen"));

    let after_seen = send_http(
        addr,
        "GET",
        "/elysianfields/api/today",
        Some(&member_token),
        None,
        None,
    )
    .json();
    let urgent_after_seen = after_seen["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|e| e["item"]["item_id"] == "today-urgent-ws-patient-42")
        .expect("urgent item");
    assert_eq!(urgent_after_seen["seen"].as_bool(), Some(true));
    assert_eq!(urgent_after_seen["done"].as_bool(), Some(false));

    let done_resp = send_http(
        addr,
        "POST",
        "/elysianfields/api/today/standing",
        Some(&member_token),
        None,
        Some(&json!({
            "item_id": "today-urgent-ws-patient-42",
            "standing": "done",
            "version_id": 1
        })),
    );
    assert_eq!(done_resp.status, 200, "done failed: {}", done_resp.body);
    assert_eq!(done_resp.json()["standing"].as_str(), Some("done"));

    let after_done = send_http(
        addr,
        "GET",
        "/elysianfields/api/today",
        Some(&member_token),
        None,
        None,
    )
    .json();
    let active_after_done = after_done["items"].as_array().expect("items");
    assert!(
        !active_after_done
            .iter()
            .any(|e| e["item"]["item_id"] == "today-urgent-ws-patient-42"),
        "item marked done at version 1 should leave the active Today queue until version advances"
    );

    // Advance clinical version_id from 1 -> 2 and verify honest resurfacing (done=false, seen=false).
    server
        .service()
        .lock()
        .expect("lock service")
        .seed_today_item(TodayQueueItem {
            item_id: "today-urgent-ws-patient-42".to_owned(),
            workspace_id: Some(workspace_id.clone()),
            urgency: TodayUrgency::Now,
            waiting_since_epoch_secs: 1_759_999_500,
            kind: TodayItemKind::UrgentClinicalConcern,
            version_id: 2,
            summary: "Repeat potassium 5.9 mEq/L on stat BMP for Elena Vance".to_owned(),
            can_mark_done: true,
        })
        .expect("bump version");

    let resurfaced = send_http(
        addr,
        "GET",
        "/elysianfields/api/today",
        Some(&member_token),
        None,
        None,
    )
    .json();
    let urgent_resurfaced = resurfaced["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|e| e["item"]["item_id"] == "today-urgent-ws-patient-42")
        .expect("resurfaced urgent item");
    assert_eq!(urgent_resurfaced["item"]["version_id"].as_u64(), Some(2));
    assert_eq!(urgent_resurfaced["seen"].as_bool(), Some(false));
    assert_eq!(urgent_resurfaced["done"].as_bool(), Some(false));

    // 5. GET /elysianfields/api/workspaces/{workspace_id}/memory returns ChartRecord + PhysicianInstruction.
    let mem_resp = send_http(
        addr,
        "GET",
        "/elysianfields/api/workspaces/ws-patient-42/memory",
        Some(&member_token),
        Some("ef-w18-mem-1"),
        None,
    );
    assert_eq!(mem_resp.status, 200, "memory failed: {}", mem_resp.body);
    assert_eq!(
        mem_resp.header("cache-control"),
        Some("private, no-store, max-age=0")
    );
    assert_eq!(mem_resp.header("x-request-id"), Some("ef-w18-mem-1"));
    let mem_json = mem_resp.json();
    assert_eq!(mem_json["workspace_id"].as_str(), Some("ws-patient-42"));
    assert!(mem_json["rounds_watermark"].is_u64());
    let entries = mem_json["entries"].as_array().expect("memory entries");
    assert_eq!(entries.len(), 2);
    assert!(
        entries
            .iter()
            .any(|e| e["authority"].as_str() == Some("chart_record")),
        "expected ChartRecord entry in {entries:?}"
    );
    assert!(
        entries
            .iter()
            .any(|e| e["authority"].as_str() == Some("physician_instruction")),
        "expected PhysicianInstruction entry in {entries:?}"
    );

    // 6. POST /elysianfields/api/workspaces/{workspace_id}/connected-care/invitations -> 201 Created + EF-XXXXXX.
    let bad_dob = send_http(
        addr,
        "POST",
        "/elysianfields/api/workspaces/ws-patient-42/connected-care/invitations",
        Some(&member_token),
        None,
        Some(&json!({ "patient_id": "pat-42", "expected_dob": "not-a-date" })),
    );
    assert_eq!(bad_dob.status, 400);

    let invite_resp = send_http(
        addr,
        "POST",
        "/elysianfields/api/workspaces/ws-patient-42/connected-care/invitations",
        Some(&member_token),
        Some("ef-w18-inv-1"),
        Some(&json!({
            "patient_id": "pat-42",
            "expected_dob": "1985-04-12"
        })),
    );
    assert_eq!(
        invite_resp.status, 201,
        "invite failed: {}",
        invite_resp.body
    );
    assert_eq!(
        invite_resp.header("cache-control"),
        Some("private, no-store, max-age=0")
    );
    assert_eq!(invite_resp.header("x-request-id"), Some("ef-w18-inv-1"));
    let invite_json = invite_resp.json();
    assert!(
        invite_json["invitation_id"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
    );
    let invite_code = invite_json["invite_code"].as_str().expect("invite_code");
    assert!(
        invite_code.starts_with("EF-") && invite_code.len() == 9,
        "expected EF-XXXXXX invite code, got {invite_code}"
    );
    assert_eq!(invite_json["workspace_id"].as_str(), Some("ws-patient-42"));

    // 7. Zero-PHI route template redaction verified across all recorded access logs.
    let logs = server.access_logs();
    assert!(
        logs.iter()
            .any(|ev| ev.route_template.as_str() == "/elysianfields/api/workspaces/:id/memory"),
        "expected redacted /elysianfields/api/workspaces/:id/memory route template in {logs:?}"
    );
    assert!(
        logs.iter().any(|ev| {
            ev.route_template.as_str()
                == "/elysianfields/api/workspaces/:id/connected-care/invitations"
        }),
        "expected redacted connected-care invitation route template in {logs:?}"
    );
    for ev in &logs {
        let line = ev.to_json_line();
        assert_eq!(assert_zero_phi(&line), Ok(()), "line={line}");
        assert!(
            !line.contains("ws-patient-42"),
            "raw workspace ID leaked into telemetry: {line}"
        );
    }

    server.shutdown();
}
