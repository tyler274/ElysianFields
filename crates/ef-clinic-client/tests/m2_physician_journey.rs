use std::sync::{Arc, Mutex};

use bevy::prelude::{App, MinimalPlugins};
use ef_clinic_client::{
    ActiveWorkspaceView, ClientBootConfig, ClientClock, ClientFrameReport, ClientRuntimeConfig,
    ClinicAction, ClinicActionMessage, ClinicEndpoint, ClinicSessionShell, ClinicTransport,
    ClinicTransportResource, CurieClinicClientPlugin, DraftBannerState, DraftConflictChoice,
    LocalVerticalSliceTransport, OriginalLanguageNotice, PutDraftRequest, STRICT_WASM_CSP,
    SharedBrowserSessionHub, SharedLocaleStorage, SignInEntryMethod, SignInFormStatus,
    TodayRosterView, VerticalSliceBackend, is_strict_wasm_csp,
};
use ef_clinic_domain::auth::{
    AccountStanding, EmailAddress, MailTransportStatus, PhysicianAccount, PhysicianRole,
    SignInMode, TabViewState, WorkspaceOpenView,
};
use ef_clinic_domain::chat::{
    AnswerState, CommandEnvelope, CommandPayload, ExchangeStep, ExchangeSteps, RetryBackoffMode,
    SubmitMessagePayload,
};
use ef_clinic_domain::workspace::{
    CitationChipRef, CitationFetchOutcome, CitationOrigin, CitationPanelState, CitedSourcePayload,
    MembershipTier, PatientRosterState, ReadablePart, RecordStatusClass, RosterAvailability,
    classify_record_status, synthetic_5000_row_lab_source,
};
use ef_core::{
    CachePolicy, ChatId, IdempotencyKey, Locale as CoreLocale, PhysicianId, PracticeId,
    RefusalCode, SourceKey, TabId, WorkspaceId,
};
use ef_harness::{CancellationToken, StubProvider, TurnExecutionOutcome};
use ef_ui_catalog::{
    A11yProjection, A11ySemanticKey, AriaBridgeEmitter, CatalogA11ySyncStatus, ComposerAction,
    ComposerSurface, FocusId, FrameDecision, HorizontalEdge, IdleRedrawGate,
    Locale as UiLocaleKind, ModifierChord, ShortcutInput, ShortcutKey, ShortcutRouter,
    TextDirection, UiKey, UiLocale, translate,
};

struct TestFixture {
    backend: Arc<Mutex<VerticalSliceBackend>>,
    practice_a: PracticeId,
    physician_a: PhysicianId,
    ws_a1: WorkspaceId,
    ws_a2_demo: WorkspaceId,
    ws_b1: WorkspaceId,
    chat_main: ChatId,
    source_note_1: SourceKey,
    source_lab_12: SourceKey,
    source_other_patient: SourceKey,
}

fn build_fixture(sign_in_mode: SignInMode) -> TestFixture {
    let practice_a = PracticeId::new("prac-a").expect("id");
    let practice_b = PracticeId::new("prac-b").expect("id");
    let physician_a = PhysicianId::new("phys-a1").expect("id");
    let physician_deact = PhysicianId::new("phys-deact").expect("id");
    let ws_a1 = WorkspaceId::new("pw-flagship-1").expect("id");
    let ws_a2_demo = WorkspaceId::new("pw-a2-demo").expect("id");
    let ws_b1 = WorkspaceId::new("pw-b1").expect("id");
    let chat_main = ChatId::new("chat-main").expect("id");
    let source_note_1 = SourceKey::new("src-note-1").expect("id");
    let source_lab_12 = SourceKey::new("src-lab-12").expect("id");
    let source_other_patient = SourceKey::new("src-other-b1").expect("id");

    let stub = StubProvider::clinical_two_round_fixture(source_note_1.clone());
    let mut backend =
        VerticalSliceBackend::new(sign_in_mode, RetryBackoffMode::DeterministicTest, stub);

    backend.auth.upsert_account(PhysicianAccount {
        physician_id: physician_a.clone(),
        practice_id: practice_a.clone(),
        email: EmailAddress::parse("clinician@practice-a.invalid").expect("email"),
        display_name: "Dr. Synthetic Alpha".to_owned(),
        role: PhysicianRole::Standard,
        standing: AccountStanding::Active,
        sessions_ended_before: None,
    });
    backend.auth.upsert_account(PhysicianAccount {
        physician_id: physician_deact,
        practice_id: practice_a.clone(),
        email: EmailAddress::parse("former@practice-a.invalid").expect("email"),
        display_name: "Dr. Former".to_owned(),
        role: PhysicianRole::Standard,
        standing: AccountStanding::Deactivated,
        sessions_ended_before: None,
    });

    backend.evidence.register_workspace(
        ws_a1.clone(),
        practice_a.clone(),
        "Synthetic Patient Flagship",
        vec![physician_a.clone()],
    );
    backend.evidence.register_workspace(
        ws_a2_demo.clone(),
        practice_a.clone(),
        "Synthetic Demographic Only",
        vec![],
    );
    backend.evidence.register_workspace(
        ws_b1.clone(),
        practice_b,
        "Synthetic Practice B Patient",
        vec![],
    );

    backend
        .default_chats
        .insert(ws_a1.clone(), chat_main.clone());
    backend.synopses.insert(
        ws_a1.clone(),
        "Synthetic renal and blood-pressure follow-up synopsis.".to_owned(),
    );

    // Source [1]: multi-section clinical progress note (English verbatim).
    backend.evidence.insert_source(CitedSourcePayload {
        source_key: source_note_1.clone(),
        workspace_id: ws_a1.clone(),
        number: 1,
        source_label: "Progress Note [1]".to_owned(),
        title: "Synthetic Outpatient Progress Note".to_owned(),
        record_type: "progress_note".to_owned(),
        recorded_at: "2026-01-12T09:15:00Z".to_owned(),
        record_status: "preliminary".to_owned(),
        status_class: classify_record_status("preliminary"),
        origin: CitationOrigin::SourceRecord,
        original_language: CoreLocale::En,
        quote: "Creatinine 1.02 mg/dL; eGFR 54 mL/min/1.73m².".to_owned(),
        body: "Assessment: Creatinine 1.02 mg/dL; eGFR 54 mL/min/1.73m² stable.".to_owned(),
        readable_parts: vec![
            ReadablePart {
                label: "Assessment".to_owned(),
                text: "Creatinine 1.02 mg/dL; eGFR 54 mL/min/1.73m² stable.".to_owned(),
                unreadable_notice: None,
            },
            ReadablePart {
                label: "Scanned Attachment Page 2".to_owned(),
                text: String::new(),
                unreadable_notice: Some("Attachment page 2 could not be decoded.".to_owned()),
            },
        ],
        fields: Vec::new(),
        raw_content: None,
        external_url: None,
    });

    // Source [12]: 5,000-row structured clinical laboratory table.
    backend
        .evidence
        .insert_source(synthetic_5000_row_lab_source(
            ws_a1.clone(),
            source_lab_12.clone(),
        ));

    // Cross-patient source strictly owned by ws_b1.
    backend.evidence.insert_source(CitedSourcePayload {
        source_key: source_other_patient.clone(),
        workspace_id: ws_b1.clone(),
        number: 99,
        source_label: "Practice B Secret Note".to_owned(),
        title: "Practice B Confidential Note".to_owned(),
        record_type: "progress_note".to_owned(),
        recorded_at: "2026-01-10T08:00:00Z".to_owned(),
        record_status: "final".to_owned(),
        status_class: RecordStatusClass::Settled,
        origin: CitationOrigin::SourceRecord,
        original_language: CoreLocale::En,
        quote: "Cross-practice data must never leak.".to_owned(),
        body: "Cross-practice data must never leak.".to_owned(),
        readable_parts: Vec::new(),
        fields: Vec::new(),
        raw_content: None,
        external_url: None,
    });

    TestFixture {
        backend: Arc::new(Mutex::new(backend)),
        practice_a,
        physician_a,
        ws_a1,
        ws_a2_demo,
        ws_b1,
        chat_main,
        source_note_1,
        source_lab_12,
        source_other_patient,
    }
}

