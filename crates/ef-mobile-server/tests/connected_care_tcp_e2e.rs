#![cfg(not(target_arch = "wasm32"))]

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};

use ef_connected_care::{
    ConfidentialityTag, ConsentAcknowledgement, ConsentRecordId, CreateInvitationRequest,
    DateOfBirth, EnvelopeItem, InviteCredential, MobileAccountId as BridgeMobileAccountId, PairKey,
    Part2WrittenConsent, VERBATIM_PART2_REDISCLOSURE_NOTICE,
};
use ef_core::{PatientId, PhysicianId, PracticeId};
use ef_mobile_server::{MobileService, TcpMobileServer};
use serde_json::Value;

struct Resp {
    status: u16,
    body: String,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|e| panic!("expected JSON, got `{}`: {e}", self.body))
    }
}

fn tcp_req(addr: SocketAddr, method: &str, target: &str, bearer: Option<&str>, body: &str) -> Resp {
    let mut stream = TcpStream::connect(addr).expect("connect mobile server");
    let mut raw = format!("{method} {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n");
    if let Some(tok) = bearer {
        raw.push_str(&format!("Authorization: Bearer {tok}\r\n"));
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
    let status: u16 = head
        .lines()
        .next()
        .expect("status line")
        .split_whitespace()
        .nth(1)
        .expect("status code")
        .parse()
        .expect("numeric status");
    Resp {
        status,
        body: body_part.to_owned(),
    }
}

#[test]
fn connected_care_mobile_tcp_e2e_lockout_share_part2_transfer_and_disconnect() {
    let mut svc = MobileService::new_in_memory(1_760_000_000);

    // Seed two practice invitations into the shared single-owner ConnectedCareBridge.
    let practice_1 = PracticeId::new("practice-cc-1").expect("practice-1");
    let patient_lock = PatientId::new("pat-lock-1").expect("pat-lock");
    let patient_ok = PatientId::new("pat-ok-2").expect("pat-ok");
    let physician = PhysicianId::new("phys-cc-1").expect("phys");

    let inv_lock = svc
        .connected_care
        .create_invitation(CreateInvitationRequest {
            pair: PairKey::new(practice_1.clone(), patient_lock.clone()),
            issued_by: physician.clone(),
            practice_name: "Synthetic Practice One".to_owned(),
            physician_name: "Dr. Bridge".to_owned(),
            expected_dob: DateOfBirth::new(1985, 4, 19).expect("dob"),
            created_at: "2026-01-12T08:00:00Z".to_owned(),
        })
        .expect("issue lock invitation");

    let inv_ok = svc
        .connected_care
        .create_invitation(CreateInvitationRequest {
            pair: PairKey::new(practice_1.clone(), patient_ok.clone()),
            issued_by: physician,
            practice_name: "Synthetic Practice One".to_owned(),
            physician_name: "Dr. Bridge".to_owned(),
            expected_dob: DateOfBirth::new(1990, 9, 3).expect("dob"),
            created_at: "2026-01-12T08:01:00Z".to_owned(),
        })
        .expect("issue ok invitation");

    let server = TcpMobileServer::bind("127.0.0.1:0", svc).expect("bind mobile server");
    let addr = server.local_addr();

    // 1. Authenticate standalone patient account over TCP.
    let link = tcp_req(
        addr,
        "POST",
        "/elysianfields/mobile-api/auth/request-link",
        None,
        r#"{"email":"cc.patient@example.invalid"}"#,
    );
    assert_eq!(link.status, 200);
    let tok = link.json()["token"].as_str().expect("token").to_owned();

    let verify = tcp_req(
        addr,
        "POST",
        "/elysianfields/mobile-api/auth/verify",
        None,
        &format!(r#"{{"email":"cc.patient@example.invalid","token":"{tok}"}}"#),
    );
    assert_eq!(verify.status, 200);
    let sess = verify.json()["session_id"]
        .as_str()
        .expect("session_id")
        .to_owned();
    let acct_id = verify.json()["account_id"]
        .as_str()
        .expect("account_id")
        .to_owned();

    // 2. Invitation 1: lookup leaks ZERO practice/physician preview; 5 wrong DOB guesses lock permanently.
    let lock_cred = InviteCredential::ShortCode(inv_lock.short_code);
    let lookup_lock = tcp_req(
        addr,
        "POST",
        "/elysianfields/mobile-api/connected-care/lookup",
        Some(&sess),
        &serde_json::to_string(&serde_json::json!({ "credential": lock_cred })).expect("json"),
    );
    assert_eq!(lookup_lock.status, 200);
    assert_eq!(lookup_lock.json()["remaining_attempts"], 5);
    assert!(!lookup_lock.body.contains("Synthetic Practice One"));
    assert!(!lookup_lock.body.contains("Dr. Bridge"));

    for attempt in 1..=4 {
        let wrong = tcp_req(
            addr,
            "POST",
            "/elysianfields/mobile-api/connected-care/verify-dob",
            Some(&sess),
            &serde_json::to_string(&serde_json::json!({
                "credential": lock_cred,
                "year": 1980,
                "month": 1,
                "day": attempt,
            }))
            .expect("json"),
        );
        assert_eq!(wrong.status, 200);
        assert_eq!(wrong.json()["Mismatch"]["remaining_attempts"], 5 - attempt);
        assert!(!wrong.body.contains("Synthetic Practice One"));
    }

    let fifth_wrong = tcp_req(
        addr,
        "POST",
        "/elysianfields/mobile-api/connected-care/verify-dob",
        Some(&sess),
        &serde_json::to_string(&serde_json::json!({
            "credential": lock_cred,
            "year": 1980,
            "month": 1,
            "day": 5,
        }))
        .expect("json"),
    );
    assert_eq!(fifth_wrong.status, 200);
    assert_eq!(fifth_wrong.json(), serde_json::json!("LockedOut"));

    // Locked invitation rejects even the true DOB with 409 Conflict!
    let after_lock = tcp_req(
        addr,
        "POST",
        "/elysianfields/mobile-api/connected-care/verify-dob",
        Some(&sess),
        &serde_json::to_string(&serde_json::json!({
            "credential": lock_cred,
            "year": 1985,
            "month": 4,
            "day": 19,
        }))
        .expect("json"),
    );
    assert_eq!(after_lock.status, 409);

    // 3. Invitation 2: valid DOB reveals PracticePreview without granting consent; explicit Share connects.
    let ok_cred = InviteCredential::LinkToken(inv_ok.link_token);
    let dob_ok = tcp_req(
        addr,
        "POST",
        "/elysianfields/mobile-api/connected-care/verify-dob",
        Some(&sess),
        &serde_json::to_string(&serde_json::json!({
            "credential": ok_cred,
            "year": 1990,
            "month": 9,
            "day": 3,
        }))
        .expect("json"),
    );
    assert_eq!(dob_ok.status, 200);
    assert_eq!(
        dob_ok.json()["Verified"]["preview"]["practice_name"],
        "Synthetic Practice One"
    );

    // Standing is still `Invited` before explicit Share (`OnlyShareGrantsConsent`).
    let status_before_share = tcp_req(
        addr,
        "GET",
        "/elysianfields/mobile-api/connected-care/status?practice_id=practice-cc-1&patient_id=pat-ok-2",
        Some(&sess),
        "",
    );
    assert_eq!(status_before_share.status, 200);
    assert_eq!(status_before_share.json()["state"], "Invited");

    let share = tcp_req(
        addr,
        "POST",
        "/elysianfields/mobile-api/connected-care/share",
        Some(&sess),
        &serde_json::to_string(&serde_json::json!({ "credential": ok_cred })).expect("json"),
    );
    assert_eq!(share.status, 200);
    assert_eq!(share.json()["state"], "Connected");
    let conn_id = share.json()["active_connection_id"]
        .as_str()
        .expect("connection_id")
        .to_owned();

    // 4. Consent-gated Part 2 transfer: rejected 403 without written consent, succeeds 200 with consent.
    let part2_item = EnvelopeItem {
        item_id: "item-part2-1".to_owned(),
        source_version: 1,
        confidentiality: ConfidentialityTag::Part2Sud,
        payload_sha256: "a".repeat(64),
        verbatim_payload: "Synthetic SUD progress summary".to_owned(),
        part2_notice: Some(VERBATIM_PART2_REDISCLOSURE_NOTICE.to_owned()),
    };

    let denied_xfer = tcp_req(
        addr,
        "POST",
        "/elysianfields/mobile-api/connected-care/transfer",
        Some(&sess),
        &serde_json::to_string(&serde_json::json!({
            "idempotency_key": "cc-xfer-1",
            "connection_id": conn_id,
            "items": [part2_item],
        }))
        .expect("json"),
    );
    assert_eq!(denied_xfer.status, 403);

    let consent = Part2WrittenConsent {
        consent_id: ConsentRecordId::new("consent-part2-1").expect("consent id"),
        patient_account: BridgeMobileAccountId::new(&acct_id).expect("acct"),
        practice_id: practice_1,
        granted_at: "2026-01-12T08:10:00Z".to_owned(),
        scope_description: "Share SUD summary with Synthetic Practice One".to_owned(),
        notice_acknowledged: ConsentAcknowledgement::Acknowledged,
    };
    let ok_xfer = tcp_req(
        addr,
        "POST",
        "/elysianfields/mobile-api/connected-care/transfer",
        Some(&sess),
        &serde_json::to_string(&serde_json::json!({
            "idempotency_key": "cc-xfer-1",
            "connection_id": conn_id,
            "items": [part2_item],
            "part2_consent": consent,
        }))
        .expect("json"),
    );
    assert_eq!(ok_xfer.status, 200);
    assert_eq!(ok_xfer.json()["part2_consent_id"], "consent-part2-1");

    // 5. Disconnect transitions `Connected -> Ended` and blocks subsequent transfers (`409`).
    let disc = tcp_req(
        addr,
        "POST",
        "/elysianfields/mobile-api/connected-care/disconnect",
        Some(&sess),
        &serde_json::to_string(&serde_json::json!({ "connection_id": conn_id })).expect("json"),
    );
    assert_eq!(disc.status, 200);
    assert_eq!(disc.json()["state"], "Ended");

    let status_after = tcp_req(
        addr,
        "GET",
        "/elysianfields/mobile-api/connected-care/status?practice_id=practice-cc-1&patient_id=pat-ok-2",
        Some(&sess),
        "",
    );
    assert_eq!(status_after.status, 200);
    assert_eq!(status_after.json()["state"], "Ended");

    let post_disc_xfer = tcp_req(
        addr,
        "POST",
        "/elysianfields/mobile-api/connected-care/transfer",
        Some(&sess),
        &serde_json::to_string(&serde_json::json!({
            "idempotency_key": "cc-xfer-2",
            "connection_id": conn_id,
            "items": [EnvelopeItem {
                item_id: "item-std-2".to_owned(),
                source_version: 1,
                confidentiality: ConfidentialityTag::Standard,
                payload_sha256: "b".repeat(64),
                verbatim_payload: "Standard item after disconnect".to_owned(),
                part2_notice: None,
            }],
        }))
        .expect("json"),
    );
    assert_eq!(post_disc_xfer.status, 409);

    server.shutdown();
}
