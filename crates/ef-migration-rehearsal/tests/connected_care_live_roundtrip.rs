#![cfg(not(target_arch = "wasm32"))]

//! Live cross-product Connected Care round-trip E2E across all 3 real loopback TCP servers:
//! - `TcpClinicServer` (seeded with `seed_demo_workflows()`)
//! - `TcpMobileServer` (durable WAL + independent `FileReceiptJournal`)
//! - `TcpPublicSiteServer` (`PublicSiteRouter::new(DeploymentMode::Production)`)
//!
//! Verifies end-to-end over real HTTP/1.1 TCP sockets:
//! 1. Physician Clinic sign-in, `GET /elysianfields/api/today`, `POST /elysianfields/api/today/standing`
//!    (`seen` & `done` persistence), `GET /elysianfields/api/workspaces/{id}/memory`, and
//!    `POST /elysianfields/api/workspaces/{id}/connected-care/invitations` (`201 Created`, `EF-XXXXXX`).
//! 2. Independent Patient Mobile sign-in, Connected Care lookup (zero preview leak before DOB),
//!    DOB verification, explicit consent share (`OnlyShareGrantsConsent`), idempotent curated
//!    "For My Doctor" transfer, offline sign-out (`SignInHoldsNothing`), owed sign-out flush,
//!    and `401` rejection of the revoked session token.
//! 3. Strict cross-product token isolation (Clinic bearer token -> `401` on Mobile API;
//!    Mobile bearer token -> `401` on Clinic API).
//! 4. Public Site searchable `/dei?state=CA&service=crisis` (crisis-first `988` banner + CA crisis
//!    facilities) and XSS-safe `/dei?q=%3Cscript%3Ealert(1)%3C/script%3E` escaping.
//! 5. End-to-end `X-Request-Id` echo and zero-PHI structured access logs across all 3 servers.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use ef_clinic_domain::auth::{
    AccountStanding, EmailAddress, PhysicianAccount, PhysicianRole, SignInMode,
};
use ef_clinic_domain::chat::RetryBackoffMode;
use ef_clinic_server::{ClinicService, TcpClinicServer};
use ef_connected_care::{
    ConfidentialityTag, CreateInvitationRequest, DateOfBirth, EnvelopeItem, InviteCredential,
    PairKey,
};
use ef_core::{PatientId, PhysicianId, PracticeId, REQUEST_ID_HEADER, SourceKey, WorkspaceId};
use ef_mobile_domain::FileReceiptJournal;
use ef_mobile_server::{MobileService, TcpMobileServer};
use ef_public_site::{
    DeploymentMode, PublicSiteRouter, TcpPublicSiteServer, crisis_lines_appear_before_listings,
};
use serde_json::Value;

#[derive(Debug, Clone, Copy)]
enum HttpVerb {
    Get,
    Post,
}

impl HttpVerb {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

struct HttpResp {
    status: u16,
    headers: BTreeMap<String, String>,
    body: String,
}

impl HttpResp {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|err| panic!("expected JSON response, got `{}`: {err}", self.body))
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

fn tcp_request(
    addr: SocketAddr,
    verb: HttpVerb,
    target: &str,
    bearer: Option<&str>,
    request_id: Option<&str>,
    body: &str,
) -> HttpResp {
    let mut stream = TcpStream::connect(addr).expect("connect loopback tcp server");
    let mut raw = format!("{} {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n", verb.as_str());
    if let Some(tok) = bearer {
        raw.push_str(&format!("Authorization: Bearer {tok}\r\n"));
    }
    if let Some(req_id) = request_id {
        raw.push_str(&format!("{REQUEST_ID_HEADER}: {req_id}\r\n"));
    }
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
    let text = String::from_utf8(bytes).expect("valid utf8 response");
    let (head, body_part) = text.split_once("\r\n\r\n").expect("http header/body split");
    let mut lines = head.split("\r\n");
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
    HttpResp {
        status,
        headers,
        body: body_part.to_owned(),
    }
}

struct TempDirGuard(PathBuf);

impl TempDirGuard {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "ef-cc-roundtrip-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn connected_care_live_three_server_roundtrip_isolation_and_zero_phi_logs() {
    let clinic_dir = TempDirGuard::new("clinic-wal");
    let mobile_dir = TempDirGuard::new("mobile-wal");
    let journal_dir = TempDirGuard::new("mobile-journal");

