#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::prelude::*;
use ef_clinic_client::{
    ActiveWorkspaceView, ClientBootConfig, ClientClock, ClinicAction, ClinicActionMessage,
    ClinicSessionShell, ClinicTransport, ClinicTransportResource, CurieClinicClientPlugin,
    LiveHttpClinicTransport, SharedBrowserSessionHub, SharedLocaleStorage, SignInEntryMethod,
    SignInFormStatus, TodayRosterView, TransportError, is_strict_wasm_csp,
};
use ef_clinic_domain::auth::{
    AccountStanding, EmailAddress, PhysicianAccount, PhysicianRole, SignInMode, TabViewState,
};
use ef_clinic_domain::chat::{AnswerState, RetryBackoffMode};
use ef_clinic_domain::workspace::{
    CitationChipRef, CitationOrigin, CitationPanelState, CitedSourcePayload, PatientRosterState,
    ReadablePart, classify_record_status, synthetic_5000_row_lab_source,
};
use ef_clinic_server::{ClinicService, RateLimitConfig, STRICT_CSP, TcpClinicServer};
use ef_clinic_workflows::today::{
    ChannelHealthStatus, ScheduledVisitEntry, TodayItemKind, TodayQueueItem, TodayUrgency,
};
use ef_core::{
    IdempotencyKey, Locale, PhysicianId, PracticeId, SessionId, SourceKey, TabId, WorkspaceId,
};
use ef_harness::StubProvider;
use ef_ui_catalog::ComposerAction;

fn fresh_temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!(
        "ef-live-fullstack-{label}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("create temp test dir");
    dir
}

fn build_client_app(
    addr: SocketAddr,
    session_hub: SharedBrowserSessionHub,
    locale_storage: SharedLocaleStorage,
    now_secs: u64,
) -> App {
    let boot = ClientBootConfig::from_location("/elysianfields/").expect("valid subpath");
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(CurieClinicClientPlugin::new(
        TabId::new("tab-live-1").expect("tab id"),
        boot,
        session_hub,
        locale_storage,
    ));
    app.insert_resource(ClinicTransportResource::new(LiveHttpClinicTransport::new(
        addr,
    )));
    app.insert_resource(ClientClock {
        now_secs,
        now_ms: now_secs.saturating_mul(1_000),
    });
    app
}

fn dispatch(app: &mut App, action: ClinicAction) {
    app.world_mut().write_message(ClinicActionMessage(action));
    app.update();
}

