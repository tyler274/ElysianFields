//! Comprehensive domain scenario tests covering `SCN-M2-0001`..`SCN-M2-0009`
//! and ALL 11 formal sign-in invariants.

use ef_clinic_domain::auth::{
    ABSOLUTE_CAP_SECS, ACTIVITY_QUANTUM_SECS, AccountStanding, AuthAuthority, DeepLinkTarget,
    EmailAddress, HOSTED_ANTI_ENUMERATION_MESSAGE, IDLE_LIMIT_SECS, MailTransportStatus,
    MultiTabBrowser, PhysicianAccount, PhysicianRole, SessionActivityDecision, SessionRecord,
    SignInInitiationResponse, SignInMode, TabViewState, VERIFICATION_TOKEN_TTL_SECS,
    VerificationTokenKind, WorkspaceOpenView,
};
use ef_clinic_domain::chat::{
    AnswerState, ChatCommandError, ChatEngine, ChatStreamEvent, CommandEnvelope, CommandPayload,
    ComposerDraftController, DeliveryMode, DraftSaveOutcome, DraftStore, DraftSyncState,
    ExchangeStep, ExchangeSteps, MAX_FAILURE_ATTEMPTS, ResumeWorkPayload, RetryBackoffMode,
    StepExecutionState, StopWorkPayload, SubmitMessagePayload, WORKER_LEASE_SECS,
    WORKER_STALL_RECLAIM_SECS, compute_retry_backoff_secs,
};
use ef_clinic_domain::workspace::{
    CitationChipRef, CitationFetchOutcome, CitationOrigin, CitationPanelController,
    CitationPanelState, CitedSourcePayload, MembershipTier, PatientRosterState, ReadablePart,
    RecordStatusClass, RosterAvailability, WorkspaceAccessError, WorkspaceEvidenceStore,
    classify_record_status, synthetic_5000_row_lab_source,
};
use ef_core::{
    AttemptId, ChatId, IdempotencyKey, Locale, PhysicianId, PracticeId, RefusalCode, ResponseId,
    RunId, SessionId, SourceKey, TabId, WorkspaceId,
};

fn active_account() -> PhysicianAccount {
    PhysicianAccount {
        physician_id: PhysicianId::new("phys-a1").expect("phys id"),
        practice_id: PracticeId::new("prac-a").expect("prac id"),
        email: EmailAddress::parse("Dr.Active@Example.ORG").expect("email"),
        display_name: "Dr. Active Clinician".to_owned(),
        role: PhysicianRole::Standard,
        standing: AccountStanding::Active,
        sessions_ended_before: None,
    }
}

fn deactivated_account() -> PhysicianAccount {
    PhysicianAccount {
        physician_id: PhysicianId::new("phys-a2").expect("phys id"),
        practice_id: PracticeId::new("prac-a").expect("prac id"),
        email: EmailAddress::parse("deactivated@example.org").expect("email"),
        display_name: "Dr. Former".to_owned(),
        role: PhysicianRole::Standard,
        standing: AccountStanding::Deactivated,
        sessions_ended_before: None,
    }
}