fn spawn_client_app(
    tab_name: &str,
    location: &str,
    backend: Arc<Mutex<VerticalSliceBackend>>,
    hub: SharedBrowserSessionHub,
    locale_storage: SharedLocaleStorage,
) -> App {
    let boot = ClientBootConfig::from_location(location).expect("valid mounted boot location");
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        CurieClinicClientPlugin::new(
            TabId::new(tab_name).expect("tab id"),
            boot,
            hub,
            locale_storage,
        ),
    ));
    app.insert_resource(ClinicTransportResource::new(
        LocalVerticalSliceTransport::new(backend),
    ));
    app
}

fn dispatch(app: &mut App, action: ClinicAction) {
    app.world_mut().write_message(ClinicActionMessage(action));
    app.update();
}

fn sign_in_local_proof(app: &mut App, email: &str) {
    dispatch(app, ClinicAction::BootTab);
    dispatch(
        app,
        ClinicAction::SubmitSignInEmail {
            email: email.to_owned(),
        },
    );
    let token = match &app.world().resource::<ClinicSessionShell>().sign_in_form {
        SignInFormStatus::LinkRequested {
            proof_token: Some(tok),
            ..
        } => tok.clone(),
        other => panic!("expected proof token, got {other:?}"),
    };
    dispatch(
        app,
        ClinicAction::VerifySignInToken {
            email: email.to_owned(),
            token,
            entry: SignInEntryMethod::DirectForm,
        },
    );
}

#[test]
fn scn_m2_0001_invite_only_sign_in_single_use_verify_and_anti_enumeration() {
    let fx = build_fixture(SignInMode::Hosted);
    let hub = SharedBrowserSessionHub::new(None);
    let locale_store = SharedLocaleStorage::default();
    let mut app = spawn_client_app(
        "tab-1",
        "/elysianfields/",
        Arc::clone(&fx.backend),
        hub,
        locale_store,
    );
    dispatch(&mut app, ClinicAction::BootTab);

    // Hosted anti-enumeration: unknown, deactivated, and active return identical response.
    let mut messages = Vec::new();
    for email in [
        "unknown@example.invalid",
        "former@practice-a.invalid",
        "clinician@practice-a.invalid",
    ] {
        dispatch(
            &mut app,
            ClinicAction::SubmitSignInEmail {
                email: email.to_owned(),
            },
        );
        match &app.world().resource::<ClinicSessionShell>().sign_in_form {
            SignInFormStatus::LinkRequested {
                message,
                proof_token,
            } => {
                assert!(proof_token.is_none(), "hosted mode must never leak token");
                messages.push(message.clone());
            }
            other => panic!("expected anti-enumeration LinkRequested, got {other:?}"),
        }
    }
    assert_eq!(messages[0], messages[1]);
    assert_eq!(messages[1], messages[2]);

    // Exactly ONE outbound email was sent (strictly for the active invited clinician).
    let issued_token = {
        let guard = fx.backend.lock().expect("lock");
        assert_eq!(guard.auth.sent_mail().len(), 1);
        assert_eq!(
            guard.auth.sent_mail()[0].recipient.as_str(),
            "clinician@practice-a.invalid"
        );
        guard.auth.sent_mail()[0].token.clone()
    };

    // First verify succeeds and lands on Today with Cache-Control: no-store.
    dispatch(
        &mut app,
        ClinicAction::VerifySignInToken {
            email: "clinician@practice-a.invalid".to_owned(),
            token: issued_token.clone(),
            entry: SignInEntryMethod::DirectForm,
        },
    );
    assert!(matches!(
        app.world().resource::<ClinicSessionShell>().view_state,
        TabViewState::Workspace {
            opens: WorkspaceOpenView::Today,
            ..
        }
    ));
    let meta = app
        .world()
        .resource::<ClinicTransportResource>()
        .transport()
        .last_response_meta()
        .expect("recorded meta");
    assert_eq!(meta.cache_policy, CachePolicy::ClinicalNoStore);

    // Replaying the exact same single-use token is rejected (401 session_ended).
    dispatch(
        &mut app,
        ClinicAction::VerifySignInToken {
            email: "clinician@practice-a.invalid".to_owned(),
            token: issued_token,
            entry: SignInEntryMethod::DirectForm,
        },
    );
    assert!(matches!(
        app.world().resource::<ClinicSessionShell>().sign_in_form,
        SignInFormStatus::Refused {
            code: RefusalCode::SessionEnded,
            ..
        }
    ));

    // Deactivated account verification returns 403 access_removed.
    dispatch(
        &mut app,
        ClinicAction::VerifySignInToken {
            email: "former@practice-a.invalid".to_owned(),
            token: "any-token".to_owned(),
            entry: SignInEntryMethod::DirectForm,
        },
    );
    assert!(matches!(
        app.world().resource::<ClinicSessionShell>().sign_in_form,
        SignInFormStatus::Refused {
            code: RefusalCode::AccessRemoved,
            ..
        }
    ));

    // Mail delivery failure returns explicit 502 error.
    fx.backend
        .lock()
        .expect("lock")
        .auth
        .set_mail_status(MailTransportStatus::DeliveryFailed);
    dispatch(
        &mut app,
        ClinicAction::SubmitSignInEmail {
            email: "clinician@practice-a.invalid".to_owned(),
        },
    );
    assert!(matches!(
        app.world().resource::<ClinicSessionShell>().sign_in_form,
        SignInFormStatus::MailDeliveryFailed { .. }
    ));
}