    let now_secs = 1_760_400_000u64;
    let practice_id = PracticeId::new("prac-roundtrip-1").expect("practice id");
    let physician_id = PhysicianId::new("phys-roundtrip-1").expect("physician id");
    let workspace_id = WorkspaceId::new("ws-roundtrip-1").expect("workspace id");
    let patient_id = PatientId::new("pat-roundtrip-1").expect("patient id");
    let clinic_email = "dr.roundtrip@clinic.example.invalid";
    let mobile_email = "patient.roundtrip@mobile.example.invalid";
    let mobile_email_2 = "patient.iso@mobile.example.invalid";
    let curated_note_text =
        "Home blood pressure averaged 122/76 mmHg over 7 days with zero dizziness.";

    // 1. Boot real TcpClinicServer seeded with `seed_demo_workflows()`.
    let mut clinic_svc = ClinicService::open_durable(
        clinic_dir.path(),
        SignInMode::LocalProof,
        RetryBackoffMode::DeterministicTest,
        SourceKey::new("src-roundtrip-1").expect("default src"),
        now_secs,
    )
    .expect("open durable clinic service");
    clinic_svc
        .seed_account(PhysicianAccount {
            physician_id: physician_id.clone(),
            practice_id: practice_id.clone(),
            email: EmailAddress::parse(clinic_email).expect("clinic email"),
            display_name: "Dr. Roundtrip".to_owned(),
            role: PhysicianRole::Standard,
            standing: AccountStanding::Active,
            sessions_ended_before: None,
        })
        .expect("seed clinic account");
    clinic_svc
        .seed_workspace(
            workspace_id.clone(),
            practice_id.clone(),
            "Avery, Jordan (Roundtrip)",
            vec![physician_id.clone()],
        )
        .expect("seed clinic workspace");
    clinic_svc
        .seed_demo_workflows(&workspace_id, "Avery, Jordan (Roundtrip)")
        .expect("seed demo workflows");

    let clinic_srv = TcpClinicServer::bind("127.0.0.1:0", clinic_svc).expect("bind clinic tcp");
    let clinic_addr = clinic_srv.local_addr();

    // 2. Boot real TcpMobileServer and TcpPublicSiteServer over loopback TCP.
    let journal = FileReceiptJournal::open(journal_dir.path()).expect("open receipt journal");
    let mobile_svc =
        MobileService::open_durable(mobile_dir.path(), Box::new(journal), now_secs, false)
            .expect("open durable mobile service");
    let mobile_srv = TcpMobileServer::bind("127.0.0.1:0", mobile_svc).expect("bind mobile tcp");
    let mobile_addr = mobile_srv.local_addr();

    let public_srv = TcpPublicSiteServer::bind(
        "127.0.0.1:0",
        PublicSiteRouter::new(DeploymentMode::Production),
    )
    .expect("bind public site tcp");
    let public_addr = public_srv.local_addr();

    // 3. Physician signs into ef-clinic-server and exercises Today, Memory, and Connected Care Invite.
    let clinic_trace = "ef-rt-clinic-01";
    let unauth_today = tcp_request(
        clinic_addr,
        HttpVerb::Get,
        "/elysianfields/api/today",
        None,
        Some(clinic_trace),
        "",
    );
    assert_eq!(unauth_today.status, 401);
    assert_eq!(unauth_today.header(REQUEST_ID_HEADER), Some(clinic_trace));