#[test]
fn scn_m2_0001_invite_only_anti_enumeration_single_use_verify_and_session_lifetime() {
    let mut authority = AuthAuthority::new(SignInMode::Hosted);
    authority.upsert_account(active_account());
    authority.upsert_account(deactivated_account());

    let t0 = 1_700_000_000u64;
    let unknown_res = authority
        .initiate_sign_in("unknown@example.org", VerificationTokenKind::SignInLink, t0)
        .expect("unknown returns anti-enumeration 200");
    let deactivated_res = authority
        .initiate_sign_in(
            "deactivated@example.org",
            VerificationTokenKind::SignInLink,
            t0,
        )
        .expect("deactivated returns anti-enumeration 200");
    let active_res = authority
        .initiate_sign_in(
            "  DR.ACTIVE@EXAMPLE.ORG ",
            VerificationTokenKind::SignInLink,
            t0,
        )
        .expect("active returns anti-enumeration 200");

    // Identical hosted anti-enumeration response across all three accounts.
    let expected = SignInInitiationResponse::Unavailable {
        token: None,
        message: HOSTED_ANTI_ENUMERATION_MESSAGE.to_owned(),
    };
    assert_eq!(unknown_res, expected);
    assert_eq!(deactivated_res, expected);
    assert_eq!(active_res, expected);

    // Outbound mail sent strictly for active account only.
    assert_eq!(authority.sent_mail().len(), 1);
    let mail = &authority.sent_mail()[0];
    assert_eq!(mail.recipient.as_str(), "dr.active@example.org");
    assert_eq!(mail.expires_at, t0 + VERIFICATION_TOKEN_TTL_SECS);
    let one_time_token = mail.token.clone();

    // Mail delivery outage returns 502.
    authority.set_mail_status(MailTransportStatus::DeliveryFailed);
    let mail_err = authority
        .initiate_sign_in(
            "dr.active@example.org",
            VerificationTokenKind::SignInLink,
            t0 + 1,
        )
        .expect_err("mail failure must return 502");
    assert_eq!(mail_err.http_status_code(), 502);
    authority.set_mail_status(MailTransportStatus::Healthy);

    // First verify succeeds; replay fails with 401 session_ended.
    let verified = authority
        .verify_token("dr.active@example.org", &one_time_token, t0 + 10, None)
        .expect("first verify must succeed");
    let replay_err = authority
        .verify_token("dr.active@example.org", &one_time_token, t0 + 11, None)
        .expect_err("replayed token must fail");
    assert_eq!(replay_err.http_status_code(), 401);
    assert_eq!(replay_err.refusal_code(), Some(RefusalCode::SessionEnded));

    // Deactivated account returns 403 access_removed.
    let deact_err = authority
        .verify_token("deactivated@example.org", "any-token", t0 + 12, None)
        .expect_err("deactivated verify must return 403");
    assert_eq!(deact_err.http_status_code(), 403);
    assert_eq!(deact_err.refusal_code(), Some(RefusalCode::AccessRemoved));

    // Session lifetime & sliding renewal rules: 15m quantum, 12h idle, 7d cap, strict inequality.
    let mut sess = SessionRecord::mint(
        verified.session.clone(),
        verified.physician_id.clone(),
        verified.practice_id.clone(),
        t0,
        None,
    );
    assert_eq!(
        sess.evaluate_activity(t0 + ACTIVITY_QUANTUM_SECS - 1),
        SessionActivityDecision::Covered {
            idle_deadline: t0 + IDLE_LIMIT_SECS
        }
    );
    assert_eq!(
        sess.evaluate_activity(t0 + ACTIVITY_QUANTUM_SECS),
        SessionActivityDecision::Extended {
            new_idle_deadline: t0 + ACTIVITY_QUANTUM_SECS + IDLE_LIMIT_SECS
        }
    );
    // Strict inequality: at now == idle_deadline it is Ended.
    let exact_idle = t0 + ACTIVITY_QUANTUM_SECS + IDLE_LIMIT_SECS;
    assert_eq!(
        sess.evaluate_activity(exact_idle),
        SessionActivityDecision::Ended
    );

    // Fixed-end session clamps strictly to min(ends_at, created_at + 7d).
    let fixed_end = t0 + 3_600;
    let mut bounded = SessionRecord::mint(
        SessionId::new("sess-bounded").expect("id"),
        verified.physician_id,
        verified.practice_id,
        t0,
        Some(fixed_end),
    );
    assert_eq!(bounded.absolute_end, fixed_end);
    assert_eq!(
        bounded.evaluate_activity(t0 + 1_800),
        SessionActivityDecision::Covered {
            idle_deadline: fixed_end
        }
    );
    assert_eq!(
        bounded.evaluate_activity(fixed_end),
        SessionActivityDecision::Ended
    );

    // 7d absolute cap clamps over-long deadlines.
    let mut long_sess = SessionRecord::mint(
        SessionId::new("sess-long").expect("id"),
        PhysicianId::new("phys-a1").expect("id"),
        PracticeId::new("prac-a").expect("id"),
        t0,
        Some(t0 + ABSOLUTE_CAP_SECS + 10_000),
    );
    assert_eq!(long_sess.absolute_end, t0 + ABSOLUTE_CAP_SECS);
    assert_eq!(
        long_sess.evaluate_activity(t0 + ABSOLUTE_CAP_SECS),
        SessionActivityDecision::Ended
    );

    // Missing bearer token -> no_session.
    let no_sess = authority
        .whoami(None, t0 + 20)
        .expect_err("missing bearer must return no_session");
    assert_eq!(no_sess.refusal_code(), Some(RefusalCode::NoSession));
}