#[test]
fn scn_m2_0002_two_tab_sign_out_expiry_clearing_and_keeps_work_race() {
    let fx = build_fixture(SignInMode::LocalProof);
    let hub = SharedBrowserSessionHub::new(None);
    let locale_store = SharedLocaleStorage::default();

    let mut tab_a = spawn_client_app(
        "tab-a",
        "/elysianfields/",
        Arc::clone(&fx.backend),
        hub.clone(),
        locale_store.clone(),
    );
    let mut tab_b = spawn_client_app(
        "tab-b",
        "/elysianfields/",
        Arc::clone(&fx.backend),
        hub.clone(),
        locale_store,
    );

    // Sign in on Tab A -> Session 1; Tab B adopts Session 1; both open patient workspace.
    sign_in_local_proof(&mut tab_a, "clinician@practice-a.invalid");
    let session_1 = hub.shared_storage().expect("session 1 stored");
    dispatch(&mut tab_b, ClinicAction::BootTab);
    dispatch(
        &mut tab_a,
        ClinicAction::OpenPatientWorkspace {
            workspace_id: fx.ws_a1.clone(),
            chat_id: None,
        },
    );
    dispatch(
        &mut tab_b,
        ClinicAction::OpenPatientWorkspace {
            workspace_id: fx.ws_a1.clone(),
            chat_id: None,
        },
    );

    // Expire Session 1 server-side -> Tab A receives 401 -> SessionEndedPrompt with ZERO patient content.
    {
        let mut guard = fx.backend.lock().expect("lock");
        guard.auth.expire_session(&session_1, 1_000);
    }
    dispatch(
        &mut tab_a,
        ClinicAction::HandleSessionEndedRefusal {
            failed_session: session_1.clone(),
        },
    );
    assert_eq!(
        tab_a.world().resource::<ClinicSessionShell>().view_state,
        TabViewState::SessionEndedPrompt
    );
    assert!(
        tab_a
            .world()
            .resource::<ActiveWorkspaceView>()
            .open_workspace
            .is_none(),
        "PromptIsEmpty: zero patient content retained on SessionEndedPrompt"
    );

    // Tab B signs in freshly (Session 2) while an older Session 1 401 arrives in Tab A.
    dispatch(&mut tab_b, ClinicAction::ReconcilePeerTabs);
    sign_in_local_proof(&mut tab_b, "clinician@practice-a.invalid");
    let session_2 = hub.shared_storage().expect("session 2 stored");
    assert_ne!(session_1, session_2);

    // Late 401 for Session 1 in Tab A must adopt Session 2 and land on Today (`KeepsWork` + `TodayFirst`).
    dispatch(
        &mut tab_a,
        ClinicAction::HandleSessionEndedRefusal {
            failed_session: session_1.clone(),
        },
    );
    assert!(matches!(
        tab_a.world().resource::<ClinicSessionShell>().view_state,
        TabViewState::Workspace {
            ref holds,
            opens: WorkspaceOpenView::Today,
            sse_connected: true,
        } if holds == &session_2
    ));

    // Sign out in Tab A -> Tab B reconciles, clears patient content, and disconnects SSE (`OnceOut`).
    dispatch(&mut tab_a, ClinicAction::SignOut);
    dispatch(&mut tab_b, ClinicAction::ReconcilePeerTabs);
    assert!(!tab_b.world().resource::<ClinicSessionShell>().sse_connected);
    assert!(
        tab_b
            .world()
            .resource::<ActiveWorkspaceView>()
            .open_workspace
            .is_none()
    );

    // Verify all formal sign-in safety invariants across both tabs & server (`ServerRefusesSignedOut`).
    let mut verifier_transport = LocalVerticalSliceTransport::new(Arc::clone(&fx.backend));
    hub.check_safety_invariants(&mut verifier_transport, 1_000)
        .expect("all formal sign-in safety invariants must hold");
    assert!(verifier_transport.whoami(Some(&session_1), 1_000).is_err());
    assert!(verifier_transport.whoami(Some(&session_2), 1_000).is_err());
}

#[test]
fn scn_m2_0003_deep_link_patient_landing_across_unauthenticated_boot() {
    let fx = build_fixture(SignInMode::LocalProof);
    let hub = SharedBrowserSessionHub::new(None);
    let locale_store = SharedLocaleStorage::default();

    let mut tab_a = spawn_client_app(
        "tab-deeplink",
        "/elysianfields/?patient=pw-flagship-1",
        Arc::clone(&fx.backend),
        hub.clone(),
        locale_store.clone(),
    );
    let mut tab_b = spawn_client_app(
        "tab-sibling",
        "/elysianfields/",
        Arc::clone(&fx.backend),
        hub.clone(),
        locale_store.clone(),
    );

    // Tab A boots unauthenticated -> SignInPage with zero patient content while preserving deep link.
    dispatch(&mut tab_a, ClinicAction::BootTab);
    assert!(matches!(
        tab_a.world().resource::<ClinicSessionShell>().view_state,
        TabViewState::SignInPage { .. }
    ));
    assert!(
        tab_a
            .world()
            .resource::<ActiveWorkspaceView>()
            .open_workspace
            .is_none()
    );

    // Sibling Tab B signs in directly -> Tab A reconciles and lands directly on pw-flagship-1 (`Lands`).
    sign_in_local_proof(&mut tab_b, "clinician@practice-a.invalid");
    dispatch(&mut tab_a, ClinicAction::ReconcilePeerTabs);
    assert!(matches!(
        tab_a.world().resource::<ClinicSessionShell>().view_state,
        TabViewState::Workspace {
            opens: WorkspaceOpenView::LinkedPatient(ref ws),
            ..
        } if ws == &fx.ws_a1
    ));
    assert_eq!(
        tab_a
            .world()
            .resource::<ActiveWorkspaceView>()
            .open_workspace
            .as_ref()
            .expect("patient loaded")
            .workspace_id,
        fx.ws_a1
    );

    // Separate tab verifying an emailed sign-in link always lands on Today (`TodayFirst`).
    let hub_email = SharedBrowserSessionHub::new(None);
    let mut tab_email = spawn_client_app(
        "tab-email",
        "/elysianfields/?patient=pw-flagship-1",
        Arc::clone(&fx.backend),
        hub_email,
        locale_store,
    );
    dispatch(&mut tab_email, ClinicAction::BootTab);
    dispatch(
        &mut tab_email,
        ClinicAction::SubmitSignInEmail {
            email: "clinician@practice-a.invalid".to_owned(),
        },
    );
    let token = match &tab_email
        .world()
        .resource::<ClinicSessionShell>()
        .sign_in_form
    {
        SignInFormStatus::LinkRequested {
            proof_token: Some(tok),
            ..
        } => tok.clone(),
        other => panic!("expected proof token, got {other:?}"),
    };
    dispatch(
        &mut tab_email,
        ClinicAction::VerifySignInToken {
            email: "clinician@practice-a.invalid".to_owned(),
            token,
            entry: SignInEntryMethod::EmailedLink,
        },
    );
    assert!(matches!(
        tab_email
            .world()
            .resource::<ClinicSessionShell>()
            .view_state,
        TabViewState::Workspace {
            opens: WorkspaceOpenView::Today,
            ..
        }
    ));
}

