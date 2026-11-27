#![cfg(not(target_arch = "wasm32"))]

//! Live PostgreSQL 18 protocol & durability proof for `ef-pgwire`, `ef-clinic-server`,
//! and `ef-mobile-server`.
//!
//! Runs automatically whenever `EF_PG_TEST_URL` is set (invoked unconditionally via
//! `scripts/verify_pgwire_live.sh`); skips cleanly with an explicit message when unset
//! during offline unit test runs.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};

use ef_clinic_domain::auth::{
    AccountStanding, EmailAddress, PhysicianAccount, PhysicianRole, SignInMode,
};
use ef_clinic_domain::chat::RetryBackoffMode;
use ef_clinic_domain::workspace::{
    CitationOrigin, CitedSourcePayload, ReadablePart, classify_record_status,
};
use ef_clinic_server::{ClinicService, TcpClinicServer};
use ef_core::{Locale, PhysicianId, PracticeId, SourceKey, WorkspaceId};
use ef_mobile_domain::{
    AccountIncarnation, ClinicalRecordBucket, ConfidentialityTag, ImportSourceKind,
    InMemoryReceiptJournal, IncomingBatchItem, MobileAccountId, PatientRecord,
};
use ef_mobile_server::{MobileService, TcpMobileServer};
use ef_pgwire::{PgClient, PgConnectionConfig, PgError, PgParam};
use serde_json::Value;

struct RawHttpResponse {
    status: u16,
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
    let mut stream = TcpStream::connect(addr).expect("connect to local TCP server");
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
    let status: u16 = head
        .lines()
        .next()
        .expect("status line")
        .split_whitespace()
        .nth(1)
        .expect("status code")
        .parse()
        .expect("numeric status");
    RawHttpResponse {
        status,
        body: body_part.to_owned(),
    }
}

fn synthetic_note(workspace_id: WorkspaceId, source_key: SourceKey) -> CitedSourcePayload {
    CitedSourcePayload {
        source_key,
        workspace_id,
        number: 1,
        source_label: "PG18 Progress Note [1]".to_owned(),
        title: "Synthetic Live PG18 Note".to_owned(),
        record_type: "progress_note".to_owned(),
        recorded_at: "2026-02-14T10:00:00Z".to_owned(),
        record_status: "final".to_owned(),
        status_class: classify_record_status("final"),
        origin: CitationOrigin::SourceRecord,
        original_language: Locale::En,
        quote: "Creatinine 1.02 mg/dL persisted in PostgreSQL 18.".to_owned(),
        body: "Synthetic PG18 durability verification note.".to_owned(),
        readable_parts: vec![ReadablePart {
            label: "Assessment".to_owned(),
            text: "Creatinine 1.02 mg/dL persisted in PostgreSQL 18.".to_owned(),
            unreadable_notice: None,
        }],
        fields: Vec::new(),
        raw_content: None,
        external_url: None,
    }
}