fn synthetic_progress_note(workspace_id: WorkspaceId, source_key: SourceKey) -> CitedSourcePayload {
    CitedSourcePayload {
        source_key,
        workspace_id,
        number: 1,
        source_label: "Synthetic Nephrology Progress Note [1]".to_owned(),
        title: "Synthetic Outpatient Follow-Up Note".to_owned(),
        record_type: "progress_note".to_owned(),
        recorded_at: "2026-02-14T10:00:00Z".to_owned(),
        record_status: "final".to_owned(),
        status_class: classify_record_status("final"),
        origin: CitationOrigin::SourceRecord,
        original_language: Locale::En,
        quote: "Creatinine 1.02 mg/dL, stable eGFR, continue current regimen.".to_owned(),
        body: "Synthetic full-stack clinical note verifying live HTTP citation retrieval."
            .to_owned(),
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

fn seed_live_clinic_service(service: &mut ClinicService, now_secs: u64) {
    let practice_a = PracticeId::new("practice-a").expect("practice-a");
    let practice_b = PracticeId::new("practice-b").expect("practice-b");
    let doc_a = PhysicianId::new("phys-a1").expect("phys-a1");
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
        .expect("seed active physician A");

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
        .expect("seed active physician B");

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
        .expect("seed note [1]");
    service
        .seed_source(synthetic_5000_row_lab_source(
            ws_a1.clone(),
            SourceKey::new("src-lab-12").expect("src-lab-12"),
        ))
        .expect("seed 5000-row lab [12]");
    service
        .seed_source(synthetic_progress_note(
            ws_a1.clone(),
            SourceKey::new("src-withdrawn-9").expect("src-withdrawn-9"),
        ))
        .expect("seed withdrawn note [9]");
    service
        .withdraw_source(
            &ws_a1,
            &SourceKey::new("src-withdrawn-9").expect("src-withdrawn-9"),
        )
        .expect("withdraw [9]");

    service
        .seed_today_item(TodayQueueItem {
            item_id: "today-low-1".to_owned(),
            workspace_id: Some(ws_a1.clone()),
            urgency: TodayUrgency::Today,
            waiting_since_epoch_secs: now_secs.saturating_sub(300),
            kind: TodayItemKind::PreparedVisitBriefing,
            version_id: 1,
            summary: "Prepared renal follow-up briefing".to_owned(),
            can_mark_done: true,
        })
        .expect("seed today item");
    service
        .seed_today_item(TodayQueueItem {
            item_id: "now-urgent-1".to_owned(),
            workspace_id: Some(ws_a1.clone()),
            urgency: TodayUrgency::Now,
            waiting_since_epoch_secs: now_secs.saturating_sub(120),
            kind: TodayItemKind::UrgentClinicalConcern,
            version_id: 2,
            summary: "Urgent potassium elevation review".to_owned(),
            can_mark_done: true,
        })
        .expect("seed urgent item");
    service
        .seed_today_item(TodayQueueItem {
            item_id: "other-practice-item".to_owned(),
            workspace_id: Some(ws_b1),
            urgency: TodayUrgency::Now,
            waiting_since_epoch_secs: now_secs.saturating_sub(600),
            kind: TodayItemKind::UrgentClinicalConcern,
            version_id: 1,
            summary: "Must never leak across practices".to_owned(),
            can_mark_done: true,
        })
        .expect("seed cross-practice item");
    service
        .seed_scheduled_visit(ScheduledVisitEntry {
            workspace_id: ws_a1,
            display_name: "Synthetic Patient A1".to_owned(),
            scheduled_at_epoch_secs: now_secs.saturating_add(1_800),
            briefing_ready: true,
        })
        .expect("seed scheduled visit");
    service
        .set_channel_health(
            "spruce-main",
            ChannelHealthStatus::TempError {
                detail: "Temporary upstream channel hiccup".to_owned(),
            },
        )
        .expect("seed channel temp error");
}

#[test]
fn live_tcp_fullstack_client_server_workflows_restart_and_rate_limit() {
    let wal_dir = fresh_temp_dir("wal");
    let web_dir = fresh_temp_dir("web");
    let now_secs = 1_760_000_000u64;
    let default_src = SourceKey::new("src-note-1").expect("src-note-1");

    // Populate optional static web bundle directory served under `/elysianfields/`.
    fs::create_dir_all(web_dir.join("assets")).expect("create assets dir");
    fs::write(
        web_dir.join("index.html"),
        "<!DOCTYPE html><html><head><title>Elysian Web Shell</title></head><body><script src=\"/elysianfields/a11y-bridge.js\"></script></body></html>",
    )
    .expect("write index.html");
    fs::write(
        web_dir.join("a11y-bridge.js"),
        "export function initA11yBridge() { return true; }\n",
    )
    .expect("write a11y-bridge.js");
    fs::write(web_dir.join("assets/app.wasm"), b"\0asm\x01\x00\x00\x00").expect("write wasm");
    fs::write(
        web_dir.parent().expect("parent").join("outside-secret.txt"),
        "top-secret",
    )
    .expect("write outside sentinel");

    // -------------------------------------------------------------------------
    // 1. Boot real TCP `ef-clinic-server` on `127.0.0.1:0` with durable WAL + web_dir
    // -------------------------------------------------------------------------
    let mut service1 = ClinicService::open_durable(
        &wal_dir,
        SignInMode::LocalProof,
        RetryBackoffMode::DeterministicTest,
        default_src.clone(),
        now_secs,
    )
    .expect("open durable clinic service");
    service1.set_web_dir(web_dir.clone());
    seed_live_clinic_service(&mut service1, now_secs);

    let server1 = TcpClinicServer::bind("127.0.0.1:0", service1).expect("bind live TCP server 1");
    let addr1 = server1.local_addr();

    // Verify static asset serving, headers, and path-traversal rejection over live TCP.
    let mut http = LiveHttpClinicTransport::new(addr1);
    let redirect = http
        .request_raw("GET", "/elysianfields", None, None, &[])
        .expect("bare subpath request");
    assert_eq!(redirect.status, 308);
    assert_eq!(redirect.header("location"), Some("/elysianfields/"));

    let index_resp = http
        .request_raw("GET", "/elysianfields/", None, None, &[])
        .expect("index.html");
    assert_eq!(index_resp.status, 200);
    assert_eq!(index_resp.header("cache-control"), Some("no-cache"));
    assert_eq!(
        index_resp.header("content-security-policy"),
        Some(STRICT_CSP)
    );
    assert!(is_strict_wasm_csp(
        index_resp.header("content-security-policy").expect("csp")
    ));
    assert!(index_resp.header("last-modified").is_none());
    assert!(
        index_resp
            .body_text()
            .expect("utf8")
            .contains("Elysian Web Shell")
    );

    let js_resp = http
        .request_raw("GET", "/elysianfields/a11y-bridge.js", None, None, &[])
        .expect("a11y-bridge.js");
    assert_eq!(js_resp.status, 200);
    assert_eq!(
        js_resp.header("content-type"),
        Some("application/javascript; charset=utf-8")
    );
    assert!(js_resp.header("last-modified").is_none());

    let wasm_resp = http
        .request_raw("GET", "/elysianfields/assets/app.wasm", None, None, &[])
        .expect("app.wasm");
    assert_eq!(wasm_resp.status, 200);
    assert_eq!(wasm_resp.header("content-type"), Some("application/wasm"));
    assert_eq!(
        wasm_resp.header("cache-control"),
        Some("public, max-age=31536000, immutable")
    );
    assert!(wasm_resp.header("last-modified").is_none());
    assert_eq!(wasm_resp.body, b"\0asm\x01\x00\x00\x00");

    let traversal1 = http
        .request_raw(
            "GET",
            "/elysianfields/../outside-secret.txt",
            None,
            None,
            &[],
        )
        .expect("traversal 1");
    assert_eq!(traversal1.status, 404);
    let traversal2 = http
        .request_raw(
            "GET",
            "/elysianfields/%2e%2e/outside-secret.txt",
            None,
            None,
            &[],
        )
        .expect("traversal 2");
    assert_eq!(traversal2.status, 404);

    // -------------------------------------------------------------------------
    // 2. Drive real Bevy `ef-clinic-client` App over `LiveHttpClinicTransport`
    // -------------------------------------------------------------------------
    let hub1 = SharedBrowserSessionHub::new(None);
    let locale_storage = SharedLocaleStorage::new(None);
    let mut app1 = build_client_app(addr1, hub1.clone(), locale_storage.clone(), now_secs);

    // Boot -> SignInPage -> Request link -> Verify token
    dispatch(&mut app1, ClinicAction::BootTab);
    assert!(matches!(
        app1.world().resource::<ClinicSessionShell>().view_state,
        TabViewState::SignInPage { .. }
    ));

    dispatch(
        &mut app1,
        ClinicAction::SubmitSignInEmail {
            email: "clinician.a@example.invalid".to_owned(),
        },
    );
    let proof_token = match &app1.world().resource::<ClinicSessionShell>().sign_in_form {
        SignInFormStatus::LinkRequested {
            proof_token: Some(tok),
            ..
        } => tok.clone(),
        other => panic!("expected LocalProof token, got {other:?}"),
    };

    dispatch(
        &mut app1,
        ClinicAction::VerifySignInToken {
            email: "clinician.a@example.invalid".to_owned(),
            token: proof_token,
            entry: SignInEntryMethod::DirectForm,
        },
    );
    let session_token: SessionId = hub1
        .shared_storage()
        .expect("live authenticated session persisted in browser hub");
    assert!(matches!(
        app1.world().resource::<ClinicSessionShell>().view_state,
        TabViewState::Workspace { .. }
    ));

    // Verify patient roster loaded from live server with zero cross-practice leakage.
    match &app1.world().resource::<TodayRosterView>().state {
        PatientRosterState::Loaded { patients, .. } => {
            assert_eq!(patients.len(), 2);
            assert!(
                patients
                    .iter()
                    .all(|p| p.patient_workspace_id.as_str() != "ws-b1")
            );
        }
        other => panic!("expected loaded patient roster, got {other:?}"),
    }

    // Select patient workspace `ws-a1`
    let ws_a1 = WorkspaceId::new("ws-a1").expect("ws-a1");
    dispatch(
        &mut app1,
        ClinicAction::OpenPatientWorkspace {
            workspace_id: ws_a1.clone(),
            chat_id: None,
        },
    );
    {
        let ws = app1.world().resource::<ActiveWorkspaceView>();
        let open = ws.open_workspace.as_ref().expect("ws-a1 opened");
        assert_eq!(open.workspace_id, ws_a1);
        assert_eq!(open.display_name, "Synthetic Patient A1");
    }

    // Open chart evidence: citation [1] progress note, citation [12] 5,000-row virtualized lab table,
    // and withdrawn citation [9] (must be unavailable with zero clinical text).
    dispatch(
        &mut app1,
        ClinicAction::OpenCitationChip(CitationChipRef {
            source_key: SourceKey::new("src-note-1").expect("src-note-1"),
            number: 1,
            label: "Progress Note [1]".to_owned(),
        }),
    );
    assert!(matches!(
        app1.world()
            .resource::<ActiveWorkspaceView>()
            .citation_panel
            .state(),
        CitationPanelState::Loaded { source, .. } if source.number == 1 && source.quote.contains("Creatinine 1.02")
    ));

    dispatch(
        &mut app1,
        ClinicAction::OpenCitationChip(CitationChipRef {
            source_key: SourceKey::new("src-lab-12").expect("src-lab-12"),
            number: 12,
            label: "Lab Panel [12]".to_owned(),
        }),
    );
    {
        let ws = app1.world().resource::<ActiveWorkspaceView>();
        assert!(matches!(
            ws.citation_panel.state(),
            CitationPanelState::Loaded { source, .. } if source.fields.len() == 5_000
        ));
        let visible = ws.lab_table.visible_range();
        assert!(visible.materialized_count() > 0 && visible.materialized_count() < 40);
        assert!(visible.total_content_height_px > 100_000.0);
    }

    dispatch(
        &mut app1,
        ClinicAction::OpenCitationChip(CitationChipRef {
            source_key: SourceKey::new("src-withdrawn-9").expect("src-withdrawn-9"),
            number: 9,
            label: "Withdrawn [9]".to_owned(),
        }),
    );
    assert!(matches!(
        app1.world()
            .resource::<ActiveWorkspaceView>()
            .citation_panel
            .state(),
        CitationPanelState::Unavailable { .. }
    ));

    // Compose prompt -> autosave draft -> submit -> poll live SSE stream!
    dispatch(
        &mut app1,
        ClinicAction::ComposerEdit(ComposerAction::InsertText(
            "Review synthetic renal clearance trend.".to_owned(),
        )),
    );
    dispatch(&mut app1, ClinicAction::AutosaveDraft);
    dispatch(
        &mut app1,
        ClinicAction::SubmitMessage {
            idempotency_key: IdempotencyKey::new("live-turn-1").expect("idem 1"),
            delivery: None,
        },
    );
    dispatch(&mut app1, ClinicAction::PollStream);
    {
        let ws = app1.world().resource::<ActiveWorkspaceView>();
        let open = ws.open_workspace.as_ref().expect("ws open");
        assert_eq!(open.exchanges.len(), 1);
        assert!(matches!(
            open.exchanges[0].answer,
            AnswerState::Finished { .. }
        ));
        assert!(ws.delta_chunks_applied > 0);
        assert_eq!(ws.full_reparse_count, 0);
        assert!(!ws.available_citations().is_empty());
    }

    // Mid-stream cancel over live TCP HTTP/SSE (`cancel_after_chunks = Some(2)` -> `StopActiveWork`).
    {
        let mut cancel_provider = StubProvider::clinical_two_round_fixture(default_src.clone());
        cancel_provider.cancel_after_chunks = Some(2);
        server1
            .service()
            .lock()
            .expect("lock server1")
            .enqueue_provider_script(cancel_provider);
    }
    dispatch(
        &mut app1,
        ClinicAction::ComposerEdit(ComposerAction::InsertText(
            "Draft extended summary to cancel mid-stream.".to_owned(),
        )),
    );
    dispatch(
        &mut app1,
        ClinicAction::SubmitMessage {
            idempotency_key: IdempotencyKey::new("live-turn-2-cancel").expect("idem 2"),
            delivery: None,
        },
    );
    dispatch(&mut app1, ClinicAction::PollStream);
    dispatch(
        &mut app1,
        ClinicAction::StopActiveWork {
            idempotency_key: IdempotencyKey::new("live-stop-2").expect("stop 2"),
        },
    );
    dispatch(&mut app1, ClinicAction::PollStream);
    {
        let ws = app1.world().resource::<ActiveWorkspaceView>();
        let open = ws.open_workspace.as_ref().expect("ws open");
        assert_eq!(open.exchanges.len(), 2);
        assert!(matches!(
            open.exchanges[1].answer,
            AnswerState::Stopped { .. }
        ));
        assert!(
            !open.exchanges[1].kept_partial_text.is_empty(),
            "partial streamed answer must be preserved after cancel"
        );
    }

    // -------------------------------------------------------------------------
    // 3. Exercise M3 Clinic Workflows & Connected Care routes over live TCP
    // -------------------------------------------------------------------------
    // Today queue: `Now` ahead of `Today`, channel `TemporaryTrouble` with `asks_to_reconnect: false`, zero cross-practice leaks.
    let (today_status, today_headers, today_json) = http
        .request_json::<serde_json::Value>(
            "GET",
            "/elysianfields/api/today",
            Some(&session_token),
            None,
        )
        .expect("GET /today");
    assert_eq!(today_status, 200);
    assert!(
        today_headers
            .get("cache-control")
            .is_some_and(|v| v.contains("no-store"))
    );
    assert_eq!(today_json["mode"], "actionable");
    let today_items = today_json["items"].as_array().expect("today items");
    assert_eq!(today_items.len(), 2);
    assert_eq!(today_items[0]["item"]["item_id"], "now-urgent-1");
    assert_eq!(today_items[1]["item"]["item_id"], "today-low-1");
    let banners = today_json["channel_banners"]
        .as_array()
        .expect("channel banners");
    assert_eq!(banners.len(), 1);
    assert_eq!(banners[0]["kind"], "temporary_trouble");
    assert_eq!(banners[0]["asks_to_reconnect"], false);

    // Message matching: ingest -> decide (link to ws-a1) -> undo -> decide again
    let (ingest_status, _, ingest_json) = http
        .request_json(
            "POST",
            "/elysianfields/api/messages/ingest",
            Some(&session_token),
            Some(&serde_json::json!({
                "channel_id": "spruce-main",
                "cursor_seq": 1,
                "conversation_id": "conv-live-1",
                "message_id": "msg-1",
                "body": "My home BP was 128/78 today.",
                "auto_signal": {
                    "signal": "ambiguous_or_contradicted",
                    "suggested": "ws-a1"
                }
            })),
        )
        .expect("POST /messages/ingest");
    assert_eq!(ingest_status, 200);
    assert_eq!(ingest_json["state"]["state"], "to_match");

    let (decide1_status, _, decide1_json) = http
        .request_json(
            "POST",
            "/elysianfields/api/messages/decide",
            Some(&session_token),
            Some(&serde_json::json!({
                "conversation_id": "conv-live-1",
                "destination": { "kind": "patient", "workspace_id": "ws-a1" }
            })),
        )
        .expect("POST /messages/decide 1");
    assert_eq!(decide1_status, 200, "decide1 failed: {decide1_json}");
    assert_eq!(decide1_json["state"]["state"], "linked");

    let (undo_status, _, undo_json) = http
        .request_json(
            "POST",
            "/elysianfields/api/messages/undo",
            Some(&session_token),
            Some(&serde_json::json!({ "conversation_id": "conv-live-1" })),
        )
        .expect("POST /messages/undo");
    assert_eq!(undo_status, 200);
    assert_eq!(undo_json["state"]["state"], "to_match");

    // Re-link to ws-a1 so MessageReply workflow operates on the linked conversation.
    let (decide2_status, _, _) = http
        .request_json(
            "POST",
            "/elysianfields/api/messages/decide",
            Some(&session_token),
            Some(&serde_json::json!({
                "conversation_id": "conv-live-1",
                "destination": { "kind": "patient", "workspace_id": "ws-a1" },
            })),
        )
        .expect("POST /messages/decide 2");
    assert_eq!(decide2_status, 200);

    // Message replies: internal citation markup `[1]` must be rejected (400); clean reply accepted (200).
    let (bad_reply_status, _, _) = http
        .request_json(
            "POST",
            "/elysianfields/api/replies/conv-live-1/review",
            Some(&session_token),
            Some(&serde_json::json!({
                "proposed_reply_text": "Your BP looks great per chart [1]."
            })),
        )
        .expect("POST /replies/review forbidden markup");
    assert_eq!(bad_reply_status, 400);

    let (review_status, _, review_json) = http
        .request_json(
            "POST",
            "/elysianfields/api/replies/conv-live-1/review",
            Some(&session_token),
            Some(&serde_json::json!({
                "proposed_reply_text": "Thank you for sharing your home blood pressure reading. Continue your current medication.",
                "basis_citations": ["src-note-1"],
                "flagged_concern": "Monitor home BP log next visit"
            })),
        )
        .expect("POST /replies/review clean");
    assert_eq!(review_status, 200);
    assert_eq!(review_json["card"]["reply_state_label"], "ready");

    let (draft_status, _, draft_json) = http
        .request_json(
            "POST",
            "/elysianfields/api/replies/conv-live-1/draft",
            Some(&session_token),
            Some(&serde_json::json!({
                "text": "Thanks for checking in! Your BP of 128/78 looks right on target."
            })),
        )
        .expect("POST /replies/draft");
    assert_eq!(draft_status, 200);
    assert_eq!(draft_json["visible_reply_origin"], "personal_draft");

    let (copy_status, _, copy_json) = http
        .request_json::<serde_json::Value>(
            "POST",
            "/elysianfields/api/replies/conv-live-1/copy",
            Some(&session_token),
            None,
        )
        .expect("POST /replies/copy");
    assert_eq!(copy_status, 200);
    assert_eq!(
        copy_json["copied_text"],
        "Thanks for checking in! Your BP of 128/78 looks right on target."
    );

    let (close_status, _, close_json) = http
        .request_json::<serde_json::Value>(
            "POST",
            "/elysianfields/api/replies/conv-live-1/close",
            Some(&session_token),
            None,
        )
        .expect("POST /replies/close");
    assert_eq!(close_status, 200);
    assert_eq!(close_json["undo_available"], true);
    assert_eq!(
        close_json["flagged_concern"],
        "Monitor home BP log next visit"
    );

    let (undo_reply_status, _, undo_reply_json) = http
        .request_json::<serde_json::Value>(
            "POST",
            "/elysianfields/api/replies/conv-live-1/undo",
            Some(&session_token),
            None,
        )
        .expect("POST /replies/undo");
    assert_eq!(undo_reply_status, 200);
    assert_eq!(undo_reply_json["reply_state_label"], "ready");

    // Patient Memory: record physician instruction + run rounds with conflicting chart record + aborted run preserves watermark
    let (mem_rec_status, _, _) = http
        .request_json(
            "POST",
            "/elysianfields/api/patients/ws-a1/memory",
            Some(&session_token),
            Some(&serde_json::json!({
                "op": "record",
                "topic": "ace_inhibitor_target",
                "statement": "Hold dose increase until repeat BMP next month.",
                "observed_at": "2026-02-14",
                "authority": "physician_instruction"
            })),
        )
        .expect("POST memory record");
    assert_eq!(mem_rec_status, 200);

    let (mem_rounds_status, _, mem_rounds_json) = http
        .request_json(
            "POST",
            "/elysianfields/api/patients/ws-a1/memory",
            Some(&session_token),
            Some(&serde_json::json!({
                "op": "run_rounds",
                "observations": [{
                    "record_seq": 7,
                    "topic": "ace_inhibitor_target",
                    "statement": "External note suggests titrating lisinopril upward.",
                    "observed_at": "2026-02-15",
                    "authority": "chart_record"
                }],
                "abort_mid_run": false
            })),
        )
        .expect("POST memory run_rounds");
    assert_eq!(mem_rounds_status, 200);
    assert_eq!(mem_rounds_json["result"]["outcome"], "updated");
    assert_eq!(mem_rounds_json["result"]["disagreements_preserved"], 1);
    assert_eq!(mem_rounds_json["memory"]["rounds_watermark"], 7);

    let (mem_abort_status, _, mem_abort_json) = http
        .request_json(
            "POST",
            "/elysianfields/api/patients/ws-a1/memory",
            Some(&session_token),
            Some(&serde_json::json!({
                "op": "run_rounds",
                "observations": [{
                    "record_seq": 9,
                    "topic": "potassium",
                    "statement": "Should not commit on abort.",
                    "observed_at": "2026-02-16",
                    "authority": "chart_record"
                }],
                "abort_mid_run": true
            })),
        )
        .expect("POST memory abort");
    assert_eq!(mem_abort_status, 200);
    assert_eq!(mem_abort_json["result"]["outcome"], "aborted");
    assert_eq!(mem_abort_json["memory"]["rounds_watermark"], 7);

    // Connected Care Invitations: create -> get -> resend -> cancel -> re-create
    let (cc_create_status, _, cc_create_json) = http
        .request_json(
            "POST",
            "/elysianfields/api/connected-care/invitations",
            Some(&session_token),
            Some(&serde_json::json!({
                "patient_id": "pat-a1",
                "workspace_id": "ws-a1",
                "practice_name": "Elysian Renal Clinic",
                "physician_name": "Dr. Synthetic A",
                "expected_dob": { "year": 1974, "month": 6, "day": 15 }
            })),
        )
        .expect("POST connected-care invitation");
    assert_eq!(cc_create_status, 200);
    let inv_id_1 = cc_create_json["invitation_id"]
        .as_str()
        .expect("invitation_id")
        .to_owned();
    assert_eq!(cc_create_json["standing"]["state"], "Invited");

    let (cc_get_status, _, cc_get_json) = http
        .request_json::<serde_json::Value>(
            "GET",
            &format!("/elysianfields/api/connected-care/invitations/{inv_id_1}"),
            Some(&session_token),
            None,
        )
        .expect("GET connected-care invitation");
    assert_eq!(cc_get_status, 200);
    assert_eq!(cc_get_json["state"], "Invited");
    assert_eq!(cc_get_json["active_invitation_id"], inv_id_1);

    let (cc_resend_status, _, cc_resend_json) = http
        .request_json::<serde_json::Value>(
            "POST",
            &format!("/elysianfields/api/connected-care/invitations/{inv_id_1}/resend"),
            Some(&session_token),
            None,
        )
        .expect("POST connected-care resend");
    assert_eq!(cc_resend_status, 200);
    assert_eq!(cc_resend_json["invitation_id"], inv_id_1);
    assert_ne!(cc_resend_json["link_token"], cc_create_json["link_token"]);

    let (cc_cancel_status, _, cc_cancel_json) = http
        .request_json::<serde_json::Value>(
            "POST",
            &format!("/elysianfields/api/connected-care/invitations/{inv_id_1}/cancel"),
            Some(&session_token),
            None,
        )
        .expect("POST connected-care cancel");
    assert_eq!(cc_cancel_status, 200);
    assert_eq!(cc_cancel_json["standing"]["state"], "NotInvited");

    // Re-create a fresh open invitation to verify restart durability across server reboot.
    let (cc_create2_status, _, cc_create2_json) = http
        .request_json(
            "POST",
            "/elysianfields/api/connected-care/invitations",
            Some(&session_token),
            Some(&serde_json::json!({
                "patient_id": "pat-a1",
                "workspace_id": "ws-a1",
                "practice_name": "Elysian Renal Clinic",
                "physician_name": "Dr. Synthetic A",
                "expected_dob": { "year": 1974, "month": 6, "day": 15 }
            })),
        )
        .expect("POST connected-care invitation 2");
    assert_eq!(cc_create2_status, 200);
    let inv_id_2 = cc_create2_json["invitation_id"]
        .as_str()
        .expect("invitation_id 2")
        .to_owned();

    // -------------------------------------------------------------------------
    // 4. Server restart + client reload durable readback
    // -------------------------------------------------------------------------
    server1.shutdown();

    let mut service2 = ClinicService::open_durable(
        &wal_dir,
        SignInMode::LocalProof,
        RetryBackoffMode::DeterministicTest,
        default_src,
        now_secs + 60,
    )
    .expect("reopen durable WAL after restart");
    service2.set_web_dir(web_dir.clone());
    let server2 = TcpClinicServer::bind("127.0.0.1:0", service2).expect("bind live TCP server 2");
    let addr2 = server2.local_addr();

    // Fresh Bevy client App resuming with the persisted session token in shared storage.
    let hub2 = SharedBrowserSessionHub::new(Some(session_token.clone()));
    let mut app2 = build_client_app(addr2, hub2, locale_storage, now_secs + 60);

    dispatch(&mut app2, ClinicAction::BootTab);
    assert!(matches!(
        app2.world().resource::<ClinicSessionShell>().view_state,
        TabViewState::Workspace { .. }
    ));

    dispatch(
        &mut app2,
        ClinicAction::OpenPatientWorkspace {
            workspace_id: ws_a1,
            chat_id: None,
        },
    );
    {
        let ws = app2.world().resource::<ActiveWorkspaceView>();
        let open = ws.open_workspace.as_ref().expect("ws-a1 reloaded");
        assert_eq!(open.exchanges.len(), 2);
        assert!(matches!(
            open.exchanges[0].answer,
            AnswerState::Finished { .. }
        ));
        assert!(matches!(
            open.exchanges[1].answer,
            AnswerState::Stopped { .. }
        ));
        assert!(!open.exchanges[1].kept_partial_text.is_empty());
    }

    // Verify patient memory, Today queue, and Connected Care invitation survived server restart!
    let mut http2 = LiveHttpClinicTransport::new(addr2);
    let (mem_after_status, _, mem_after_json) = http2
        .request_json::<serde_json::Value>(
            "GET",
            "/elysianfields/api/patients/ws-a1/memory",
            Some(&session_token),
            None,
        )
        .expect("GET memory after restart");
    assert_eq!(mem_after_status, 200);
    assert_eq!(mem_after_json["rounds_watermark"], 7);
    assert_eq!(
        mem_after_json["entries"].as_array().expect("entries").len(),
        2
    );

    let (cc_after_status, _, cc_after_json) = http2
        .request_json::<serde_json::Value>(
            "GET",
            &format!("/elysianfields/api/connected-care/invitations/{inv_id_2}"),
            Some(&session_token),
            None,
        )
        .expect("GET connected-care after restart");
    assert_eq!(cc_after_status, 200);
    assert_eq!(cc_after_json["state"], "Invited");

    // -------------------------------------------------------------------------
    // 5. Per-IP sliding-window `429 Too Many Requests` + `Retry-After` + recovery
    // -------------------------------------------------------------------------
    {
        server2
            .service()
            .lock()
            .expect("lock server2")
            .set_rate_limit(RateLimitConfig {
                max_requests: 3,
                window_secs: 30,
            });
    }

    for _ in 0..3 {
        let who = http2.whoami(Some(&session_token), now_secs + 60);
        assert!(who.is_ok(), "first 3 requests within window must succeed");
    }

    let rate_limited_err = http2
        .whoami(Some(&session_token), now_secs + 60)
        .expect_err("4th request must hit 429 rate limit");
    assert!(
        matches!(rate_limited_err, TransportError::RateLimited { retry_after_secs } if retry_after_secs >= 1),
        "expected RateLimited, got {rate_limited_err:?}"
    );

    let raw_429 = http2
        .request_raw(
            "GET",
            "/elysianfields/api/auth/whoami",
            Some(&session_token),
            None,
            &[],
        )
        .expect("raw 429 response");
    assert_eq!(raw_429.status, 429);
    assert!(
        raw_429
            .header("retry-after")
            .and_then(|v| v.parse::<u64>().ok())
            .is_some_and(|secs| secs >= 1)
    );
    assert!(
        raw_429
            .header("cache-control")
            .is_some_and(|v| v.contains("no-store"))
    );

    // Advance server clock past the 30s sliding window -> requests succeed again!
    {
        server2
            .service()
            .lock()
            .expect("lock server2")
            .set_clock_secs(now_secs + 95);
    }
    assert!(http2.whoami(Some(&session_token), now_secs + 95).is_ok());

    server2.shutdown();
    let _ = fs::remove_dir_all(&wal_dir);
    let _ = fs::remove_dir_all(&web_dir);
}