#[test]
fn scn_m2_0004_practice_scoped_three_tier_access_and_honest_roster_states() {
    let fx = build_fixture(SignInMode::LocalProof);
    let hub = SharedBrowserSessionHub::new(None);
    let locale_store = SharedLocaleStorage::default();
    let mut app = spawn_client_app(
        "tab-roster",
        "/elysianfields/",
        Arc::clone(&fx.backend),
        hub.clone(),
        locale_store,
    );
    sign_in_local_proof(&mut app, "clinician@practice-a.invalid");
    let session = hub.shared_storage().expect("session");

    // Roster contains ONLY Practice A entries (never Practice B's pw-b1).
    match &app.world().resource::<TodayRosterView>().state {
        PatientRosterState::Loaded { patients, .. } => {
            assert_eq!(patients.len(), 2);
            assert!(patients.iter().any(|p| p.patient_workspace_id == fx.ws_a1));
            assert!(
                patients
                    .iter()
                    .any(|p| p.patient_workspace_id == fx.ws_a2_demo)
            );
            assert!(!patients.iter().any(|p| p.patient_workspace_id == fx.ws_b1));
        }
        other => panic!("expected Loaded roster, got {other:?}"),
    }

    // Cross-practice requests for pw-b1 return 404 Not Found with Cache-Control: no-store.
    let mut transport = LocalVerticalSliceTransport::new(Arc::clone(&fx.backend));
    let err_detail = transport
        .fetch_workspace_detail(&session, &fx.ws_b1, 1_000)
        .expect_err("cross-practice workspace detail must fail");
    assert_eq!(err_detail.http_status_code(), 404);
    assert_eq!(
        transport.last_response_meta().expect("meta").cache_policy,
        CachePolicy::ClinicalNoStore
    );

    let err_snap = transport
        .fetch_chat_snapshot(&session, &fx.ws_b1, &fx.chat_main, 1_000)
        .expect_err("cross-practice chat snapshot must fail");
    assert_eq!(err_snap.http_status_code(), 404);

    let err_cmd = transport
        .submit_command(
            &session,
            &CommandEnvelope {
                idempotency_key: IdempotencyKey::new("idem-cross-b1").expect("key"),
                command: CommandPayload::MessageSubmit(SubmitMessagePayload {
                    patient_workspace_id: fx.ws_b1.clone(),
                    chat_id: fx.chat_main.clone(),
                    text: "unauthorized".to_owned(),
                    physician_time_zone: "UTC".to_owned(),
                    delivery: None,
                }),
            },
            1_000,
        )
        .expect_err("cross-practice command must fail");
    assert_eq!(err_cmd.http_status_code(), 404);

    // ClinicDemographic patient opens demographic view ONLY (no clinical synopsis, transcript, or composer).
    dispatch(
        &mut app,
        ClinicAction::OpenPatientWorkspace {
            workspace_id: fx.ws_a2_demo.clone(),
            chat_id: None,
        },
    );
    let open = app
        .world()
        .resource::<ActiveWorkspaceView>()
        .open_workspace
        .as_ref()
        .expect("demographic view open");
    assert_eq!(open.membership, MembershipTier::ClinicDemographic);
    assert!(open.synopsis.is_none());
    assert!(open.exchanges.is_empty());

    // Partial outage sets Incomplete honestly; full store outage renders Failed (never Empty).
    fx.backend.lock().expect("lock").partial_roster_outage = true;
    dispatch(&mut app, ClinicAction::LoadPatientRoster { query: None });
    assert!(matches!(
        app.world().resource::<TodayRosterView>().state,
        PatientRosterState::Loaded {
            availability: RosterAvailability::Incomplete {
                may_be_missing_someone: true
            },
            ..
        }
    ));

    fx.backend
        .lock()
        .expect("lock")
        .evidence
        .set_store_outage(Some("access store timeout".to_owned()));
    dispatch(&mut app, ClinicAction::LoadPatientRoster { query: None });
    assert!(matches!(
        app.world().resource::<TodayRosterView>().state,
        PatientRosterState::Failed { .. }
    ));
}

#[test]
fn scn_m2_0005_evidence_reading_5000_row_virtualized_citations_and_redaction() {
    let fx = build_fixture(SignInMode::LocalProof);
    let hub = SharedBrowserSessionHub::new(None);
    let locale_store = SharedLocaleStorage::default();
    let mut app = spawn_client_app(
        "tab-cite",
        "/elysianfields/",
        Arc::clone(&fx.backend),
        hub.clone(),
        locale_store,
    );
    sign_in_local_proof(&mut app, "clinician@practice-a.invalid");
    let session = hub.shared_storage().expect("session");
    dispatch(
        &mut app,
        ClinicAction::OpenPatientWorkspace {
            workspace_id: fx.ws_a1.clone(),
            chat_id: None,
        },
    );

    // Open citation [1] (multi-section progress note with preliminary flagged badge & unreadable page notice).
    let chip_1 = CitationChipRef {
        source_key: fx.source_note_1.clone(),
        number: 1,
        label: "Progress Note [1]".to_owned(),
    };
    dispatch(&mut app, ClinicAction::OpenCitationChip(chip_1.clone()));
    let first_req_id = match app
        .world()
        .resource::<ActiveWorkspaceView>()
        .citation_panel
        .state()
    {
        CitationPanelState::Loaded {
            request_id, source, ..
        } => {
            assert_eq!(source.status_class, RecordStatusClass::Flagged);
            assert!(source.quote.contains("Creatinine 1.02 mg/dL"));
            assert!(source.readable_parts[1].unreadable_notice.is_some());
            *request_id
        }
        other => panic!("expected Loaded citation [1], got {other:?}"),
    };

    // Open citation [12] (5,000-row lab table): must virtualize to O(viewport) rows.
    let chip_12 = CitationChipRef {
        source_key: fx.source_lab_12.clone(),
        number: 12,
        label: "Lab Panel [12]".to_owned(),
    };
    dispatch(&mut app, ClinicAction::OpenCitationChip(chip_12));
    {
        let ws = app.world().resource::<ActiveWorkspaceView>();
        let CitationPanelState::Loaded { source, .. } = ws.citation_panel.state() else {
            panic!("expected Loaded citation [12]");
        };
        assert_eq!(source.fields.len(), 5_000);
        let (range, visible_slice) = ws.lab_table.visible_rows_slice(&source.fields);
        assert!(
            range.materialized_count() <= 25,
            "5,000-row table must materialize O(viewport) rows, got {}",
            range.materialized_count()
        );
        assert_eq!(visible_slice.len(), range.materialized_count());
    }

    // Scroll deep into the 5,000-row lab table.
    dispatch(
        &mut app,
        ClinicAction::ScrollLabTable {
            offset_px: 70_000.0,
        },
    );
    assert!(
        app.world()
            .resource::<ActiveWorkspaceView>()
            .lab_table
            .visible_range()
            .start_row
            > 2_000
    );

    // Monotonic RequestId stale-discard: late response for chip [1]'s older RequestId is ignored!
    dispatch(
        &mut app,
        ClinicAction::DeliverCitationResponse {
            request_id: first_req_id,
            outcome: CitationFetchOutcome::Failed {
                message: "stale failure".to_owned(),
            },
        },
    );
    assert!(
        matches!(
            app.world()
                .resource::<ActiveWorkspaceView>()
                .citation_panel
                .state(),
            CitationPanelState::Loaded { chip, .. } if chip.number == 12
        ),
        "stale RequestId must never overwrite active citation [12]"
    );

    // Withdraw source [1] server-side and re-open -> transitions immediately to Unavailable.
    assert!(
        fx.backend
            .lock()
            .expect("lock")
            .evidence
            .withdraw_source(&fx.ws_a1, &fx.source_note_1)
    );
    dispatch(&mut app, ClinicAction::OpenCitationChip(chip_1));
    assert!(matches!(
        app.world()
            .resource::<ActiveWorkspaceView>()
            .citation_panel
            .state(),
        CitationPanelState::Unavailable { .. }
    ));

    // Cross-patient source_key lookup returns 404 Not Found.
    let mut transport = LocalVerticalSliceTransport::new(Arc::clone(&fx.backend));
    let err = transport
        .fetch_cited_source(
            &session,
            &fx.ws_a1,
            &fx.chat_main,
            &fx.source_other_patient,
            1_000,
        )
        .expect_err("cross-patient citation must fail");
    assert_eq!(err.http_status_code(), 404);
}