#[test]
fn scn_m2_0002_and_0003_all_11_sign_in_invariants_multi_tab_and_deep_link() {
    let mut authority = AuthAuthority::new(SignInMode::LocalProof);
    authority.upsert_account(active_account());
    let t0 = 1_700_000_000u64;

    let tab_a = TabId::new("tab-a").expect("tab a");
    let tab_b = TabId::new("tab-b").expect("tab b");
    let patient_ws = WorkspaceId::new("pw-flagship-1").expect("ws");

    // Mint Session 1.
    let SignInInitiationResponse::TokenIssued { token: tok1, .. } = authority
        .initiate_sign_in(
            "dr.active@example.org",
            VerificationTokenKind::SignInLink,
            t0,
        )
        .expect("tok1")
    else {
        panic!("expected local proof token");
    };
    let s1 = authority
        .verify_token("dr.active@example.org", &tok1, t0 + 1, None)
        .expect("s1")
        .session;

    // SCN-M2-0003: Unauthenticated deep-link boot -> Lands invariant.
    let mut browser = MultiTabBrowser::new(None);
    browser.open_tab(
        tab_a.clone(),
        DeepLinkTarget::LinkedPatient(patient_ws.clone()),
    );
    browser.open_tab(tab_b.clone(), DeepLinkTarget::Today);
    browser.resolve_opening(&tab_a, &mut authority, t0 + 2);
    browser.resolve_opening(&tab_b, &mut authority, t0 + 2);
    assert_eq!(
        browser.tab_state(&tab_a),
        Some(&TabViewState::SignInPage {
            wants: DeepLinkTarget::LinkedPatient(patient_ws.clone()),
        })
    );

    // Tab B signs in with s1; Tab A catches up and lands on deep-linked patient (Lands + CatchesUp).
    browser.complete_sign_in(&tab_b, s1.clone());
    browser.deliver_all_peer_notices(&mut authority, t0 + 3);
    assert_eq!(
        browser.tab_state(&tab_a),
        Some(&TabViewState::Workspace {
            holds: s1.clone(),
            opens: WorkspaceOpenView::LinkedPatient(patient_ws.clone()),
            sse_connected: true,
        })
    );
    browser
        .check_safety_invariants(&authority, t0 + 3)
        .expect("all invariants hold after Lands");

    // SCN-M2-0002 part 1: Expiry -> Tab A transitions immediately to SessionEndedPrompt with 0 patient content (Expiry & PromptIsEmpty).
    authority.expire_session(&s1, t0 + 10);
    browser.handle_session_ended_response(&tab_a, &s1, &mut authority, t0 + 10);
    assert_eq!(
        browser.tab_state(&tab_a),
        Some(&TabViewState::SessionEndedPrompt)
    );
    assert!(
        !browser
            .tab_state(&tab_a)
            .expect("tab a")
            .shows_protected_content()
    );

    // SCN-M2-0002 part 2: KeepsWork + TodayFirst!
    // Tab B signs in freshly (minting Session 2) while Tab A receives a late Session 1 401.
    let SignInInitiationResponse::TokenIssued { token: tok2, .. } = authority
        .initiate_sign_in(
            "dr.active@example.org",
            VerificationTokenKind::SignInLink,
            t0 + 15,
        )
        .expect("tok2")
    else {
        panic!("expected local proof token");
    };
    let s2 = authority
        .verify_token("dr.active@example.org", &tok2, t0 + 16, None)
        .expect("s2")
        .session;
    browser.begin_email_sign_in_link(&tab_b);
    browser.complete_sign_in(&tab_b, s2.clone());
    // Stale s1 401 arrives in Tab A BEFORE peer notice — Tab A adopts s2 on Today (KeepsWork & TodayFirst)!
    browser.handle_session_ended_response(&tab_a, &s1, &mut authority, t0 + 17);
    assert_eq!(
        browser.tab_state(&tab_a),
        Some(&TabViewState::Workspace {
            holds: s2.clone(),
            opens: WorkspaceOpenView::Today,
            sse_connected: true,
        })
    );
    browser.deliver_all_peer_notices(&mut authority, t0 + 17);
    browser
        .check_safety_invariants(&authority, t0 + 17)
        .expect("KeepsWork + TodayFirst + SignedIn hold");
    assert!(
        browser
            .attempt_workspace_action(&tab_a, &authority, t0 + 17)
            .is_ok()
    );

    // SCN-M2-0002 part 3: Sign out in Tab A -> Tab B clears immediately (OnceOut, ServerRefusesSignedOut, CatchesUp).
    browser.sign_out(&tab_a, &mut authority, t0 + 20);
    // Fallback sign-out from Tab B while storage is already empty still revokes any held token.
    browser.deliver_all_peer_notices(&mut authority, t0 + 20);
    assert!(!authority.is_session_live(&s2, t0 + 20));
    assert!(!authority.revocation_broadcasts().is_empty());
    assert!(
        !browser
            .tab_state(&tab_b)
            .expect("tab b")
            .shows_protected_content()
    );
    assert!(
        browser
            .attempt_workspace_action(&tab_b, &authority, t0 + 20)
            .is_err()
    );
    browser
        .check_safety_invariants(&authority, t0 + 20)
        .expect("OnceOut + ServerRefusesSignedOut + NoDeadlock hold");
}