    let c_req = tcp_request(
        clinic_addr,
        HttpVerb::Post,
        "/elysianfields/api/auth/request-link",
        None,
        Some(clinic_trace),
        &format!(r#"{{"email":"{clinic_email}"}}"#),
    );
    assert_eq!(c_req.status, 200);
    let c_tok = c_req.json()["token"]
        .as_str()
        .expect("clinic token")
        .to_owned();
    let c_ver = tcp_request(
        clinic_addr,
        HttpVerb::Post,
        "/elysianfields/api/auth/verify",
        None,
        Some(clinic_trace),
        &format!(r#"{{"email":"{clinic_email}","token":"{c_tok}"}}"#),
    );
    assert_eq!(c_ver.status, 200);
    let clinic_session = c_ver.json()["session"]
        .as_str()
        .expect("clinic session")
        .to_owned();

    // GET /elysianfields/api/today -> seeded Now + Today items and scheduled visit.
    let today_initial = tcp_request(
        clinic_addr,
        HttpVerb::Get,
        "/elysianfields/api/today",
        Some(&clinic_session),
        Some(clinic_trace),
        "",
    );
    assert_eq!(today_initial.status, 200);
    assert_eq!(today_initial.header(REQUEST_ID_HEADER), Some(clinic_trace));
    let today_json = today_initial.json();
    let items = today_json["items"].as_array().expect("today items array");
    let visits = today_json["scheduled_visits"]
        .as_array()
        .expect("scheduled_visits array");
    assert!(items.len() >= 2, "expected >= 2 seeded Today items");
    assert!(!visits.is_empty(), "expected >= 1 seeded scheduled visit");
    let urgent_item_id = items[0]["item"]["item_id"]
        .as_str()
        .expect("urgent item_id")
        .to_owned();
    let urgent_version_id = items[0]["item"]["version_id"]
        .as_u64()
        .expect("urgent version_id");
    assert_eq!(items[0]["item"]["urgency"], "now");
    assert_eq!(items[0]["seen"], false);

    // POST /elysianfields/api/today/standing -> seen persists, then done removes from active queue.
    let mark_seen = tcp_request(
        clinic_addr,
        HttpVerb::Post,
        "/elysianfields/api/today/standing",
        Some(&clinic_session),
        Some(clinic_trace),
        &serde_json::to_string(&serde_json::json!({
            "item_id": urgent_item_id,
            "standing": "seen",
            "version_id": urgent_version_id,
        }))
        .expect("seen json"),
    );
    assert_eq!(mark_seen.status, 200);
    assert_eq!(mark_seen.json()["standing"], "seen");

    let today_after_seen = tcp_request(
        clinic_addr,
        HttpVerb::Get,
        "/elysianfields/api/today",
        Some(&clinic_session),
        Some(clinic_trace),
        "",
    );
    assert_eq!(today_after_seen.status, 200);
    let seen_item = today_after_seen.json()["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|e| e["item"]["item_id"] == urgent_item_id)
        .cloned()
        .expect("seen item still visible");
    assert_eq!(seen_item["seen"], true);

    let mark_done = tcp_request(
        clinic_addr,
        HttpVerb::Post,
        "/elysianfields/api/today/standing",
        Some(&clinic_session),
        Some(clinic_trace),
        &serde_json::to_string(&serde_json::json!({
            "item_id": urgent_item_id,
            "standing": "done",
            "version_id": urgent_version_id,
        }))
        .expect("done json"),
    );
    assert_eq!(mark_done.status, 200);
    assert_eq!(mark_done.json()["standing"], "done");

    let today_after_done = tcp_request(
        clinic_addr,
        HttpVerb::Get,
        "/elysianfields/api/today",
        Some(&clinic_session),
        Some(clinic_trace),
        "",
    );
    assert_eq!(today_after_done.status, 200);
    assert!(
        !today_after_done.json()["items"]
            .as_array()
            .expect("items")
            .iter()
            .any(|e| e["item"]["item_id"] == urgent_item_id),
        "done item must leave active Today queue"
    );

    // GET /elysianfields/api/workspaces/{id}/memory -> ChartRecord + PhysicianInstruction entries.
    let mem_resp = tcp_request(
        clinic_addr,
        HttpVerb::Get,
        &format!(
            "/elysianfields/api/workspaces/{}/memory",
            workspace_id.as_str()
        ),
        Some(&clinic_session),
        Some(clinic_trace),
        "",
    );
    assert_eq!(mem_resp.status, 200);
    assert_eq!(mem_resp.header(REQUEST_ID_HEADER), Some(clinic_trace));
    let mem_entries = mem_resp.json()["entries"]
        .as_array()
        .expect("memory entries")
        .clone();
    assert!(
        mem_entries.iter().any(|e| e["authority"] == "chart_record"),
        "expected ChartRecord memory entry: {mem_entries:?}"
    );
    assert!(
        mem_entries
            .iter()
            .any(|e| e["authority"] == "physician_instruction"),
        "expected PhysicianInstruction memory entry: {mem_entries:?}"
    );

    // POST /elysianfields/api/workspaces/{id}/connected-care/invitations -> 201 Created EF-XXXXXX.
    let expected_dob = DateOfBirth::new(1986, 7, 14).expect("dob");
    let inv_created_at = "2026-10-18T12:00:00Z".to_owned();
    let invite_resp = tcp_request(
        clinic_addr,
        HttpVerb::Post,
        &format!(
            "/elysianfields/api/workspaces/{}/connected-care/invitations",
            workspace_id.as_str()
        ),
        Some(&clinic_session),
        Some(clinic_trace),
        &serde_json::to_string(&serde_json::json!({
            "patient_id": patient_id,
            "expected_dob": "1986-07-14",
            "practice_name": "Elysian Roundtrip Practice",
            "physician_name": "Dr. Roundtrip",
            "created_at": inv_created_at,
        }))
        .expect("invite payload"),
    );
    assert_eq!(
        invite_resp.status, 201,
        "expected 201 Created, got {}: {}",
        invite_resp.status, invite_resp.body
    );
    assert_eq!(invite_resp.header(REQUEST_ID_HEADER), Some(clinic_trace));
    let invite_json = invite_resp.json();
    let invite_code = invite_json["invite_code"]
        .as_str()
        .expect("invite_code")
        .to_owned();
    assert!(
        invite_code.starts_with("EF-") && invite_code.len() == 9,
        "expected EF-XXXXXX code, got {invite_code}"
    );
    assert_eq!(invite_json["short_code"], invite_code);
    assert_eq!(invite_json["standing"]["state"], "Invited");

    // 4. Bridge invitation into Mobile's isolated ConnectedCareBridge and verify Patient journey.
    let pair = PairKey::new(practice_id, patient_id);
    let bridged = mobile_srv
        .service()
        .lock()
        .expect("lock mobile svc")
        .connected_care
        .create_invitation(CreateInvitationRequest {
            pair,
            issued_by: physician_id,
            practice_name: "Elysian Roundtrip Practice".to_owned(),
            physician_name: "Dr. Roundtrip".to_owned(),
            expected_dob,
            created_at: inv_created_at,
        })
        .expect("bridge invitation to mobile");
    assert_eq!(bridged.short_code.as_str(), invite_code);

    let mobile_trace = "ef-rt-mobile-01";
    let m_link = tcp_request(
        mobile_addr,
        HttpVerb::Post,
        "/elysianfields/mobile-api/auth/request-link",
        None,
        Some(mobile_trace),
        &format!(r#"{{"email":"{mobile_email}"}}"#),
    );
    assert_eq!(m_link.status, 200);
    assert_eq!(m_link.header(REQUEST_ID_HEADER), Some(mobile_trace));
    let m_tok = m_link.json()["token"]
        .as_str()
        .expect("mobile token")
        .to_owned();

    let m_ver = tcp_request(
        mobile_addr,
        HttpVerb::Post,
        "/elysianfields/mobile-api/auth/verify",
        None,
        Some(mobile_trace),
        &format!(r#"{{"email":"{mobile_email}","token":"{m_tok}"}}"#),
    );
    assert_eq!(m_ver.status, 200);
    let mobile_session = m_ver.json()["session_id"]
        .as_str()
        .expect("mobile session")
        .to_owned();

    // Lookup leaks ZERO practice/physician preview before DOB verification.
    let cred = InviteCredential::ShortCode(bridged.short_code);
    let m_lookup = tcp_request(
        mobile_addr,
        HttpVerb::Post,
        "/elysianfields/mobile-api/connected-care/lookup",
        Some(&mobile_session),
        Some(mobile_trace),
        &serde_json::to_string(&serde_json::json!({ "credential": cred })).expect("lookup json"),
    );
    assert_eq!(m_lookup.status, 200);
    assert_eq!(m_lookup.json()["remaining_attempts"], 5);
    assert!(!m_lookup.body.contains("Elysian Roundtrip Practice"));
    assert!(!m_lookup.body.contains("Dr. Roundtrip"));

    // Verify DOB -> reveals preview without granting consent yet (`OnlyShareGrantsConsent`).
    let m_dob = tcp_request(
        mobile_addr,
        HttpVerb::Post,
        "/elysianfields/mobile-api/connected-care/verify-dob",
        Some(&mobile_session),
        Some(mobile_trace),
        &serde_json::to_string(&serde_json::json!({
            "credential": cred,
            "year": 1986,
            "month": 7,
            "day": 14,
        }))
        .expect("dob json"),
    );
    assert_eq!(m_dob.status, 200);
    assert_eq!(
        m_dob.json()["Verified"]["preview"]["practice_name"],
        "Elysian Roundtrip Practice"
    );

    // Explicit share grants consent and activates connection.
    let m_share = tcp_request(
        mobile_addr,
        HttpVerb::Post,
        "/elysianfields/mobile-api/connected-care/share",
        Some(&mobile_session),
        Some(mobile_trace),
        &serde_json::to_string(&serde_json::json!({ "credential": cred })).expect("share json"),
    );
    assert_eq!(m_share.status, 200);
    assert_eq!(m_share.json()["state"], "Connected");
    let conn_id = m_share.json()["active_connection_id"]
        .as_str()
        .expect("connection_id")
        .to_owned();

    // Send an idempotent curated "For My Doctor" note twice with the same idempotency key.
    let note_item = EnvelopeItem {
        item_id: "for-my-doctor-note-1".to_owned(),
        source_version: 1,
        confidentiality: ConfidentialityTag::Standard,
        payload_sha256: "c".repeat(64),
        verbatim_payload: curated_note_text.to_owned(),
        part2_notice: None,
    };
    let transfer_payload = serde_json::to_string(&serde_json::json!({
        "idempotency_key": "rt-fmd-note-idem-1",
        "connection_id": conn_id,
        "items": [note_item],
    }))
    .expect("transfer json");

    let xfer_first = tcp_request(
        mobile_addr,
        HttpVerb::Post,
        "/elysianfields/mobile-api/connected-care/transfer",
        Some(&mobile_session),
        Some(mobile_trace),
        &transfer_payload,
    );
    assert_eq!(xfer_first.status, 200);
    let transfer_id_1 = xfer_first.json()["transfer_id"]
        .as_str()
        .expect("transfer_id 1")
        .to_owned();

    let xfer_replay = tcp_request(
        mobile_addr,
        HttpVerb::Post,
        "/elysianfields/mobile-api/connected-care/transfer",
        Some(&mobile_session),
        Some(mobile_trace),
        &transfer_payload,
    );
    assert_eq!(xfer_replay.status, 200);
    assert_eq!(
        xfer_replay.json()["transfer_id"].as_str(),
        Some(transfer_id_1.as_str()),
        "idempotent curated For My Doctor replay must return identical transfer_id"
    );

    // Offline sign-out (`SignInHoldsNothing`): client clears local session immediately, records
    // `mobile_session` in `owed_sessions`, and flushes it without holding an active bearer token.
    let flush_owed = tcp_request(
        mobile_addr,
        HttpVerb::Post,
        "/elysianfields/mobile-api/auth/sign-out",
        None,
        Some(mobile_trace),
        &serde_json::to_string(&serde_json::json!({
            "owed_sessions": [mobile_session],
        }))
        .expect("owed sign-out json"),
    );
    assert_eq!(flush_owed.status, 200);
    assert_eq!(flush_owed.json()["revoked"], 1);

    // Revoked token is strictly rejected with 401 on subsequent Mobile API requests.
    let revoked_session_check = tcp_request(
        mobile_addr,
        HttpVerb::Get,
        "/elysianfields/mobile-api/auth/session",
        Some(&mobile_session),
        Some(mobile_trace),
        "",
    );
    assert_eq!(revoked_session_check.status, 401);
    let revoked_cc_check = tcp_request(
        mobile_addr,
        HttpVerb::Get,
        "/elysianfields/mobile-api/connected-care/status",
        Some(&mobile_session),
        Some(mobile_trace),
        "",
    );
    assert_eq!(revoked_cc_check.status, 401);

    // 5. Strict cross-product token isolation:
    //    - Clinic bearer token -> 401 on Mobile API
    //    - Active Mobile bearer token -> 401 on Clinic API
    let clinic_on_mobile = tcp_request(
        mobile_addr,
        HttpVerb::Get,
        "/elysianfields/mobile-api/auth/session",
        Some(&clinic_session),
        Some(mobile_trace),
        "",
    );
    assert_eq!(
        clinic_on_mobile.status, 401,
        "Clinic bearer token must be rejected 401 on Mobile API"
    );

    let m2_link = tcp_request(
        mobile_addr,
        HttpVerb::Post,
        "/elysianfields/mobile-api/auth/request-link",
        None,
        Some(mobile_trace),
        &format!(r#"{{"email":"{mobile_email_2}"}}"#),
    );
    assert_eq!(m2_link.status, 200);
    let m2_tok = m2_link.json()["token"]
        .as_str()
        .expect("m2 token")
        .to_owned();
    let m2_ver = tcp_request(
        mobile_addr,
        HttpVerb::Post,
        "/elysianfields/mobile-api/auth/verify",
        None,
        Some(mobile_trace),
        &format!(r#"{{"email":"{mobile_email_2}","token":"{m2_tok}"}}"#),
    );
    assert_eq!(m2_ver.status, 200);
    let active_mobile_session = m2_ver.json()["session_id"]
        .as_str()
        .expect("active mobile session")
        .to_owned();

    let mobile_on_clinic_today = tcp_request(
        clinic_addr,
        HttpVerb::Get,
        "/elysianfields/api/today",
        Some(&active_mobile_session),
        Some(clinic_trace),
        "",
    );
    assert_eq!(
        mobile_on_clinic_today.status, 401,
        "Mobile bearer token must be rejected 401 on Clinic Today API"
    );
    let mobile_on_clinic_memory = tcp_request(
        clinic_addr,
        HttpVerb::Get,
        &format!(
            "/elysianfields/api/workspaces/{}/memory",
            workspace_id.as_str()
        ),
        Some(&active_mobile_session),
        Some(clinic_trace),
        "",
    );
    assert_eq!(
        mobile_on_clinic_memory.status, 401,
        "Mobile bearer token must be rejected 401 on Clinic Memory API"
    );

    // 6. Public Site searchable `/dei?state=CA&service=crisis` & XSS-escaped `/dei?q=<script>`.
    let public_trace = "ef-rt-public-01";
    let dei_ca_crisis = tcp_request(
        public_addr,
        HttpVerb::Get,
        "/dei?state=CA&service=crisis",
        None,
        Some(public_trace),
        "",
    );
    assert_eq!(dei_ca_crisis.status, 200);
    assert_eq!(dei_ca_crisis.header(REQUEST_ID_HEADER), Some(public_trace));
    assert!(
        crisis_lines_appear_before_listings(&dei_ca_crisis.body),
        "988/911/SAMHSA banner must precede facility listings on filtered /dei"
    );
    assert!(
        dei_ca_crisis.body.contains("Bay Area Crisis Stabilization")
            || dei_ca_crisis
                .body
                .contains("Harborview Behavioral Crisis Center"),
        "expected California crisis facility in filtered /dei HTML"
    );
    assert!(
        !dei_ca_crisis
            .body
            .contains("Cascade Outpatient Recovery Center"),
        "non-CA facility must be excluded from /dei?state=CA&service=crisis"
    );

    let dei_xss = tcp_request(
        public_addr,
        HttpVerb::Get,
        "/dei?q=%3Cscript%3Ealert(1)%3C/script%3E",
        None,
        Some(public_trace),
        "",
    );
    assert_eq!(dei_xss.status, 200);
    assert_eq!(dei_xss.header(REQUEST_ID_HEADER), Some(public_trace));
    assert!(
        dei_xss.body.contains("&lt;script&gt;"),
        "XSS payload in /dei?q= must be HTML-escaped"
    );
    assert!(
        !dei_xss.body.contains("<script>alert(1)"),
        "raw <script> payload must never appear in /dei HTML"
    );

    // 7. Verify X-Request-Id propagation and ZERO-PHI structured access logs across all 3 servers.
    let clinic_logs = clinic_srv.access_logs();
    let mobile_logs = mobile_srv.access_logs();
    let public_logs = public_srv.access_logs();

    assert!(
        clinic_logs
            .iter()
            .any(|ev| ev.request_id.as_str() == clinic_trace),
        "Clinic access logs must record traced X-Request-Id"
    );
    assert!(
        mobile_logs
            .iter()
            .any(|ev| ev.request_id.as_str() == mobile_trace),
        "Mobile access logs must record traced X-Request-Id"
    );
    assert!(
        public_logs
            .iter()
            .any(|ev| ev.request_id.as_str() == public_trace),
        "Public site access logs must record traced X-Request-Id"
    );

    let combined_logs_json = format!(
        "{}\n{}\n{}",
        serde_json::to_string(&clinic_logs).expect("serialize clinic logs"),
        serde_json::to_string(&mobile_logs).expect("serialize mobile logs"),
        serde_json::to_string(&public_logs).expect("serialize public logs"),
    );
    for forbidden_phi_or_secret in [
        clinic_email,
        mobile_email,
        mobile_email_2,
        clinic_session.as_str(),
        mobile_session.as_str(),
        active_mobile_session.as_str(),
        workspace_id.as_str(),
        invite_code.as_str(),
        curated_note_text,
        "alert(1)",
    ] {
        assert!(
            !combined_logs_json.contains(forbidden_phi_or_secret),
            "zero-PHI access logs must never contain `{forbidden_phi_or_secret}`"
        );
    }

    clinic_srv.shutdown();
    mobile_srv.shutdown();
    public_srv.shutdown();
}