#[test]
fn scn_m2_0006_streamed_clinical_chat_tool_trace_idempotency_and_idle_zero_frames() {
    let fx = build_fixture(SignInMode::LocalProof);
    let hub = SharedBrowserSessionHub::new(None);
    let locale_store = SharedLocaleStorage::default();
    let mut app = spawn_client_app(
        "tab-stream",
        "/elysianfields/",
        Arc::clone(&fx.backend),
        hub,
        locale_store,
    );
    sign_in_local_proof(&mut app, "clinician@practice-a.invalid");
    dispatch(
        &mut app,
        ClinicAction::OpenPatientWorkspace {
            workspace_id: fx.ws_a1.clone(),
            chat_id: None,
        },
    );

    // Type prompt and submit with idempotency_key.
    dispatch(
        &mut app,
        ClinicAction::ComposerEdit(ComposerAction::InsertText(
            "Summarize renal clearance trend.".to_owned(),
        )),
    );
    let idem = IdempotencyKey::new("idem-turn-1").expect("key");
    dispatch(
        &mut app,
        ClinicAction::SubmitMessage {
            idempotency_key: idem.clone(),
            delivery: None,
        },
    );
    // Duplicate submission with the same idempotency_key must create ZERO duplicate turns.
    dispatch(
        &mut app,
        ClinicAction::ComposerEdit(ComposerAction::InsertText(
            "Summarize renal clearance trend.".to_owned(),
        )),
    );
    dispatch(
        &mut app,
        ClinicAction::SubmitMessage {
            idempotency_key: idem,
            delivery: None,
        },
    );
    assert_eq!(
        app.world()
            .resource::<ActiveWorkspaceView>()
            .open_workspace
            .as_ref()
            .expect("open")
            .exchanges
            .len(),
        1
    );

    // Run deterministic 2-round tool + streamed clinical answer via ef-harness StubProvider.
    let cancel = CancellationToken::new();
    let outcome = fx
        .backend
        .lock()
        .expect("lock")
        .run_next_worker_attempt(&fx.practice_a, &fx.physician_a, "worker-1", &cancel, 1_005)
        .expect("worker ok")
        .expect("claimed turn");
    assert!(matches!(outcome.1, TurnExecutionOutcome::Finished { .. }));

    // Concurrently poll SSE deltas, type in composer, and scroll transcript.
    app.world_mut()
        .resource_mut::<ClientClock>()
        .advance_ms(900);
    dispatch(&mut app, ClinicAction::PollStream);
    dispatch(
        &mut app,
        ClinicAction::ComposerEdit(ComposerAction::InsertText("Follow-up draft".to_owned())),
    );
    dispatch(&mut app, ClinicAction::ScrollTranscript { offset_px: 48.0 });

    let ws = app.world().resource::<ActiveWorkspaceView>();
    assert_eq!(
        ws.full_reparse_count, 0,
        "incremental SSE delta delivery must perform 0 full reparses"
    );
    assert!(ws.delta_chunks_applied >= 4);
    let open = ws.open_workspace.as_ref().expect("open");
    assert_eq!(open.exchanges.len(), 1);
    let ex = &open.exchanges[0];
    assert!(matches!(
        &ex.steps,
        ExchangeSteps::Read { steps }
            if matches!(steps.first(), Some(ExchangeStep::SearchedChart { .. }))
                && matches!(steps.get(1), Some(ExchangeStep::ReadRecords { .. }))
    ));
    assert!(matches!(ex.answer, AnswerState::Finished { .. }));
    assert_eq!(ws.available_citations().len(), 1);

    // Idle phase: 60 frames with no input/stream must submit ZERO redraws (`IdleRedrawGate`).
    let rendered_before_idle = app.world().resource::<IdleRedrawGate>().frames_rendered();
    for _ in 0..60 {
        app.update();
        assert_eq!(
            app.world().resource::<ClientFrameReport>().last_decision,
            Some(FrameDecision::SkipIdle)
        );
    }
    assert_eq!(
        app.world().resource::<IdleRedrawGate>().frames_rendered(),
        rendered_before_idle
    );
}

#[test]
fn scn_m2_0007_mid_stream_cancellation_stop_and_fenced_resume() {
    let fx = build_fixture(SignInMode::LocalProof);
    fx.backend
        .lock()
        .expect("lock")
        .stub_provider
        .cancel_after_chunks = Some(2);

    let hub = SharedBrowserSessionHub::new(None);
    let locale_store = SharedLocaleStorage::default();
    let mut app = spawn_client_app(
        "tab-stop",
        "/elysianfields/",
        Arc::clone(&fx.backend),
        hub.clone(),
        locale_store,
    );
    sign_in_local_proof(&mut app, "clinician@practice-a.invalid");
    let session = hub.shared_storage().expect("session");
    dispatch(
        &mut app,
        ClinicAction::OpenPatientWorkspace {
            workspace_id: fx.ws_a1.clone(),
            chat_id: None,
        },
    );

    dispatch(
        &mut app,
        ClinicAction::ComposerEdit(ComposerAction::InsertText(
            "Check potassium and dosing.".to_owned(),
        )),
    );
    dispatch(
        &mut app,
        ClinicAction::SubmitMessage {
            idempotency_key: IdempotencyKey::new("idem-stop-1").expect("key"),
            delivery: None,
        },
    );

    // Run worker until mid-stream pause (2 chunks streamed, partial work kept).
    let cancel = CancellationToken::new();
    let (work_id, out) = fx
        .backend
        .lock()
        .expect("lock")
        .run_next_worker_attempt(&fx.practice_a, &fx.physician_a, "worker-1", &cancel, 1_010)
        .expect("ok")
        .expect("claimed");
    assert_eq!(out, TurnExecutionOutcome::StoppedMidStream);
    dispatch(&mut app, ClinicAction::PollStream);

    // Clinician clicks Stop (`work.stop`).
    dispatch(
        &mut app,
        ClinicAction::StopActiveWork {
            idempotency_key: IdempotencyKey::new("idem-stop-cmd").expect("key"),
        },
    );
    let resume_handle = {
        let ws = app.world().resource::<ActiveWorkspaceView>();
        let ex = &ws.open_workspace.as_ref().expect("open").exchanges[0];
        assert!(
            !ex.kept_partial_text.is_empty(),
            "partial text must be kept"
        );
        match &ex.answer {
            AnswerState::Stopped {
                resume: Some(handle),
            } => handle.clone(),
            other => panic!("expected Stopped with resume handle, got {other:?}"),
        }
    };

    // Clear cancel trigger and click Resume (`work.resume` with matching `expected_run_id`).
    fx.backend
        .lock()
        .expect("lock")
        .stub_provider
        .cancel_after_chunks = None;
    dispatch(
        &mut app,
        ClinicAction::ResumeOrRetryWork {
            idempotency_key: IdempotencyKey::new("idem-resume-1").expect("key"),
            work_item_id: resume_handle.work_item_id.clone(),
            expected_run_id: resume_handle.expected_run_id.clone(),
        },
    );

    let cancel_2 = CancellationToken::new();
    let (_, resumed_out) = fx
        .backend
        .lock()
        .expect("lock")
        .run_next_worker_attempt(
            &fx.practice_a,
            &fx.physician_a,
            "worker-1",
            &cancel_2,
            1_020,
        )
        .expect("ok")
        .expect("claimed");
    assert!(matches!(resumed_out, TurnExecutionOutcome::Finished { .. }));
    dispatch(&mut app, ClinicAction::PollStream);
    assert!(matches!(
        app.world()
            .resource::<ActiveWorkspaceView>()
            .open_workspace
            .as_ref()
            .expect("open")
            .exchanges[0]
            .answer,
        AnswerState::Finished { .. }
    ));

    // Replaying stale work.resume with old expected_run_id returns 409 Conflict.
    let mut transport = LocalVerticalSliceTransport::new(Arc::clone(&fx.backend));
    let stale_err = transport
        .submit_command(
            &session,
            &CommandEnvelope {
                idempotency_key: IdempotencyKey::new("idem-stale-resume").expect("key"),
                command: CommandPayload::WorkResume(ef_clinic_domain::chat::ResumeWorkPayload {
                    patient_workspace_id: fx.ws_a1.clone(),
                    chat_id: fx.chat_main.clone(),
                    work_item_id: work_id,
                    expected_run_id: resume_handle.expected_run_id,
                }),
            },
            1_025,
        )
        .expect_err("stale resume must fail with 409");
    assert_eq!(stale_err.http_status_code(), 409);
}