#[test]
fn scn_m2_0004_and_0005_three_tier_access_citations_monotonic_fencing_and_redaction() {
    let prac_a = PracticeId::new("prac-a").expect("prac a");
    let prac_b = PracticeId::new("prac-b").expect("prac b");
    let phys_1 = PhysicianId::new("phys-1").expect("phys 1");
    let phys_2 = PhysicianId::new("phys-2").expect("phys 2");
    let ws_a1 = WorkspaceId::new("pw-a1").expect("ws a1");
    let ws_a2 = WorkspaceId::new("pw-a2").expect("ws a2");
    let ws_b1 = WorkspaceId::new("pw-b1").expect("ws b1");

    let mut store = WorkspaceEvidenceStore::new();
    store.register_workspace(
        ws_a1.clone(),
        prac_a.clone(),
        "Synthetic Patient A1",
        vec![phys_1.clone()],
    );
    store.register_workspace(
        ws_a2.clone(),
        prac_a.clone(),
        "Synthetic Patient A2",
        vec![phys_2.clone()],
    );
    store.register_workspace(
        ws_b1.clone(),
        prac_b.clone(),
        "Synthetic Patient B1",
        vec![phys_2],
    );

    // 3-tier access checks.
    assert_eq!(
        store.membership_for(&prac_a, &phys_1, &ws_a1),
        Ok(MembershipTier::Allowed)
    );
    assert_eq!(
        store.membership_for(&prac_a, &phys_1, &ws_a2),
        Ok(MembershipTier::ClinicDemographic)
    );
    assert_eq!(
        store.membership_for(&prac_a, &phys_1, &ws_b1),
        Ok(MembershipTier::Denied)
    );

    // Cross-practice access returns 404 non-disclosure; ClinicDemographic refuses clinical disclosure.
    assert_eq!(
        store.require_clinical_access(&prac_a, &phys_1, &ws_b1),
        Err(WorkspaceAccessError::NotFound)
    );
    assert_eq!(
        store.require_clinical_access(&prac_a, &phys_1, &ws_a2),
        Err(WorkspaceAccessError::ClinicalDisclosureForbidden)
    );

    // Patient roster lists ONLY Practice A entries and reports Incomplete honestly.
    let roster = store.query_patient_roster(&prac_a, &phys_1, None, true);
    match roster {
        PatientRosterState::Loaded {
            patients,
            availability,
        } => {
            assert_eq!(patients.len(), 2);
            assert!(patients.iter().all(|p| p.practice_id == prac_a));
            assert_eq!(
                availability,
                RosterAvailability::Incomplete {
                    may_be_missing_someone: true
                }
            );
        }
        other => panic!("expected Loaded roster, got {other:?}"),
    }

    // Store outage returns Failed (never Empty!).
    store.set_store_outage(Some("read replica unavailable".to_owned()));
    assert!(matches!(
        store.query_patient_roster(&prac_a, &phys_1, None, false),
        PatientRosterState::Failed { .. }
    ));
    store.set_store_outage(None);

    // Insert note [1] (flagged preliminary) + 5,000-row lab table [12] + patient B source.
    let note_key = SourceKey::new("src-note-1").expect("key");
    let lab_key = SourceKey::new("src-lab-12").expect("key");
    let other_key = SourceKey::new("src-b1-note").expect("key");

    assert_eq!(
        classify_record_status("entered_in_error"),
        RecordStatusClass::Flagged
    );
    assert_eq!(
        classify_record_status("PRELIMINARY"),
        RecordStatusClass::Flagged
    );
    assert_eq!(classify_record_status("final"), RecordStatusClass::Settled);

    store.insert_source(CitedSourcePayload {
        source_key: note_key.clone(),
        workspace_id: ws_a1.clone(),
        number: 1,
        source_label: "Progress Note [1]".to_owned(),
        title: "Synthetic Progress Note".to_owned(),
        record_type: "progress_note".to_owned(),
        recorded_at: "2026-01-12T09:00:00Z".to_owned(),
        record_status: "preliminary".to_owned(),
        status_class: classify_record_status("preliminary"),
        origin: CitationOrigin::SourceRecord,
        original_language: Locale::En,
        quote: "Renal clearance stable above dosing threshold.".to_owned(),
        body: "Synthetic verbatim clinical note.".to_owned(),
        readable_parts: vec![
            ReadablePart {
                label: "Page 1".to_owned(),
                text: "Renal clearance stable above dosing threshold.".to_owned(),
                unreadable_notice: None,
            },
            ReadablePart {
                label: "Page 2 (Attachment)".to_owned(),
                text: String::new(),
                unreadable_notice: Some("Attachment page 2 could not be decoded.".to_owned()),
            },
        ],
        fields: Vec::new(),
        raw_content: None,
        external_url: None,
    });
    store.insert_source(synthetic_5000_row_lab_source(
        ws_a1.clone(),
        lab_key.clone(),
    ));
    store.insert_source(synthetic_5000_row_lab_source(
        ws_b1.clone(),
        other_key.clone(),
    ));

    // Cross-patient source lookup returns 404 Not Found.
    assert_eq!(
        store.fetch_cited_source(&prac_a, &phys_1, &ws_a1, &other_key),
        Err(WorkspaceAccessError::NotFound)
    );

    // Monotonic request_id fencing in CitationPanelController discards stale responses.
    let mut panel = CitationPanelController::new();
    let req1 = panel.open_chip(CitationChipRef {
        source_key: note_key.clone(),
        number: 1,
        label: "Note [1]".to_owned(),
    });
    let req2 = panel.open_chip(CitationChipRef {
        source_key: lab_key.clone(),
        number: 12,
        label: "Labs [12]".to_owned(),
    });
    assert!(req2 > req1);

    let note_payload = store
        .fetch_cited_source(&prac_a, &phys_1, &ws_a1, &note_key)
        .expect("note fetch");
    let lab_payload = store
        .fetch_cited_source(&prac_a, &phys_1, &ws_a1, &lab_key)
        .expect("lab fetch");
    assert_eq!(lab_payload.fields.len(), 5_000);

    // Late response for req1 is discarded cleanly!
    assert!(!panel.apply_response(req1, CitationFetchOutcome::Loaded(Box::new(note_payload))));
    assert!(panel.apply_response(req2, CitationFetchOutcome::Loaded(Box::new(lab_payload))));
    assert!(matches!(
        panel.state(),
        CitationPanelState::Loaded { source, .. } if source.fields.len() == 5_000
    ));

    // Withdraw source [1] server-side -> returns SourceWithdrawn and panel clears clinical text immediately.
    assert!(store.withdraw_source(&ws_a1, &note_key));
    assert_eq!(
        store.fetch_cited_source(&prac_a, &phys_1, &ws_a1, &note_key),
        Err(WorkspaceAccessError::SourceWithdrawn)
    );
    let req3 = panel.open_chip(CitationChipRef {
        source_key: note_key,
        number: 1,
        label: "Note [1]".to_owned(),
    });
    assert!(panel.apply_response(
        req3,
        CitationFetchOutcome::Unavailable {
            eyebrow: "Source withdrawn".to_owned(),
            title: "Progress Note [1]".to_owned(),
            reason: "This record is no longer available.".to_owned(),
        }
    ));
    assert!(matches!(
        panel.state(),
        CitationPanelState::Unavailable { .. }
    ));
}

