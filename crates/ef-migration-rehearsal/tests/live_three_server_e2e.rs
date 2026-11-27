#![cfg(not(target_arch = "wasm32"))]
//! Live 3-server cross-product TCP E2E test booting `TcpClinicServer`, `TcpMobileServer`,
//! and `TcpPublicSiteServer` simultaneously on `127.0.0.1:0` and verifying:
//! 1. Clinic sign-in, workspace-scoped `ef-encoder` semantic search (3-tier access gate +
//!    zero cross-patient leakage + withdrawn exclusion), and `ef-clinical-lib` PHI rejection
//!    vs valid de-identified concept queries.
//! 2. Connected Care cross-product invite (Clinic -> Mobile DOB verify -> explicit Share ->
//!    42 CFR Part 2 consent-gated transfer with verbatim § 2.32(a) notice).
//! 3. Public site `/dei` crisis lines (`988`, `911`, `SAMHSA`) strictly before facilities
//!    and `/invite` `noindex, nofollow` + `no-store`.
//! 4. Mobile account deletion -> Connected Care link termination -> pre-deletion backup
//!    fail-closed `503 /readyz` -> independent privacy-receipt replay -> `200 /readyz` +
//!    non-resurrection (`403`).

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
use ef_clinic_domain::workspace::{
    CitationOrigin, CitedSourcePayload, ReadablePart, classify_record_status,
};
use ef_clinic_server::{ClinicService, TcpClinicServer};
use ef_connected_care::{
    ConfidentialityTag, ConnectedCareState, ConsentAcknowledgement, ConsentRecordId,
    CreateInvitationRequest, DateOfBirth, EnvelopeItem, InviteCredential,
    MobileAccountId as BridgeMobileAccountId, PairKey, Part2WrittenConsent,
    VERBATIM_PART2_REDISCLOSURE_NOTICE,
};
use ef_core::{Locale, PatientId, PhysicianId, PracticeId, SourceKey, WorkspaceId};
use ef_mobile_domain::FileReceiptJournal;
use ef_mobile_server::{MobileService, TcpMobileServer};
use ef_public_site::{
    DeploymentMode, PublicSiteRouter, TcpPublicSiteServer, crisis_lines_appear_before_listings,
};
use serde_json::Value;

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
    method: &str,
    target: &str,
    bearer: Option<&str>,
    body: &str,
) -> HttpResp {
    let mut stream = TcpStream::connect(addr).expect("connect loopback tcp server");
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

fn fresh_temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!(
        "ef-live-3server-{label}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn make_source(
    workspace_id: &WorkspaceId,
    source_key: &str,
    number: u16,
    title: &str,
    quote: &str,
    body: &str,
) -> CitedSourcePayload {
    CitedSourcePayload {
        source_key: SourceKey::new(source_key).expect("source key"),
        workspace_id: workspace_id.clone(),
        number,
        source_label: format!("{title} [{number}]"),
        title: title.to_owned(),
        record_type: "progress_note".to_owned(),
        recorded_at: "2026-01-12T08:00:00Z".to_owned(),
        record_status: "final".to_owned(),
        status_class: classify_record_status("final"),
        origin: CitationOrigin::SourceRecord,
        original_language: Locale::En,
        quote: quote.to_owned(),
        body: body.to_owned(),
        readable_parts: vec![ReadablePart {
            label: "Assessment".to_owned(),
            text: quote.to_owned(),
            unreadable_notice: None,
        }],
        fields: Vec::new(),
        raw_content: None,
        external_url: None,
    }
}

#[test]
fn live_three_server_cross_product_tcp_e2e() {
    let clinic_dir = fresh_temp_dir("clinic-wal");
    let mobile_dir = fresh_temp_dir("mobile-wal");
    let mobile_backup_dir = fresh_temp_dir("mobile-backup");
    let journal_dir = fresh_temp_dir("mobile-journal");

    let now_secs = 1_760_000_000;
    let practice_1 = PracticeId::new("practice-3srv-1").expect("practice-1");
    let practice_2 = PracticeId::new("practice-3srv-2").expect("practice-2");
    let phys_1 = PhysicianId::new("phys-3srv-1").expect("phys-1");
    let phys_other = PhysicianId::new("phys-3srv-other").expect("phys-other");
    let ws_nephro = WorkspaceId::new("ws-3srv-nephro").expect("ws-nephro");
    let ws_cardio = WorkspaceId::new("ws-3srv-cardio").expect("ws-cardio");
    let ws_demo = WorkspaceId::new("ws-3srv-demo").expect("ws-demo");
    let ws_other_practice = WorkspaceId::new("ws-3srv-other-practice").expect("ws-other");

    // 1. Boot Clinic server with durable WAL + seeded workspaces & sources.
    let mut clinic_svc = ClinicService::open_durable(
        &clinic_dir,
        SignInMode::LocalProof,
        RetryBackoffMode::DeterministicTest,
        SourceKey::new("src-ckd-1").expect("default src"),
        now_secs,
    )
    .expect("open durable clinic service");
    clinic_svc
        .seed_account(PhysicianAccount {
            physician_id: phys_1.clone(),
            practice_id: practice_1.clone(),
            email: EmailAddress::parse("dr.three@clinic.example.invalid").expect("email"),
            display_name: "Dr. Three Server".to_owned(),
            role: PhysicianRole::Standard,
            standing: AccountStanding::Active,
            sessions_ended_before: None,
        })
        .expect("seed physician");
    clinic_svc
        .seed_workspace(
            ws_nephro.clone(),
            practice_1.clone(),
            "Synthetic Nephrology Patient",
            vec![phys_1.clone()],
        )
        .expect("seed ws_nephro");
    clinic_svc
        .seed_workspace(
            ws_cardio.clone(),
            practice_1.clone(),
            "Synthetic Cardiology Patient",
            vec![phys_1.clone()],
        )
        .expect("seed ws_cardio");
    clinic_svc
        .seed_workspace(
            ws_demo.clone(),
            practice_1.clone(),
            "Synthetic Demographic Only Patient",
            vec![phys_other.clone()],
        )
        .expect("seed ws_demo");
    clinic_svc
        .seed_workspace(
            ws_other_practice.clone(),
            practice_2,
            "Synthetic Cross-Practice Patient",
            vec![phys_other],
        )
        .expect("seed ws_other");

    clinic_svc
        .seed_source(make_source(
            &ws_nephro,
            "src-ckd-1",
            1,
            "Nephrology Lab Note",
            "Serum creatinine 1.6 mg/dL, eGFR 44 mL/min, CKD stage G3b.",
            "Continue ACE inhibitor and repeat renal panel in 4 weeks.",
        ))
        .expect("seed nephro source");
    clinic_svc
        .seed_source(make_source(
            &ws_nephro,
            "src-withdrawn-1",
            2,
            "Withdrawn Nephrology Addendum",
            "Creatinine eGFR erroneous duplicate report withdrawn.",
            "Superseded duplicate lab report.",
        ))
        .expect("seed withdrawn source");
    clinic_svc
        .withdraw_source(&ws_nephro, &SourceKey::new("src-withdrawn-1").expect("key"))
        .expect("withdraw source");
    clinic_svc
        .seed_source(make_source(
            &ws_cardio,
            "src-afib-1",
            1,
            "Cardiology Consult",
            "Paroxysmal atrial fibrillation with normal troponin and preserved EF.",
            "Anticoagulation maintained without bleeding complications.",
        ))
        .expect("seed cardio source");

    let clinic_srv = TcpClinicServer::bind("127.0.0.1:0", clinic_svc).expect("bind clinic server");
    let clinic_addr = clinic_srv.local_addr();

    // 2. Boot Mobile server (durable WAL + independent FileReceiptJournal) AND Public site server simultaneously.
    let journal1 = FileReceiptJournal::open(&journal_dir).expect("open file receipt journal");
    let mobile_svc = MobileService::open_durable(&mobile_dir, Box::new(journal1), now_secs, false)
        .expect("open durable mobile service");
    let mobile_srv1 =
        TcpMobileServer::bind("127.0.0.1:0", mobile_svc).expect("bind mobile server 1");
    let mobile_addr1 = mobile_srv1.local_addr();

    let public_srv = TcpPublicSiteServer::bind(
        "127.0.0.1:0",
        PublicSiteRouter::new(DeploymentMode::Production),
    )
    .expect("bind public site server");
    let public_addr = public_srv.local_addr();

    // 3. Clinic sign-in + workspace-scoped `ef-encoder` semantic search + `ef-clinical-lib` concept query.
    let c_req = tcp_request(
        clinic_addr,
        "POST",
        "/elysianfields/api/auth/request-link",
        None,
        r#"{"email":"dr.three@clinic.example.invalid"}"#,
    );
    assert_eq!(c_req.status, 200);
    let c_tok = c_req.json()["token"]
        .as_str()
        .expect("clinic token")
        .to_owned();
    let c_ver = tcp_request(
        clinic_addr,
        "POST",
        "/elysianfields/api/auth/verify",
        None,
        &format!(r#"{{"email":"dr.three@clinic.example.invalid","token":"{c_tok}"}}"#),
    );
    assert_eq!(c_ver.status, 200);
    let clinic_session = c_ver.json()["session"]
        .as_str()
        .expect("clinic session")
        .to_owned();

    // Semantic search scoped strictly to ws_nephro (256-dim offline encoder; zero cross-patient or withdrawn leakage).
    let search_ok = tcp_request(
        clinic_addr,
        "POST",
        "/elysianfields/api/patients/ws-3srv-nephro/search",
        Some(&clinic_session),
        r#"{"query":"creatinine eGFR nephrology CKD","top_k":5}"#,
    );
    assert_eq!(search_ok.status, 200);
    let search_json = search_ok.json();
    assert_eq!(search_json["model_version"], "ef-offline-hash-v1");
    assert_eq!(search_json["embedding_dim"], 256);
    let hits = search_json["hits"].as_array().expect("hits array");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["doc_id"], "src-ckd-1");
    assert_eq!(hits[0]["scope_id"], "ws-3srv-nephro");

    // 3-tier access gate on semantic search: ClinicDemographic -> 403, cross-practice Denied -> 404.
    let search_demo = tcp_request(
        clinic_addr,
        "POST",
        "/elysianfields/api/patients/ws-3srv-demo/search",
        Some(&clinic_session),
        r#"{"query":"creatinine"}"#,
    );
    assert_eq!(search_demo.status, 403);

    let search_cross_practice = tcp_request(
        clinic_addr,
        "POST",
        "/elysianfields/api/patients/ws-3srv-other-practice/search",
        Some(&clinic_session),
        r#"{"query":"creatinine"}"#,
    );
    assert_eq!(search_cross_practice.status, 404);

    // Clinical Library de-identified concept query: PHI fails closed with 400; de-identified concept succeeds with 200.
    let lib_phi_mrn = tcp_request(
        clinic_addr,
        "POST",
        "/elysianfields/api/clinical-library/query",
        Some(&clinic_session),
        r#"{"query":"MRN:998811 patient@example.invalid 123-45-6789 creatinine"}"#,
    );
    assert_eq!(lib_phi_mrn.status, 400);
    assert_eq!(lib_phi_mrn.json()["error"], "phi_or_invalid_concept_query");

    let lib_ok = tcp_request(
        clinic_addr,
        "POST",
        "/elysianfields/api/clinical-library/query",
        Some(&clinic_session),
        r#"{"query":"chronic kidney disease eGFR albuminuria staging","top_k":2}"#,
    );
    assert_eq!(lib_ok.status, 200);
    let lib_json = lib_ok.json();
    assert_eq!(lib_json["deidentified_verified"], true);
    let matches = lib_json["matches"].as_array().expect("matches array");
    assert!(!matches.is_empty());
    assert_eq!(matches[0]["article_id"], "ref-ckd-egfr-staging");

    // 4. Connected Care cross-product invite: Clinic HTTP -> Mobile HTTP DOB verify -> Share -> Part 2 transfer.
    let patient_id = PatientId::new("pat-3srv-1").expect("patient id");
    let dob = DateOfBirth::new(1987, 3, 21).expect("dob");
    let inv_created_at = "2026-01-12T10:00:00Z".to_owned();
    let clinic_inv = tcp_request(
        clinic_addr,
        "POST",
        "/elysianfields/api/connected-care/invitations",
        Some(&clinic_session),
        &serde_json::to_string(&serde_json::json!({
            "patient_id": patient_id,
            "workspace_id": ws_nephro,
            "practice_name": "Synthetic Three-Server Clinic",
            "physician_name": "Dr. Three Server",
            "expected_dob": { "year": 1987, "month": 3, "day": 21 },
            "created_at": inv_created_at,
        }))
        .expect("invite json"),
    );
    assert_eq!(clinic_inv.status, 200);
    let clinic_inv_json = clinic_inv.json();
    assert_eq!(clinic_inv_json["standing"]["state"], "Invited");
    let clinic_short_code = clinic_inv_json["short_code"]
        .as_str()
        .expect("short_code")
        .to_owned();

    // Deliver the cross-product Connected Care invitation event into Mobile's bridge store.
    let pair = PairKey::new(practice_1.clone(), patient_id);
    let bridged = mobile_srv1
        .service()
        .lock()
        .expect("lock mobile svc")
        .connected_care
        .create_invitation(CreateInvitationRequest {
            pair: pair.clone(),
            issued_by: phys_1,
            practice_name: "Synthetic Three-Server Clinic".to_owned(),
            physician_name: "Dr. Three Server".to_owned(),
            expected_dob: dob,
            created_at: inv_created_at,
        })
        .expect("bridge invite onto mobile");
    assert_eq!(bridged.short_code.as_str(), clinic_short_code);

    // Patient signs in on Mobile TCP server.
    let m_link = tcp_request(
        mobile_addr1,
        "POST",
        "/elysianfields/mobile-api/auth/request-link",
        None,
        r#"{"email":"patient.three@mobile.example.invalid"}"#,
    );
    assert_eq!(m_link.status, 200);
    let m_tok = m_link.json()["token"].as_str().expect("m token").to_owned();
    let m_ver = tcp_request(
        mobile_addr1,
        "POST",
        "/elysianfields/mobile-api/auth/verify",
        None,
        &format!(r#"{{"email":"patient.three@mobile.example.invalid","token":"{m_tok}"}}"#),
    );
    assert_eq!(m_ver.status, 200);
    let mobile_session = m_ver.json()["session_id"]
        .as_str()
        .expect("mobile session_id")
        .to_owned();
    let mobile_acct_id = m_ver.json()["account_id"]
        .as_str()
        .expect("mobile account_id")
        .to_owned();

    let cred = InviteCredential::ShortCode(bridged.short_code);
    let m_lookup = tcp_request(
        mobile_addr1,
        "POST",
        "/elysianfields/mobile-api/connected-care/lookup",
        Some(&mobile_session),
        &serde_json::to_string(&serde_json::json!({ "credential": cred })).expect("json"),
    );
    assert_eq!(m_lookup.status, 200);
    assert_eq!(m_lookup.json()["remaining_attempts"], 5);
    assert!(!m_lookup.body.contains("Synthetic Three-Server Clinic"));

    let m_dob = tcp_request(
        mobile_addr1,
        "POST",
        "/elysianfields/mobile-api/connected-care/verify-dob",
        Some(&mobile_session),
        &serde_json::to_string(&serde_json::json!({
            "credential": cred,
            "year": 1987,
            "month": 3,
            "day": 21,
        }))
        .expect("json"),
    );
    assert_eq!(m_dob.status, 200);
    assert_eq!(
        m_dob.json()["Verified"]["preview"]["practice_name"],
        "Synthetic Three-Server Clinic"
    );

    let m_share = tcp_request(
        mobile_addr1,
        "POST",
        "/elysianfields/mobile-api/connected-care/share",
        Some(&mobile_session),
        &serde_json::to_string(&serde_json::json!({ "credential": cred })).expect("json"),
    );
    assert_eq!(m_share.status, 200);
    assert_eq!(m_share.json()["state"], "Connected");
    let conn_id = m_share.json()["active_connection_id"]
        .as_str()
        .expect("active_connection_id")
        .to_owned();

    // 42 CFR Part 2 consent-gated transfer over live TCP: 403 without consent, 200 with consent.
    let part2_item = EnvelopeItem {
        item_id: "3srv-part2-1".to_owned(),
        source_version: 1,
        confidentiality: ConfidentialityTag::Part2Sud,
        payload_sha256: "d".repeat(64),
        verbatim_payload: "Synthetic SUD progress note for care team".to_owned(),
        part2_notice: Some(VERBATIM_PART2_REDISCLOSURE_NOTICE.to_owned()),
    };
    let denied_xfer = tcp_request(
        mobile_addr1,
        "POST",
        "/elysianfields/mobile-api/connected-care/transfer",
        Some(&mobile_session),
        &serde_json::to_string(&serde_json::json!({
            "idempotency_key": "3srv-xfer-1",
            "connection_id": conn_id,
            "items": [part2_item],
        }))
        .expect("json"),
    );
    assert_eq!(denied_xfer.status, 403);

    let part2_consent = Part2WrittenConsent {
        consent_id: ConsentRecordId::new("consent-3srv-part2-1").expect("consent id"),
        patient_account: BridgeMobileAccountId::new(&mobile_acct_id).expect("bridge acct"),
        practice_id: practice_1,
        granted_at: "2026-01-12T10:05:00Z".to_owned(),
        scope_description: "Share SUD summary with Synthetic Three-Server Clinic".to_owned(),
        notice_acknowledged: ConsentAcknowledgement::Acknowledged,
    };
    let ok_xfer = tcp_request(
        mobile_addr1,
        "POST",
        "/elysianfields/mobile-api/connected-care/transfer",
        Some(&mobile_session),
        &serde_json::to_string(&serde_json::json!({
            "idempotency_key": "3srv-xfer-1",
            "connection_id": conn_id,
            "items": [part2_item],
            "part2_consent": part2_consent,
        }))
        .expect("json"),
    );
    assert_eq!(ok_xfer.status, 200);
    assert_eq!(ok_xfer.json()["part2_consent_id"], "consent-3srv-part2-1");

    // 5. Public site TCP checks: `/dei` crisis lines strictly before facilities + `/invite` `noindex` + `/health`/`/readyz`/`/grafana/`.
    let pub_health = tcp_request(public_addr, "GET", "/health", None, "");
    assert_eq!(pub_health.status, 200);
    let pub_ready = tcp_request(public_addr, "GET", "/readyz", None, "");
    assert_eq!(pub_ready.status, 200);

    let pub_dei = tcp_request(public_addr, "GET", "/dei", None, "");
    assert_eq!(pub_dei.status, 200);
    assert!(pub_dei.body.contains("988"));
    assert!(pub_dei.body.contains("911"));
    assert!(pub_dei.body.contains("SAMHSA"));
    assert!(
        crisis_lines_appear_before_listings(&pub_dei.body),
        "/dei crisis lines must precede facility listings"
    );

    let pub_dei_search = tcp_request(public_addr, "GET", "/dei/search?q=Outpatient", None, "");
    assert_eq!(pub_dei_search.status, 200);
    assert!(
        crisis_lines_appear_before_listings(&pub_dei_search.body),
        "/dei/search crisis lines must precede facility listings"
    );

    let pub_invite = tcp_request(public_addr, "GET", "/invite", None, "");
    assert_eq!(pub_invite.status, 200);
    assert!(
        pub_invite.body.contains("noindex, nofollow")
            || pub_invite
                .header("x-robots-tag")
                .is_some_and(|v| v.contains("noindex")),
        "/invite must declare noindex, nofollow"
    );
    assert!(
        pub_invite
            .header("cache-control")
            .is_some_and(|v| v.contains("no-store")),
        "/invite must set no-store"
    );

    let pub_grafana = tcp_request(public_addr, "GET", "/grafana/", None, "");
    assert_eq!(pub_grafana.status, 404);
    assert!(
        pub_grafana
            .header("cache-control")
            .is_some_and(|v| v.contains("no-store"))
    );

    // 6. Snapshot pre-deletion mobile WAL -> delete mobile account -> Connected Care link terminated ->
    //    restore backup fails closed (`503 /readyz`) -> replay gate -> `200 /readyz` + non-resurrection (`403`).
    fs::copy(
        mobile_dir.join("mobile-wal.jsonl"),
        mobile_backup_dir.join("mobile-wal.jsonl"),
    )
    .expect("snapshot pre-deletion mobile WAL backup");

    let m_del = tcp_request(
        mobile_addr1,
        "POST",
        "/elysianfields/mobile-api/account/delete",
        Some(&mobile_session),
        r#"{"event_id":"evt-3srv-del-1","occurred_at":"2026-01-12T10:15:00Z"}"#,
    );
    assert_eq!(m_del.status, 200);
    assert_eq!(m_del.json()["deleted"], true);

    // Verify account deletion terminated the active Connected Care connection immediately.
    {
        let guard = mobile_srv1.service();
        let standing = guard
            .lock()
            .expect("lock")
            .connected_care
            .mobile_standing(&pair);
        assert_eq!(standing.state, ConnectedCareState::Ended);
    }
    mobile_srv1.shutdown();

    let journal2 = FileReceiptJournal::open(&journal_dir).expect("reopen journal");
    let mobile_svc2 =
        MobileService::open_durable(&mobile_backup_dir, Box::new(journal2), now_secs + 600, true)
            .expect("open pre-deletion backup");
    let mobile_srv2 =
        TcpMobileServer::bind("127.0.0.1:0", mobile_svc2).expect("bind restored mobile server");
    let mobile_addr2 = mobile_srv2.local_addr();

    let pre_replay = tcp_request(
        mobile_addr2,
        "GET",
        "/elysianfields/mobile-api/readyz",
        None,
        "",
    );
    assert_eq!(pre_replay.status, 503);

    {
        let svc_arc = mobile_srv2.service();
        let report = svc_arc
            .lock()
            .expect("lock restored mobile svc")
            .execute_restore_replay_gate(1_000)
            .expect("execute restore replay gate");
        assert_eq!(report.applied, 1);
    }

    let post_replay = tcp_request(
        mobile_addr2,
        "GET",
        "/elysianfields/mobile-api/readyz",
        None,
        "",
    );
    assert_eq!(post_replay.status, 200);

    let old_sess = tcp_request(
        mobile_addr2,
        "GET",
        "/elysianfields/mobile-api/auth/session",
        Some(&mobile_session),
        "",
    );
    assert_eq!(old_sess.status, 401);

    let resurrect_attempt = tcp_request(
        mobile_addr2,
        "POST",
        "/elysianfields/mobile-api/auth/request-link",
        None,
        r#"{"email":"patient.three@mobile.example.invalid"}"#,
    );
    assert_eq!(resurrect_attempt.status, 403);

    mobile_srv2.shutdown();
    clinic_srv.shutdown();
    public_srv.shutdown();

    let _ = fs::remove_dir_all(&clinic_dir);
    let _ = fs::remove_dir_all(&mobile_dir);
    let _ = fs::remove_dir_all(&mobile_backup_dir);
    let _ = fs::remove_dir_all(&journal_dir);
}