#[test]
fn scn_m2_0008_bounded_5_failure_retry_exhaustion_and_try_again_recovery() {
    let fx = build_fixture(SignInMode::LocalProof);
    fx.backend
        .lock()
        .expect("lock")
        .stub_provider
        .remaining_failures = 5;

    let hub = SharedBrowserSessionHub::new(None);
    let locale_store = SharedLocaleStorage::default();
    let mut app = spawn_client_app(
        "tab-retry",
        "/elysianfields/",
        Arc::clone(&fx.backend),
        hub,
        locale_store,
    );
    sign_in_local_proof(&mut app, "clinician@practice-a.invalid");
    dispatch(
        &mut app,
        ClinicAction::OpenPatientWorkspace {
            workspace_id: fx.ws_a1.clone(),
            chat_id: None,
        },
    );
    dispatch(
        &mut app,
        ClinicAction::ComposerEdit(ComposerAction::InsertText(
            "Evaluate electrolytes.".to_owned(),
        )),
    );
    dispatch(
        &mut app,
        ClinicAction::SubmitMessage {
            idempotency_key: IdempotencyKey::new("idem-fail-5").expect("key"),
            delivery: None,
        },
    );

    // Exhaust 5 automatic attempts -> permanent CouldntFinish with explicit Try again handle.
    let cancel = CancellationToken::new();
    for attempt in 1..=5u32 {
        let (_, outcome) = fx
            .backend
            .lock()
            .expect("lock")
            .run_next_worker_attempt(
                &fx.practice_a,
                &fx.physician_a,
                "worker-1",
                &cancel,
                1_000 + u64::from(attempt),
            )
            .expect("ok")
            .expect("claimed");
        if attempt < 5 {
            assert_eq!(
                outcome,
                TurnExecutionOutcome::ScheduledRetry {
                    failed_attempts: attempt
                }
            );
        } else {
            assert_eq!(
                outcome,
                TurnExecutionOutcome::ExhaustedCouldntFinish { failed_attempts: 5 }
            );
        }
    }
    // Zero background retry churn after 5th failure.
    assert!(
        fx.backend
            .lock()
            .expect("lock")
            .run_next_worker_attempt(&fx.practice_a, &fx.physician_a, "worker-1", &cancel, 1_010)
            .expect("ok")
            .is_none()
    );

    dispatch(&mut app, ClinicAction::PollStream);
    let try_again = {
        let ws = app.world().resource::<ActiveWorkspaceView>();
        let ex = &ws.open_workspace.as_ref().expect("open").exchanges[0];
        match &ex.answer {
            AnswerState::CouldntFinish {
                try_again: Some(handle),
            } => handle.clone(),
            other => panic!("expected CouldntFinish with Try again handle, got {other:?}"),
        }
    };
    assert!(
        app.world()
            .resource::<A11yProjection>()
            .nodes
            .iter()
            .any(|n| n.key == A11ySemanticKey::TryAgainButton),
        "Try again button must be exposed in accessibility tree"
    );

    // Activate Try again -> attempt 6 executes and settles to Finished.
    dispatch(
        &mut app,
        ClinicAction::ResumeOrRetryWork {
            idempotency_key: IdempotencyKey::new("idem-try-again-6").expect("key"),
            work_item_id: try_again.work_item_id,
            expected_run_id: try_again.expected_run_id,
        },
    );
    let (_, sixth) = fx
        .backend
        .lock()
        .expect("lock")
        .run_next_worker_attempt(&fx.practice_a, &fx.physician_a, "worker-1", &cancel, 1_020)
        .expect("ok")
        .expect("claimed attempt 6");
    assert!(matches!(sixth, TurnExecutionOutcome::Finished { .. }));
    dispatch(&mut app, ClinicAction::PollStream);
    assert!(matches!(
        app.world()
            .resource::<ActiveWorkspaceView>()
            .open_workspace
            .as_ref()
            .expect("open")
            .exchanges[0]
            .answer,
        AnswerState::Finished { .. }
    ));
}

#[test]
fn scn_m2_0009_reload_bus_resync_draft_409_conflict_and_durable_readback() {
    let fx = build_fixture(SignInMode::LocalProof);
    let hub = SharedBrowserSessionHub::new(None);
    let locale_store = SharedLocaleStorage::default();

    let mut app = spawn_client_app(
        "tab-durable",
        "/elysianfields/",
        Arc::clone(&fx.backend),
        hub.clone(),
        locale_store.clone(),
    );
    sign_in_local_proof(&mut app, "clinician@practice-a.invalid");
    let session = hub.shared_storage().expect("session");
    dispatch(
        &mut app,
        ClinicAction::OpenPatientWorkspace {
            workspace_id: fx.ws_a1.clone(),
            chat_id: None,
        },
    );

    // Submit and finish one durable clinical turn.
    dispatch(
        &mut app,
        ClinicAction::ComposerEdit(ComposerAction::InsertText(
            "Initial durable question.".to_owned(),
        )),
    );
    dispatch(
        &mut app,
        ClinicAction::SubmitMessage {
            idempotency_key: IdempotencyKey::new("idem-durable-1").expect("key"),
            delivery: None,
        },
    );
    let cancel = CancellationToken::new();
    let _ = fx
        .backend
        .lock()
        .expect("lock")
        .run_next_worker_attempt(&fx.practice_a, &fx.physician_a, "worker-1", &cancel, 1_005)
        .expect("worker ok");

    // Emit lossy bus ResyncRequired -> client refetches authoritative snapshot cleanly.
    fx.backend
        .lock()
        .expect("lock")
        .chat
        .emit_resync("valkey broker restarted");
    dispatch(&mut app, ClinicAction::PollStream);
    assert_eq!(
        app.world().resource::<ActiveWorkspaceView>().resync_count,
        1
    );
    assert!(matches!(
        app.world()
            .resource::<ActiveWorkspaceView>()
            .open_workspace
            .as_ref()
            .expect("open")
            .exchanges[0]
            .answer,
        AnswerState::Finished { .. }
    ));

    // Draft autosave (rev 1), then simulate second tab writing rev 2 -> 409 Conflict!
    dispatch(
        &mut app,
        ClinicAction::ComposerEdit(ComposerAction::InsertText("Local draft v1".to_owned())),
    );
    dispatch(&mut app, ClinicAction::AutosaveDraft);
    assert_eq!(
        app.world().resource::<ActiveWorkspaceView>().draft_banner,
        DraftBannerState::Saved { revision: 1 }
    );

    let mut second_tab_transport = LocalVerticalSliceTransport::new(Arc::clone(&fx.backend));
    let _ = second_tab_transport
        .put_draft(
            &session,
            &fx.ws_a1,
            fx.chat_main.as_str(),
            &PutDraftRequest {
                text: "Other tab newer draft v2".to_owned(),
                expected_revision: 1,
            },
            1_010,
        )
        .expect("second tab saved rev 2");

    dispatch(
        &mut app,
        ClinicAction::ComposerEdit(ComposerAction::InsertText(" + more local".to_owned())),
    );
    dispatch(&mut app, ClinicAction::AutosaveDraft);
    assert!(matches!(
        app.world().resource::<ActiveWorkspaceView>().draft_banner,
        DraftBannerState::Conflict { ref server_draft } if server_draft.revision == 2
    ));
    assert_eq!(
        app.world().resource::<ComposerSurface>().text(),
        "Local draft v1 + more local",
        "409 conflict must NEVER silently overwrite local typing"
    );

    // Choose KeepMine -> saves local text as rev 3.
    dispatch(
        &mut app,
        ClinicAction::ResolveDraftConflict(DraftConflictChoice::KeepMine),
    );
    assert_eq!(
        app.world().resource::<ActiveWorkspaceView>().draft_banner,
        DraftBannerState::Saved { revision: 3 }
    );

    // Trigger another conflict (rev 4 from second tab) and choose UseNewer.
    let _ = second_tab_transport
        .put_draft(
            &session,
            &fx.ws_a1,
            fx.chat_main.as_str(),
            &PutDraftRequest {
                text: "Authoritative newer text rev 4".to_owned(),
                expected_revision: 3,
            },
            1_012,
        )
        .expect("saved rev 4");
    dispatch(
        &mut app,
        ClinicAction::ComposerEdit(ComposerAction::InsertText("!".to_owned())),
    );
    dispatch(&mut app, ClinicAction::AutosaveDraft);
    dispatch(
        &mut app,
        ClinicAction::ResolveDraftConflict(DraftConflictChoice::UseNewer),
    );
    assert_eq!(
        app.world().resource::<ComposerSurface>().text(),
        "Authoritative newer text rev 4"
    );

    // Simulated full browser reload: drop old Bevy App, boot fresh App against same backend!
    drop(app);
    let mut reloaded = spawn_client_app(
        "tab-reloaded",
        "/elysianfields/?patient=pw-flagship-1",
        Arc::clone(&fx.backend),
        hub,
        locale_store,
    );
    dispatch(&mut reloaded, ClinicAction::BootTab);

    let open = reloaded
        .world()
        .resource::<ActiveWorkspaceView>()
        .open_workspace
        .as_ref()
        .expect("reloaded patient workspace");
    assert_eq!(open.exchanges.len(), 1);
    assert!(matches!(
        open.exchanges[0].answer,
        AnswerState::Finished { .. }
    ));
    assert_eq!(
        reloaded.world().resource::<ComposerSurface>().text(),
        "Authoritative newer text rev 4"
    );
}