#[test]
fn live_postgres18_protocol_scram_injection_safety_and_server_restart_durability() {
    let Ok(base_url) = std::env::var("EF_PG_TEST_URL") else {
        eprintln!(
            "SKIP live_postgres18_durability: set EF_PG_TEST_URL or run ./scripts/verify_pgwire_live.sh"
        );
        return;
    };

    // -------------------------------------------------------------------------
    // 1. SCRAM-SHA-256 auth (plus optional trust auth), Extended Query CRUD,
    //    SQL-injection resistance, and transaction commit/rollback
    // -------------------------------------------------------------------------
    let cfg = PgConnectionConfig::parse_url(&base_url).expect("parse EF_PG_TEST_URL");
    let mut client = PgClient::connect(&cfg).expect("connect via SCRAM-SHA-256");
    let server_ver = client
        .server_parameter("server_version")
        .unwrap_or("unknown")
        .to_owned();
    eprintln!("Connected via ef-pgwire to PostgreSQL {server_ver}");

    if let Ok(trust_url) = std::env::var("EF_PG_TRUST_URL") {
        let trust_cfg = PgConnectionConfig::parse_url(&trust_url).expect("parse EF_PG_TRUST_URL");
        let mut trust_client = PgClient::connect(&trust_cfg).expect("connect via trust auth");
        let ping = trust_client
            .query("SELECT $1::bigint + 1", &[PgParam::Int64(41)])
            .expect("trust auth query");
        assert_eq!(ping[0].get_i64(0).expect("i64"), Some(42));
    }

    client
        .execute_batch_ddl(
            "DROP TABLE IF EXISTS ef_pgwire_probe; \
             CREATE TABLE ef_pgwire_probe (\
                 id BIGINT PRIMARY KEY, \
                 label TEXT NOT NULL, \
                 active BOOLEAN NOT NULL, \
                 raw_blob BYTEA, \
                 opt_note TEXT\
             );",
        )
        .expect("create probe table");

    let injection_payload = "'; DROP TABLE ef_pgwire_probe; --";
    let blob_payload: &[u8] = &[0x00, 0xde, 0xad, 0xbe, 0xef, 0x7f, 0xff];

    let inserted = client
        .execute(
            "INSERT INTO ef_pgwire_probe (id, label, active, raw_blob, opt_note) \
             VALUES ($1, $2, $3, $4, $5)",
            &[
                PgParam::Int64(1),
                PgParam::Text(injection_payload),
                PgParam::Bool(true),
                PgParam::Bytes(blob_payload),
                PgParam::Null,
            ],
        )
        .expect("parameterized INSERT");
    assert_eq!(inserted, 1);

    let rows = client
        .query(
            "SELECT id, label, active, raw_blob, opt_note FROM ef_pgwire_probe WHERE label = $1",
            &[PgParam::Text(injection_payload)],
        )
        .expect("parameterized SELECT");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get_i64(0).expect("id"), Some(1));
    assert_eq!(
        rows[0].get_text(1).expect("label").as_deref(),
        Some(injection_payload)
    );
    assert_eq!(rows[0].get_bool(2).expect("active"), Some(true));
    assert_eq!(
        rows[0].get_bytes(3).expect("blob").as_deref(),
        Some(blob_payload)
    );
    assert_eq!(rows[0].get_text(4).expect("opt_note"), None);

    // Transaction rollback leaves state untouched; commit persists.
    let rollback_res: Result<(), PgError> = client.in_transaction(|tx| {
        tx.execute(
            "INSERT INTO ef_pgwire_probe (id, label, active, raw_blob, opt_note) \
             VALUES ($1, $2, $3, $4, $5)",
            &[
                PgParam::Int64(2),
                PgParam::Text("rolled-back"),
                PgParam::Bool(false),
                PgParam::Null,
                PgParam::Text("temp"),
            ],
        )?;
        Err(PgError::Protocol {
            detail: "intentional rollback".to_owned(),
        })
    });
    assert!(rollback_res.is_err());

    let count_after_rb = client
        .query("SELECT COUNT(*) FROM ef_pgwire_probe", &[])
        .expect("count after rollback");
    assert_eq!(count_after_rb[0].get_i64(0).expect("count"), Some(1));

    client
        .in_transaction(|tx| {
            tx.execute(
                "INSERT INTO ef_pgwire_probe (id, label, active, raw_blob, opt_note) \
                 VALUES ($1, $2, $3, $4, $5)",
                &[
                    PgParam::Int64(3),
                    PgParam::Text("committed"),
                    PgParam::Bool(true),
                    PgParam::Null,
                    PgParam::Text("durable"),
                ],
            )?;
            Ok(())
        })
        .expect("commit transaction");
    let count_after_commit = client
        .query("SELECT COUNT(*) FROM ef_pgwire_probe", &[])
        .expect("count after commit");
    assert_eq!(count_after_commit[0].get_i64(0).expect("count"), Some(2));

    // -------------------------------------------------------------------------
    // 2. Live `ef-clinic-server` write -> kill -> restart -> durable PG18 readback
    // -------------------------------------------------------------------------
    let clinic_url = std::env::var("EF_PG_CLINIC_URL").unwrap_or_else(|_| base_url.clone());
    let clinic_cfg = PgConnectionConfig::parse_url(&clinic_url).expect("parse clinic PG URL");
    let now_secs = 1_760_000_000u64;
    let default_src = SourceKey::new("src-note-1").expect("src-note-1");
    let practice_a = PracticeId::new("practice-pg").expect("practice");
    let doc_a = PhysicianId::new("phys-pg-1").expect("phys");
    let ws_a1 = WorkspaceId::new("ws-pg-1").expect("ws");

    let mut clinic_svc_1 = ClinicService::open_postgres(
        &clinic_cfg,
        SignInMode::LocalProof,
        RetryBackoffMode::DeterministicTest,
        default_src.clone(),
        now_secs,
    )
    .expect("open ClinicService on live PG18");
    clinic_svc_1
        .seed_account(PhysicianAccount {
            physician_id: doc_a.clone(),
            practice_id: practice_a.clone(),
            email: EmailAddress::parse("pg.doc@example.invalid").expect("email"),
            display_name: "Dr. PG18".to_owned(),
            role: PhysicianRole::Standard,
            standing: AccountStanding::Active,
            sessions_ended_before: None,
        })
        .expect("seed clinic account");
    clinic_svc_1
        .seed_workspace(
            ws_a1.clone(),
            practice_a,
            "Synthetic PG18 Patient",
            vec![doc_a],
        )
        .expect("seed clinic workspace");
    clinic_svc_1
        .seed_source(synthetic_note(ws_a1, default_src.clone()))
        .expect("seed clinic source");

    let clinic_srv_1 =
        TcpClinicServer::bind("127.0.0.1:0", clinic_svc_1).expect("bind clinic server 1");
    let c_addr_1 = clinic_srv_1.local_addr();

    let link_resp = send_tcp_request(
        c_addr_1,
        "POST",
        "/elysianfields/api/auth/sign-in",
        &[],
        r#"{"email":"pg.doc@example.invalid"}"#,
    );
    assert_eq!(link_resp.status, 200);
    let proof_tok = link_resp.json()["token"]
        .as_str()
        .expect("proof token")
        .to_owned();
    let verify_resp = send_tcp_request(
        c_addr_1,
        "POST",
        "/elysianfields/api/auth/verify",
        &[],
        &format!(r#"{{"email":"pg.doc@example.invalid","token":"{proof_tok}"}}"#),
    );
    assert_eq!(verify_resp.status, 200);
    let clinic_sess = verify_resp.json()["session"]
        .as_str()
        .expect("clinic session")
        .to_owned();
    let c_auth = format!("Bearer {clinic_sess}");
    let c_headers = [("Authorization", c_auth.as_str())];

    let cmd_resp = send_tcp_request(
        c_addr_1,
        "POST",
        "/elysianfields/api/workspaces/ws-pg-1/chats/chat-pg/commands",
        &c_headers,
        r#"{
            "type": "message.submit",
            "idempotency_key": "idem-pg18-1",
            "payload": {
                "patient_workspace_id": "ws-pg-1",
                "chat_id": "chat-pg",
                "text": "Verify PostgreSQL 18 durability."
            }
        }"#,
    );
    assert_eq!(cmd_resp.status, 200);

    let draft_resp = send_tcp_request(
        c_addr_1,
        "PUT",
        "/elysianfields/api/workspaces/ws-pg-1/chats/chat-pg/draft",
        &c_headers,
        r#"{"text":"Durable PG18 draft","expected_revision":0}"#,
    );
    assert_eq!(draft_resp.status, 200);

    let mem_resp = send_tcp_request(
        c_addr_1,
        "POST",
        "/elysianfields/api/patients/ws-pg-1/memory",
        &c_headers,
        r#"{
            "op": "record",
            "topic": "pg18_durability",
            "statement": "Persisted in PostgreSQL 18 journal.",
            "observed_at": "2026-02-14",
            "authority": "physician_instruction"
        }"#,
    );
    assert_eq!(mem_resp.status, 200);

    // Kill Clinic server 1 and restart Clinic server 2 against the exact same PG18 database!
    clinic_srv_1.shutdown();

    let clinic_svc_2 = ClinicService::open_postgres(
        &clinic_cfg,
        SignInMode::LocalProof,
        RetryBackoffMode::DeterministicTest,
        default_src,
        now_secs + 30,
    )
    .expect("reopen ClinicService on live PG18");
    let clinic_srv_2 =
        TcpClinicServer::bind("127.0.0.1:0", clinic_svc_2).expect("bind clinic server 2");
    let c_addr_2 = clinic_srv_2.local_addr();

    let snap_after = send_tcp_request(
        c_addr_2,
        "GET",
        "/elysianfields/api/workspaces/ws-pg-1/chats/chat-pg",
        &c_headers,
        "",
    );
    assert_eq!(snap_after.status, 200);
    assert_eq!(
        snap_after.json()["exchanges"].as_array().expect("ex").len(),
        1
    );
    assert_eq!(snap_after.json()["draft"]["text"], "Durable PG18 draft");

    let mem_after = send_tcp_request(
        c_addr_2,
        "GET",
        "/elysianfields/api/patients/ws-pg-1/memory",
        &c_headers,
        "",
    );
    assert_eq!(mem_after.status, 200);
    assert_eq!(
        mem_after.json()["entries"]
            .as_array()
            .expect("entries")
            .len(),
        1
    );
    clinic_srv_2.shutdown();

    // -------------------------------------------------------------------------
    // 3. Live `ef-mobile-server` write -> kill -> restart -> durable PG18 readback
    // -------------------------------------------------------------------------
    let mobile_url = std::env::var("EF_PG_MOBILE_URL").unwrap_or_else(|_| base_url.clone());
    let mobile_cfg = PgConnectionConfig::parse_url(&mobile_url).expect("parse mobile PG URL");

    let mut mobile_svc_1 = MobileService::open_postgres(
        &mobile_cfg,
        Box::new(InMemoryReceiptJournal::new()),
        now_secs,
        true,
    )
    .expect("open MobileService on live PG18");
    mobile_svc_1
        .execute_restore_replay_gate(1_000)
        .expect("mobile restore replay gate 1");
    let mobile_srv_1 =
        TcpMobileServer::bind("127.0.0.1:0", mobile_svc_1).expect("bind mobile server 1");
    let m_addr_1 = mobile_srv_1.local_addr();

    let m_link = send_tcp_request(
        m_addr_1,
        "POST",
        "/elysianfields/mobile-api/auth/request-link",
        &[],
        r#"{"email":"pg.patient@example.invalid"}"#,
    );
    assert_eq!(m_link.status, 200);
    let m_tok = m_link.json()["token"]
        .as_str()
        .expect("mobile token")
        .to_owned();
    let m_verify = send_tcp_request(
        m_addr_1,
        "POST",
        "/elysianfields/mobile-api/auth/verify",
        &[],
        &format!(r#"{{"email":"pg.patient@example.invalid","token":"{m_tok}"}}"#),
    );
    assert_eq!(m_verify.status, 200);
    let m_verify_json = m_verify.json();
    let m_sess = m_verify_json["session_id"]
        .as_str()
        .expect("mobile session")
        .to_owned();
    let m_acct = m_verify_json["account_id"]
        .as_str()
        .expect("mobile account id")
        .to_owned();
    let m_inc = m_verify_json["incarnation"].as_u64().unwrap_or(1);
    let m_auth = format!("Bearer {m_sess}");
    let m_headers = [("Authorization", m_auth.as_str())];

    let batch_body = serde_json::json!({
        "items": [IncomingBatchItem::Valid(PatientRecord {
            record_id: "rec-pg18-1".to_owned(),
            version: 1,
            account_id: MobileAccountId::new(m_acct).expect("acct"),
            incarnation: AccountIncarnation::new(m_inc).expect("inc"),
            source_kind: ImportSourceKind::CcdaR21Xml,
            portal_id: None,
            bucket: ClinicalRecordBucket::Results,
            confidentiality: ConfidentialityTag::Standard,
            part2_notice: None,
            original_date: "2026-02-14".to_owned(),
            title: "PostgreSQL 18 BMP Panel".to_owned(),
            summary: "Sodium 140, Potassium 4.1, Creatinine 0.98".to_owned(),
            native_bytes: b"{}".to_vec(),
            removed_upstream: false,
        })]
    })
    .to_string();

    let ingest_resp = send_tcp_request(
        m_addr_1,
        "POST",
        "/elysianfields/mobile-api/records/ingest",
        &m_headers,
        &batch_body,
    );
    assert_eq!(
        ingest_resp.status, 200,
        "mobile ingest failed: {}",
        ingest_resp.body
    );

    // Kill Mobile server 1 and restart Mobile server 2 against the exact same PG18 database!
    mobile_srv_1.shutdown();

    let mut mobile_svc_2 = MobileService::open_postgres(
        &mobile_cfg,
        Box::new(InMemoryReceiptJournal::new()),
        now_secs + 30,
        true,
    )
    .expect("reopen MobileService on live PG18");
    mobile_svc_2
        .execute_restore_replay_gate(1_000)
        .expect("mobile restore replay gate 2");
    let mobile_srv_2 =
        TcpMobileServer::bind("127.0.0.1:0", mobile_svc_2).expect("bind mobile server 2");
    let m_addr_2 = mobile_srv_2.local_addr();

    let m_ready = send_tcp_request(m_addr_2, "GET", "/elysianfields/mobile-api/readyz", &[], "");
    assert_eq!(m_ready.status, 200);

    let m_records = send_tcp_request(
        m_addr_2,
        "GET",
        "/elysianfields/mobile-api/records",
        &m_headers,
        "",
    );
    assert_eq!(
        m_records.status, 200,
        "mobile records failed: {}",
        m_records.body
    );
    let rec_list = m_records.json()["records"]
        .as_array()
        .expect("persisted mobile records")
        .clone();
    assert_eq!(rec_list.len(), 1);
    assert_eq!(rec_list[0]["title"], "PostgreSQL 18 BMP Panel");

    // Verify both PG18 tables actually exist and hold the durable WAL rows.
    let mut clinic_check = PgClient::connect(&clinic_cfg).expect("clinic verify conn");
    let c_wal_count = clinic_check
        .query("SELECT COUNT(*) FROM ef_clinic_wal_events", &[])
        .expect("count clinic wal rows");
    assert!(c_wal_count[0].get_i64(0).expect("count").unwrap_or(0) >= 6);

    let mut mobile_check = PgClient::connect(&mobile_cfg).expect("mobile verify conn");
    let m_wal_count = mobile_check
        .query("SELECT COUNT(*) FROM ef_mobile_wal_events", &[])
        .expect("count mobile wal rows");
    assert!(m_wal_count[0].get_i64(0).expect("count").unwrap_or(0) >= 3);

    let _unused: BTreeMap<String, String> = BTreeMap::new();
    mobile_srv_2.shutdown();
}