#[test]
fn scn_m2_0006_to_0009_chat_wire_contract_idempotency_stop_resume_5_failure_retry_and_draft_conflict()
 {
    let ws = WorkspaceId::new("pw-flagship-1").expect("ws");
    let chat = ChatId::new("chat-1").expect("chat");

    // Verify exact JSON schema wire round-trip for CommandEnvelope and AnswerState.
    let submit_cmd = CommandEnvelope {
        idempotency_key: IdempotencyKey::new("idem-1").expect("idem"),
        command: CommandPayload::MessageSubmit(SubmitMessagePayload {
            patient_workspace_id: ws.clone(),
            chat_id: chat.clone(),
            text: "Summarize renal function trends.".to_owned(),
            physician_time_zone: "UTC".to_owned(),
            delivery: Some(DeliveryMode::SendNow),
        }),
    };
    let wire_json = serde_json::to_string(&submit_cmd).expect("serialize cmd");
    assert!(wire_json.contains("\"type\":\"message.submit\""));
    assert!(wire_json.contains("\"idempotency_key\":\"idem-1\""));
    let parsed_cmd: CommandEnvelope = serde_json::from_str(&wire_json).expect("deserialize cmd");
    assert_eq!(parsed_cmd, submit_cmd);

    // Verify hosted exponential backoff schedule: 10s, 20s, 40s, 80s, 160s, ..., capped at 600s.
    assert_eq!(compute_retry_backoff_secs(1, RetryBackoffMode::Hosted), 10);
    assert_eq!(compute_retry_backoff_secs(2, RetryBackoffMode::Hosted), 20);
    assert_eq!(compute_retry_backoff_secs(3, RetryBackoffMode::Hosted), 40);
    assert_eq!(compute_retry_backoff_secs(4, RetryBackoffMode::Hosted), 80);
    assert_eq!(compute_retry_backoff_secs(7, RetryBackoffMode::Hosted), 600);
    assert_eq!(
        compute_retry_backoff_secs(1, RetryBackoffMode::DeterministicTest),
        0
    );

    let mut engine = ChatEngine::new(RetryBackoffMode::Hosted);
    let t0 = 1_000u64;

    // Submit & verify idempotent replay creates ZERO duplicate turns.
    let acc1 = engine
        .execute_command(submit_cmd.clone(), t0)
        .expect("submit accepted");
    assert!(!acc1.deduplicated);
    let acc1_replay = engine
        .execute_command(submit_cmd, t0 + 1)
        .expect("replay accepted");
    assert!(acc1_replay.deduplicated);
    assert_eq!(acc1_replay.work_item_id, acc1.work_item_id);
    assert_eq!(engine.transcript_snapshot().len(), 1);

    // Worker claims 60s lease; stalls -> another worker reclaims after 60s lease expiry / 300s stall.
    let (claimed_id, _) = engine.claim_due_work("worker-1", t0).expect("claim 1");
    assert_eq!(claimed_id, acc1.work_item_id);
    assert!(engine.claim_due_work("worker-2", t0 + 10).is_none());
    let (reclaimed_id, _) = engine
        .claim_due_work("worker-2", t0 + WORKER_LEASE_SECS)
        .expect("reclaim after 60s lease");
    assert_eq!(reclaimed_id, acc1.work_item_id);
    let (stall_reclaimed, _) = engine
        .claim_due_work(
            "worker-3",
            t0 + WORKER_LEASE_SECS + WORKER_STALL_RECLAIM_SECS,
        )
        .expect("reclaim after stall");
    assert_eq!(stall_reclaimed, acc1.work_item_id);

    // Record tool step + incremental delta chunk + queue follow-up -> Stop -> Resume -> Stale Resume 409.
    engine
        .record_step(
            &acc1.work_item_id,
            ExchangeStep::SearchedChart {
                query: "creatinine".to_owned(),
                group: "labs".to_owned(),
                page: 1,
                state: StepExecutionState::Completed,
            },
            t0 + 400,
        )
        .expect("step");
    engine
        .append_delta(&acc1.work_item_id, "Partial clinical synthesis ", t0 + 401)
        .expect("delta");

    // Queue a SendNext message while Working.
    engine
        .execute_command(
            CommandEnvelope {
                idempotency_key: IdempotencyKey::new("idem-q1").expect("k"),
                command: CommandPayload::MessageSubmit(SubmitMessagePayload {
                    patient_workspace_id: ws.clone(),
                    chat_id: chat.clone(),
                    text: "Follow-up question".to_owned(),
                    physician_time_zone: "UTC".to_owned(),
                    delivery: Some(DeliveryMode::SendNext),
                }),
            },
            t0 + 402,
        )
        .expect("queued");

    // Stop returns queued follow-up and transitions to Stopped with expected_run_id.
    let stop_acc = engine
        .execute_command(
            CommandEnvelope {
                idempotency_key: IdempotencyKey::new("idem-stop-1").expect("k"),
                command: CommandPayload::WorkStop(StopWorkPayload {
                    patient_workspace_id: ws.clone(),
                    chat_id: chat.clone(),
                    work_item_id: acc1.work_item_id.clone(),
                }),
            },
            t0 + 403,
        )
        .expect("stop");
    assert_eq!(stop_acc.returned_messages, vec!["Follow-up question"]);

    let expected_run_id = match &engine.exchange(&acc1.work_item_id).expect("ex").answer {
        AnswerState::Stopped { resume: Some(h) } => h.expected_run_id.clone(),
        other => panic!("expected Stopped with resume handle, got {other:?}"),
    };

    // Resume succeeds with matching expected_run_id; stale replay returns 409 Conflict!
    engine
        .execute_command(
            CommandEnvelope {
                idempotency_key: IdempotencyKey::new("idem-resume-1").expect("k"),
                command: CommandPayload::WorkResume(ResumeWorkPayload {
                    patient_workspace_id: ws.clone(),
                    chat_id: chat.clone(),
                    work_item_id: acc1.work_item_id.clone(),
                    expected_run_id: expected_run_id.clone(),
                }),
            },
            t0 + 404,
        )
        .expect("resume");

    let stale_err = engine
        .execute_command(
            CommandEnvelope {
                idempotency_key: IdempotencyKey::new("idem-resume-stale").expect("k"),
                command: CommandPayload::WorkResume(ResumeWorkPayload {
                    patient_workspace_id: ws.clone(),
                    chat_id: chat.clone(),
                    work_item_id: acc1.work_item_id.clone(),
                    expected_run_id,
                }),
            },
            t0 + 405,
        )
        .expect_err("stale resume must fail 409");
    assert_eq!(stale_err.http_status_code(), 409);

    // Exhaust exactly 5 failures -> CouldntFinish with explicit Try again.
    for attempt in 1..=MAX_FAILURE_ATTEMPTS {
        let count = engine
            .record_attempt_failure(&acc1.work_item_id, t0 + 500 + u64::from(attempt) * 100)
            .expect("record failure");
        assert_eq!(count, attempt);
    }
    let try_again_run_id = match &engine.exchange(&acc1.work_item_id).expect("ex").answer {
        AnswerState::CouldntFinish { try_again: Some(h) } => h.expected_run_id.clone(),
        other => panic!("expected CouldntFinish after 5 failures, got {other:?}"),
    };
    assert!(engine.claim_due_work("worker-4", t0 + 10_000).is_none());

    // Explicit Try again executes attempt 6 and settles to Finished!
    engine
        .execute_command(
            CommandEnvelope {
                idempotency_key: IdempotencyKey::new("idem-try-again").expect("k"),
                command: CommandPayload::WorkResume(ResumeWorkPayload {
                    patient_workspace_id: ws.clone(),
                    chat_id: chat.clone(),
                    work_item_id: acc1.work_item_id.clone(),
                    expected_run_id: try_again_run_id,
                }),
            },
            t0 + 2_000,
        )
        .expect("try again");
    engine
        .complete_exchange(
            &acc1.work_item_id,
            ResponseId::new("resp-1").expect("resp"),
            AttemptId::new("att-6").expect("att"),
            "2026-01-15T15:00:00Z".to_owned(),
            Vec::new(),
        )
        .expect("complete");
    assert!(matches!(
        engine.exchange(&acc1.work_item_id).expect("ex").answer,
        AnswerState::Finished { .. }
    ));
    assert!(matches!(
        engine.exchange(&acc1.work_item_id).expect("ex").steps,
        ExchangeSteps::Read { ref steps } if steps.len() == 1
    ));

    // Bus restart emits ResyncRequired.
    engine.emit_resync("valkey restarted");
    assert!(
        engine
            .events()
            .iter()
            .any(|e| matches!(e, ChatStreamEvent::ResyncRequired { .. }))
    );

    // Stopping an already Finished turn returns 409 Conflict.
    let stop_finished = engine
        .execute_command(
            CommandEnvelope {
                idempotency_key: IdempotencyKey::new("idem-stop-finished").expect("k"),
                command: CommandPayload::WorkStop(StopWorkPayload {
                    patient_workspace_id: ws.clone(),
                    chat_id: chat,
                    work_item_id: acc1.work_item_id,
                }),
            },
            t0 + 2_005,
        )
        .expect_err("stop on finished must return 409");
    assert_eq!(
        stop_finished,
        ChatCommandError::Conflict {
            message: "no matching active turn to stop".to_owned()
        }
    );

    // SCN-M2-0009: Optimistic-concurrency composer draft autosave & 409 Conflict resolution.
    let mut draft_store = DraftStore::new();
    let mut tab1 = ComposerDraftController::from_persisted(draft_store.get(&ws, "chat-1"));
    assert!(!tab1.is_dirty());
    tab1.edit("Draft from tab 1");
    assert!(tab1.is_dirty());

    // Tab 2 saves concurrently on server, bumping revision 0 -> 1.
    assert!(matches!(
        draft_store.put(&ws, "chat-1", "Newer draft from tab 2".to_owned(), 0),
        DraftSaveOutcome::Saved(ref d) if d.revision == 1
    ));

    // Tab 1 attempts autosave with expected_revision = 0 -> 409 Conflict pauses autosave.
    let (txt, exp_rev, gen1) = tab1.begin_autosave().expect("autosave starts");
    let outcome = draft_store.put(&ws, "chat-1", txt, exp_rev);
    assert!(matches!(outcome, DraftSaveOutcome::Conflict { .. }));
    tab1.finish_autosave(gen1, outcome);
    assert!(matches!(tab1.sync_state(), DraftSyncState::Conflict { .. }));
    assert!(tab1.begin_autosave().is_none());

    // Physician selects Retry (keep mine) -> saves over server revision 1 -> revision 2!
    let (keep_txt, keep_rev, keep_gen) = tab1.resolve_keep_mine().expect("keep mine");
    assert_eq!(keep_rev, 1);
    let saved = draft_store.put(&ws, "chat-1", keep_txt, keep_rev);
    tab1.finish_autosave(keep_gen, saved);
    assert!(!tab1.is_dirty());
    assert_eq!(tab1.saved_revision(), 2);

    // Typing after pressing Send preserves the post-Send suffix when clearing submitted prefix.
    tab1.edit("Submitted question");
    tab1.append_typing(" plus follow-up typed during submit");
    tab1.clear_submitted_prefix("Submitted question");
    assert_eq!(tab1.local_text(), " plus follow-up typed during submit");

    // Verify RunId deserialization rejects blank strings.
    assert!(serde_json::from_str::<RunId>("\"   \"").is_err());
}