#[test]
fn scn_m2_0010_subpath_mounting_loopback_safety_cache_headers_and_strict_csp() {
    // Bare /elysianfields redirects 308 preserving query string.
    let bare = ClientBootConfig::from_location("/elysianfields?patient=pw-flagship-1")
        .expect("valid bare mount");
    assert_eq!(
        bare.canonical_redirect.as_deref(),
        Some("/elysianfields/?patient=pw-flagship-1")
    );

    // Non-loopback ?api= override is ignored; API stays strictly under /elysianfields/api/...
    let attacker = ClientBootConfig::from_location(
        "/elysianfields/?patient=pw-flagship-1&api=https://attacker.example",
    )
    .expect("valid mount");
    assert!(attacker.loopback_api_override.is_none());
    assert_eq!(
        attacker
            .api_url(&ClinicEndpoint::Commands)
            .expect("resolved"),
        "/elysianfields/api/commands"
    );

    // Loopback ?api= override is accepted safely for local testing.
    let local_dev = ClientBootConfig::from_location("/elysianfields/?api=http://127.0.0.1:8080")
        .expect("valid local dev");
    assert_eq!(
        local_dev
            .api_url(&ClinicEndpoint::AuthWhoAmI)
            .expect("resolved"),
        "http://127.0.0.1:8080/elysianfields/api/auth/whoami"
    );

    // Strict CSP passes and excludes unsafe-inline / unsafe-eval.
    assert!(is_strict_wasm_csp(STRICT_WASM_CSP));
    assert!(!is_strict_wasm_csp("script-src 'self' 'unsafe-inline'"));

    // All clinical/auth transport calls target /elysianfields/api/... with no-store.
    let fx = build_fixture(SignInMode::LocalProof);
    let hub = SharedBrowserSessionHub::new(None);
    let locale_store = SharedLocaleStorage::default();
    let mut app = spawn_client_app(
        "tab-subpath",
        "/elysianfields/",
        Arc::clone(&fx.backend),
        hub,
        locale_store,
    );
    sign_in_local_proof(&mut app, "clinician@practice-a.invalid");
    let guard = fx.backend.lock().expect("lock");
    assert!(!guard.observed_requests.is_empty());
    for req in &guard.observed_requests {
        assert!(
            req.endpoint_url.starts_with("/elysianfields/api/"),
            "endpoint {} must mount under /elysianfields/api/",
            req.endpoint_url
        );
        assert_eq!(req.cache_policy, CachePolicy::ClinicalNoStore);
    }
}

#[test]
fn scn_m2_0011_keyboard_only_navigation_privacy_cover_and_zero_duplicate_a11y() {
    let fx = build_fixture(SignInMode::LocalProof);
    let hub = SharedBrowserSessionHub::new(None);
    let locale_store = SharedLocaleStorage::default();
    let mut app = spawn_client_app(
        "tab-a11y",
        "/elysianfields/",
        Arc::clone(&fx.backend),
        hub,
        locale_store,
    );
    sign_in_local_proof(&mut app, "clinician@practice-a.invalid");
    dispatch(
        &mut app,
        ClinicAction::OpenPatientWorkspace {
            workspace_id: fx.ws_a1.clone(),
            chat_id: None,
        },
    );

    // Focus composer: `/` and `j` must NOT trigger workspace shortcuts while editing text.
    app.world_mut()
        .resource_mut::<ShortcutRouter>()
        .set_focus(FocusId(A11ySemanticKey::ComposerInput));
    dispatch(
        &mut app,
        ClinicAction::DispatchShortcut(ShortcutInput::new(
            ShortcutKey::Char('/'),
            ModifierChord::None,
        )),
    );
    assert!(!app.world().resource::<TodayRosterView>().finder_open);

    // Global Cmd/Ctrl+Shift+L activates Privacy Cover EVEN while editing text!
    dispatch(
        &mut app,
        ClinicAction::DispatchShortcut(ShortcutInput::new(
            ShortcutKey::Char('l'),
            ModifierChord::PrimaryShift,
        )),
    );
    assert!(
        app.world()
            .resource::<ShortcutRouter>()
            .is_privacy_cover_active()
    );
    {
        let projection = app.world().resource::<A11yProjection>();
        assert!(
            projection
                .nodes
                .iter()
                .any(|n| n.key == A11ySemanticKey::PrivacyCoverDialog)
        );
        assert!(
            !projection
                .nodes
                .iter()
                .any(|n| n.key == A11ySemanticKey::ComposerInput),
            "clinical composer must be hidden behind opaque privacy cover"
        );
        let audit = projection
            .verify_zero_duplicate_nodes()
            .expect("zero duplicates under privacy cover");
        assert_eq!(audit.duplicated_nodes, 0);
        assert_eq!(audit.mirrored_css_rules, 0);
    }

    // Second Cmd/Ctrl+Shift+L reveals workspace and restores exact focus to ComposerInput!
    dispatch(
        &mut app,
        ClinicAction::DispatchShortcut(ShortcutInput::new(
            ShortcutKey::Char('L'),
            ModifierChord::PrimaryShift,
        )),
    );
    assert!(
        !app.world()
            .resource::<ShortcutRouter>()
            .is_privacy_cover_active()
    );
    assert_eq!(
        app.world().resource::<ShortcutRouter>().current_focus(),
        &FocusId(A11ySemanticKey::ComposerInput)
    );

    // Blur composer -> `/` opens Patient Finder -> Escape dismisses & restores focus -> j/k/Enter opens patient.
    app.world_mut()
        .resource_mut::<ShortcutRouter>()
        .set_focus(FocusId(A11ySemanticKey::ListRow(0)));
    dispatch(
        &mut app,
        ClinicAction::DispatchShortcut(ShortcutInput::new(
            ShortcutKey::Char('/'),
            ModifierChord::None,
        )),
    );
    assert!(app.world().resource::<TodayRosterView>().finder_open);
    dispatch(
        &mut app,
        ClinicAction::DispatchShortcut(ShortcutInput::new(
            ShortcutKey::Escape,
            ModifierChord::None,
        )),
    );
    assert!(!app.world().resource::<TodayRosterView>().finder_open);
    assert_eq!(
        app.world().resource::<ShortcutRouter>().current_focus(),
        &FocusId(A11ySemanticKey::ListRow(0))
    );

    // Keyboard j / k / Enter navigation across roster.
    dispatch(
        &mut app,
        ClinicAction::DispatchShortcut(ShortcutInput::new(
            ShortcutKey::Char('j'),
            ModifierChord::None,
        )),
    );
    dispatch(
        &mut app,
        ClinicAction::DispatchShortcut(ShortcutInput::new(
            ShortcutKey::Char('k'),
            ModifierChord::None,
        )),
    );
    dispatch(
        &mut app,
        ClinicAction::DispatchShortcut(ShortcutInput::new(ShortcutKey::Enter, ModifierChord::None)),
    );

    // Open citation chip via action, then Escape dismisses panel and restores focus to CitationChip(1).
    dispatch(
        &mut app,
        ClinicAction::OpenCitationChip(CitationChipRef {
            source_key: fx.source_note_1.clone(),
            number: 1,
            label: "Progress Note [1]".to_owned(),
        }),
    );
    dispatch(
        &mut app,
        ClinicAction::DispatchShortcut(ShortcutInput::new(
            ShortcutKey::Escape,
            ModifierChord::None,
        )),
    );
    assert!(matches!(
        app.world()
            .resource::<ActiveWorkspaceView>()
            .citation_panel
            .state(),
        CitationPanelState::Idle
    ));
    assert_eq!(
        app.world().resource::<ShortcutRouter>().current_focus(),
        &FocusId(A11ySemanticKey::CitationChip(1))
    );

    // Final single-owner AccessKit + ARIA bridge audit across the live workspace tree.
    assert!(
        app.world()
            .resource::<CatalogA11ySyncStatus>()
            .errors
            .is_empty()
    );
    let audit = app
        .world()
        .resource::<A11yProjection>()
        .verify_zero_duplicate_nodes()
        .expect("single-owner zero-duplicate a11y audit");
    assert_eq!(audit.duplicated_nodes, 0);
    assert_eq!(audit.mirrored_css_rules, 0);
}

#[test]
fn scn_m2_0012_nine_locale_switching_arabic_rtl_mirroring_and_verbatim_source_language() {
    let fx = build_fixture(SignInMode::LocalProof);
    let hub = SharedBrowserSessionHub::new(None);
    let locale_store = SharedLocaleStorage::default();
    let mut app = spawn_client_app(
        "tab-i18n",
        "/elysianfields/",
        Arc::clone(&fx.backend),
        hub.clone(),
        locale_store.clone(),
    );
    sign_in_local_proof(&mut app, "clinician@practice-a.invalid");
    dispatch(
        &mut app,
        ClinicAction::OpenPatientWorkspace {
            workspace_id: fx.ws_a1.clone(),
            chat_id: None,
        },
    );

    // Cycle through all 9 shipped locales verifying exhaustive 56-key translations.
    for locale in UiLocaleKind::ALL {
        dispatch(&mut app, ClinicAction::SwitchLocale(locale));
        assert_eq!(app.world().resource::<UiLocale>().current, locale);
        for key in UiKey::ALL {
            assert!(!translate(locale, key).trim().is_empty());
        }
    }

    // Arabic RTL mirroring + BiDi mixed lab values + ARIA document direction update.
    dispatch(&mut app, ClinicAction::SwitchLocale(UiLocaleKind::Ar));
    assert_eq!(
        app.world().resource::<ClientRuntimeConfig>().sidebar_edge,
        HorizontalEdge::Right
    );
    assert_eq!(
        app.world()
            .resource::<ClientRuntimeConfig>()
            .citation_panel_edge,
        HorizontalEdge::Left
    );
    assert_eq!(
        app.world().resource::<ComposerSurface>().direction(),
        TextDirection::Rtl
    );
    dispatch(
        &mut app,
        ClinicAction::ComposerEdit(ComposerAction::InsertText(
            "الكرياتينين 54 mL/min/1.73m²".to_owned(),
        )),
    );
    assert_eq!(
        app.world().resource::<ComposerSurface>().text(),
        "الكرياتينين 54 mL/min/1.73m²"
    );
    let patch_json = app
        .world()
        .resource::<AriaBridgeEmitter>()
        .serialize_last_patches()
        .expect("json");
    assert!(!patch_json.is_empty());

    // Open English cited clinical record while UI locale is Arabic -> verbatim English text + Arabic provenance notice!
    dispatch(
        &mut app,
        ClinicAction::OpenCitationChip(CitationChipRef {
            source_key: fx.source_note_1.clone(),
            number: 1,
            label: "Progress Note [1]".to_owned(),
        }),
    );
    {
        let ws = app.world().resource::<ActiveWorkspaceView>();
        let CitationPanelState::Loaded { source, .. } = ws.citation_panel.state() else {
            panic!("expected loaded citation [1]");
        };
        assert_eq!(
            source.quote, "Creatinine 1.02 mg/dL; eGFR 54 mL/min/1.73m².",
            "verbatim clinical text must remain in its original language"
        );
        assert_eq!(
            ws.original_language_notice,
            OriginalLanguageNotice::Visible {
                source_locale: CoreLocale::En,
                ui_locale: UiLocaleKind::Ar,
                localized_text: translate(UiLocaleKind::Ar, UiKey::ClinicOriginalLanguageNote),
            }
        );
    }

    // Reload against same SharedLocaleStorage: curie.locale.v1 persists Arabic across reload!
    drop(app);
    let reloaded = spawn_client_app(
        "tab-i18n-reloaded",
        "/elysianfields/",
        Arc::clone(&fx.backend),
        hub.clone(),
        locale_store.clone(),
    );
    assert_eq!(
        reloaded.world().resource::<UiLocale>().current,
        UiLocaleKind::Ar
    );

    // ?lang= query parameter overrides persisted preference; malformed ?lang= falls back cleanly.
    let query_override = spawn_client_app(
        "tab-i18n-ja",
        "/elysianfields/?lang=ja-JP",
        Arc::clone(&fx.backend),
        hub.clone(),
        locale_store.clone(),
    );
    assert_eq!(
        query_override.world().resource::<UiLocale>().current,
        UiLocaleKind::Ja
    );

    let invalid_lang = spawn_client_app(
        "tab-i18n-bad",
        "/elysianfields/?lang=xx-INVALID",
        Arc::clone(&fx.backend),
        hub,
        locale_store,
    );
    assert_eq!(
        invalid_lang.world().resource::<UiLocale>().current,
        UiLocaleKind::Ar
    );
}
