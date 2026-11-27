//! Integration tests proving `SCN-M3-0001` through `SCN-M3-0006` across all
//! M3 Increment 1 Curie Clinic workflow modules.

use ef_clinic_workflows::ehr_channels::{
    AssistedOnboardingVendor, ClinicChannelAdapter, ClinicChannelKind, ClinicalRecordBucket,
    ConfidentialityTag, IngestionError, KipuApiAuthMode,
    PART2_STATUTORY_REDISCLOSURE_NOTICE_2_32_A, SmartFhirVendor, SmartScopeVersion,
    build_smart_authorize_request, decode_hl7_escapes, ingest_ccda_r21_xml,
    ingest_fhir_r4_resource, ingest_hl7_v2_message, ingest_kipu_record, is_same_origin_bundle_next,
    pkce_s256_challenge, sha256_hex, validate_shlink_payload,
};
use ef_clinic_workflows::matching::{
    AutomatedMatchSignal, BacklogConsentState, ConversationMatchState, DEFAULT_UNDO_WINDOW_SECS,
    MatchingError, MessageMatchingEngine, MoveDestination, RawChannelFrame,
    UNDECIDED_FALLBACK_SECS,
};
use ef_clinic_workflows::memory::{
    MemoryAuthority, MemoryEntryState, PatientMemoryStore, RoundsObservation, RoundsRunOutcome,
    verify_model_facing_text_hygiene,
};
use ef_clinic_workflows::replies::{
    MessageReplyEngine, PracticeReplyKind, ReplyError, ReviewCompletion, ReviewExecutionState,
    VisibleReplyOrigin, sanitize_patient_facing_reply,
};
use ef_clinic_workflows::today::{
    ChannelHealthStatus, ScheduledVisitEntry, TodayChannelBanner, TodayFailedPart, TodayItemKind,
    TodayQueueEngine, TodayQueueItem, TodayUrgency, TodayViewState,
};
use ef_core::{PhysicianId, SourceKey, WorkspaceId};

#[test]
fn scn_m3_0001_today_queue_urgency_incomplete_honesty_and_channel_banners() {
    let dr_1 = PhysicianId::new("phys-1").expect("phys");
    let ws_a = WorkspaceId::new("pw-a").expect("ws");
    let ws_b = WorkspaceId::new("pw-b").expect("ws");

    let mut today = TodayQueueEngine::new();
    today.set_channel_health("spruce-main", ChannelHealthStatus::Connected);

    let items = vec![
        TodayQueueItem {
            item_id: "item-today-1".to_owned(),
            workspace_id: Some(ws_b.clone()),
            urgency: TodayUrgency::Today,
            waiting_since_epoch_secs: 100,
            kind: TodayItemKind::SameDayClinicalConcern,
            version_id: 1,
            summary: "Same-day electrolyte review".to_owned(),
            can_mark_done: true,
        },
        TodayQueueItem {
            item_id: "item-now-1".to_owned(),
            workspace_id: Some(ws_a.clone()),
            urgency: TodayUrgency::Now,
            waiting_since_epoch_secs: 200,
            kind: TodayItemKind::UrgentClinicalConcern,
            version_id: 1,
            summary: "Urgent potassium alert".to_owned(),
            can_mark_done: true,
        },
    ];
    let visits = vec![ScheduledVisitEntry {
        workspace_id: ws_a,
        display_name: "Synthetic Patient A".to_owned(),
        scheduled_at_epoch_secs: 500,
        briefing_ready: true,
    }];

    // Urgent ('now') surfaces ahead of 'today'.
    let view = today.build_view(&dr_1, &items, visits.clone(), 4, Vec::new());
    let TodayViewState::Actionable {
        items: rendered,
        may_be_missing_someone,
        channel_banners,
        ..
    } = view
    else {
        panic!("expected actionable Today view");
    };
    assert_eq!(rendered[0].item.item_id, "item-now-1");
    assert_eq!(rendered[1].item.item_id, "item-today-1");
    assert!(!may_be_missing_someone);
    assert!(channel_banners.is_empty());

    // Mark both done at version 1 -> quiet day!
    assert!(today.mark_done(&dr_1, &items[0]));
    assert!(today.mark_done(&dr_1, &items[1]));
    let quiet = today.build_view(&dr_1, &items, visits.clone(), 6, Vec::new());
    assert!(matches!(
        quiet,
        TodayViewState::Quiet {
            also_reviewed: 6,
            ..
        }
    ));

    // Newer clinical version (version_id = 2) resurfaces item honestly!
    let mut updated_items = items.clone();
    updated_items[1].version_id = 2;
    let resurfaced = today.build_view(&dr_1, &updated_items, visits.clone(), 6, Vec::new());
    assert!(matches!(
        resurfaced,
        TodayViewState::Actionable { ref items, .. } if items.len() == 1 && items[0].item.item_id == "item-now-1"
    ));

    // Temp channel error -> trouble notice WITHOUT asking to reconnect (TodaySaysSo).
    today.set_channel_health(
        "spruce-main",
        ChannelHealthStatus::TempError {
            detail: "HTTP 503 upstream timeout".to_owned(),
        },
    );
    let temp_view = today.build_view(&dr_1, &[], visits.clone(), 6, Vec::new());
    let TodayViewState::Quiet {
        channel_banners, ..
    } = temp_view
    else {
        panic!("expected quiet view with channel trouble banner");
    };
    assert_eq!(
        channel_banners,
        vec![TodayChannelBanner::TemporaryTrouble {
            channel_id: "spruce-main".to_owned(),
            detail: "HTTP 503 upstream timeout".to_owned(),
            asks_to_reconnect: false,
        }]
    );

    // Revoked credentials -> explicit reconnect prompt.
    today.set_channel_health(
        "spruce-main",
        ChannelHealthStatus::Revoked {
            detail: "OAuth refresh token revoked".to_owned(),
        },
    );
    let revoked_view = today.build_view(&dr_1, &[], visits.clone(), 6, Vec::new());
    let TodayViewState::Quiet {
        channel_banners: rev_banners,
        ..
    } = revoked_view
    else {
        panic!("expected quiet view with reconnect banner");
    };
    assert!(matches!(
        rev_banners.as_slice(),
        [TodayChannelBanner::ReconnectRequired { .. }]
    ));

    // Partial producer failure sets may_be_missing_someone=true and NEVER shows Quiet!
    let partial = today.build_view(
        &dr_1,
        &[],
        visits,
        6,
        vec![TodayFailedPart {
            producer: "lab-feed".to_owned(),
            reason: "upstream timeout".to_owned(),
        }],
    );
    assert!(matches!(
        partial,
        TodayViewState::Actionable {
            may_be_missing_someone: true,
            ref failed_parts,
            ..
        } if failed_parts.len() == 1
    ));
}

#[test]
fn scn_m3_0002_and_0003_all_14_message_matching_invariants() {
    let d1 = PhysicianId::new("dr-1").expect("d1");
    let d2 = PhysicianId::new("dr-2").expect("d2");
    let d_both = PhysicianId::new("dr-both").expect("d_both");
    let pt_a = WorkspaceId::new("pw-a").expect("a");
    let pt_b = WorkspaceId::new("pw-b").expect("b");

    let has_access = |doc: &PhysicianId, pt: &WorkspaceId| -> bool {
        if doc == &d_both {
            return true;
        }
        (doc == &d1 && pt == &pt_a) || (doc == &d2 && pt == &pt_b)
    };

    let mut engine = MessageMatchingEngine::new();
    engine.register_channel("spruce-1", ChannelHealthStatus::Connected);

    // ExactlyOnce cursor rejection when skipping seq 1 -> 2.
    assert_eq!(
        engine.ingest_channel_frame(
            "spruce-1",
            RawChannelFrame::Readable {
                cursor_seq: 2,
                conversation_id: "c-skip".to_owned(),
                message_id: "m-skip".to_owned(),
                body: "skipped".to_owned(),
            },
            100,
        ),
        Err(MatchingError::CursorSequenceMismatch {
            expected: 1,
            actual: 2,
        })
    );

    // 1. Unambiguous match auto-links to Patient A (LinksOnItsOwn).
    engine
        .ingest_channel_frame(
            "spruce-1",
            RawChannelFrame::Readable {
                cursor_seq: 1,
                conversation_id: "c-1".to_owned(),
                message_id: "m-1".to_owned(),
                body: "Hi Dr, refill request".to_owned(),
            },
            100,
        )
        .expect("seq 1");
    engine
        .apply_automated_match(
            "c-1",
            AutomatedMatchSignal::UnambiguousSingle(pt_a.clone()),
            101,
        )
        .expect("auto-link");
    assert_eq!(
        engine
            .linked_conversations_for_patient(&d1, &pt_a, has_access)
            .len(),
        1
    );
    // OnlyWithAccess: d2 lacks access to Patient A.
    assert!(
        engine
            .linked_conversations_for_patient(&d2, &pt_a, has_access)
            .is_empty()
    );

    // 2. Ambiguous match routes to to_match; SuggestionsWithAccess strips Patient B for d1.
    engine
        .ingest_channel_frame(
            "spruce-1",
            RawChannelFrame::Readable {
                cursor_seq: 2,
                conversation_id: "c-ambig".to_owned(),
                message_id: "m-2".to_owned(),
                body: "Ambiguous sender".to_owned(),
            },
            102,
        )
        .expect("seq 2");
    engine
        .apply_automated_match(
            "c-ambig",
            AutomatedMatchSignal::AmbiguousOrContradicted {
                suggested: Some(pt_b.clone()),
            },
            103,
        )
        .expect("to_match");
    let d1_to_match = engine.visible_to_match_for(&d1, has_access);
    assert_eq!(d1_to_match[0].suggested_patient, None);
    let d2_to_match = engine.visible_to_match_for(&d2, has_access);
    assert_eq!(d2_to_match[0].suggested_patient, Some(pt_b.clone()));

    // 3. 15-minute undecided fallback (NeverInvisible).
    engine
        .ingest_channel_frame(
            "spruce-1",
            RawChannelFrame::Readable {
                cursor_seq: 3,
                conversation_id: "c-stall".to_owned(),
                message_id: "m-3".to_owned(),
                body: "Stalled matcher".to_owned(),
            },
            105,
        )
        .expect("seq 3");
    assert_eq!(
        engine.run_undecided_fallback(105 + UNDECIDED_FALLBACK_SECS - 1),
        0
    );
    assert_eq!(
        engine.run_undecided_fallback(105 + UNDECIDED_FALLBACK_SECS),
        1
    );
    assert!(matches!(
        engine.conversation("c-stall").expect("c-stall").state,
        ConversationMatchState::ToMatch { .. }
    ));

    // 4. Move A -> B with Undo; ingest new message during Undo; press Undo -> restores ALL messages to A with 0 deferred updates!
    engine
        .open_conversation_view(&d1, "c-1", has_access)
        .expect("d1 opens c-1");
    let prior_commits = engine.committed_record_updates().len();

    // OnlyBetweenTheirPatients: d1 cannot move to Patient B.
    assert!(matches!(
        engine.move_conversation(
            &d1,
            "c-1",
            MoveDestination::Patient(pt_b.clone()),
            has_access,
            2_000,
        ),
        Err(MatchingError::UnauthorizedPatient { .. })
    ));

    // d_both moves c-1 from A -> B; d1's open view is closed immediately!
    engine
        .move_conversation(
            &d_both,
            "c-1",
            MoveDestination::Patient(pt_b.clone()),
            has_access,
            2_000,
        )
        .expect("move A->B");
    assert!(
        !engine
            .conversation("c-1")
            .expect("c-1")
            .open_viewers
            .contains(&d1)
    );
    // ActsAfterUndo: zero deferred updates committed while Undo is active.
    assert_eq!(engine.committed_record_updates().len(), prior_commits);

    // Message arrives during Undo window.
    engine
        .ingest_channel_frame(
            "spruce-1",
            RawChannelFrame::Readable {
                cursor_seq: 4,
                conversation_id: "c-1".to_owned(),
                message_id: "m-during-undo".to_owned(),
                body: "Second message during Undo".to_owned(),
            },
            2_005,
        )
        .expect("seq 4");
    engine.undo_move("c-1").expect("undo restores origin");
    let restored = engine.conversation("c-1").expect("c-1");
    assert_eq!(
        restored.state,
        ConversationMatchState::Linked {
            patient: pt_a.clone()
        }
    );
    assert_eq!(restored.messages.len(), 2);
    assert_eq!(engine.committed_record_updates().len(), prior_commits);

    // 5. Move A -> B again and let Undo expire -> deferred record update executes & c-1 disappears from A (RightPatient, ActsAfterUndo).
    engine
        .move_conversation(
            &d_both,
            "c-1",
            MoveDestination::Patient(pt_b.clone()),
            has_access,
            3_000,
        )
        .expect("move A->B 2");
    assert_eq!(
        engine.expire_undo_windows(3_000 + DEFAULT_UNDO_WINDOW_SECS),
        1
    );
    assert_eq!(engine.committed_record_updates().len(), prior_commits + 1);
    assert!(
        engine
            .linked_conversations_for_patient(&d_both, &pt_a, has_access)
            .is_empty()
    );
    assert_eq!(
        engine
            .linked_conversations_for_patient(&d_both, &pt_b, has_access)
            .len(),
        1
    );

    // 6. Unreadable message (CantReadShows), disconnect (ClearedOnDisconnect), reconnect with backlog consent (BacklogOnlyWhenAccepted).
    engine
        .ingest_channel_frame(
            "spruce-1",
            RawChannelFrame::Unreadable {
                cursor_seq: 5,
                reason: "corrupted MIME payload".to_owned(),
            },
            3_100,
        )
        .expect("seq 5 unreadable");
    assert_eq!(engine.visible_unreadable_entries().len(), 1);

    engine.disconnect_channel("spruce-1");
    assert!(engine.visible_unreadable_entries().is_empty());
    assert!(engine.conversation("c-ambig").is_none());
    assert!(engine.conversation("c-stall").is_none());
    // Linked conversation c-1 survives disconnect!
    assert!(engine.conversation("c-1").is_some());

    // Reconnect offers backlog -> NEVER ingested until explicit consent accepted.
    engine.reconnect_channel_offering_backlog(
        "spruce-1",
        vec![RawChannelFrame::Readable {
            cursor_seq: 6,
            conversation_id: "c-backlog".to_owned(),
            message_id: "m-backlog".to_owned(),
            body: "Backlog message".to_owned(),
        }],
    );
    assert_eq!(
        engine.backlog_consent_state("spruce-1"),
        Some(&BacklogConsentState::Offered { pending_count: 1 })
    );
    assert!(engine.conversation("c-backlog").is_none());

    let ingested = engine
        .accept_backlog_consent("spruce-1", 3_200)
        .expect("accept backlog");
    assert_eq!(ingested, 1);
    assert!(engine.conversation("c-backlog").is_some());
}

#[test]
fn scn_m3_0004_all_12_message_reply_invariants() {
    let d1 = PhysicianId::new("dr-1").expect("d1");
    let d2 = PhysicianId::new("dr-2").expect("d2");
    let d_no_access = PhysicianId::new("dr-other").expect("d3");
    let pt_a = WorkspaceId::new("pw-a").expect("a");
    let pt_b = WorkspaceId::new("pw-b").expect("b");

    let has_access = |doc: &PhysicianId, _pt: &WorkspaceId| -> bool { doc == &d1 || doc == &d2 };

    let mut engine = MessageReplyEngine::new();
    engine.register_conversation("conv-1", pt_a.clone(), true);
    engine
        .receive_patient_message(
            "conv-1",
            "pm-1",
            "Can I take ibuprofen with my blood pressure med?",
        )
        .expect("receive pm-1");

    // Auto-responder practice message NEVER marks patient message answered (OnlyTheReviewAnswers + HistoryTrue).
    engine
        .record_practice_sent(
            "conv-1",
            "auto-1",
            PracticeReplyKind::AutoResponder,
            "Our office received your message and will reply within 1 business day.",
            500,
        )
        .expect("record auto-responder");
    assert_eq!(engine.sent_history("conv-1").len(), 1);

    let (rev_id, for_pt) = engine.start_review("conv-1").expect("start review");
    // OneReviewEach: second concurrent review fails.
    assert!(matches!(
        engine.start_review("conv-1"),
        Err(ReplyError::ReviewAlreadyRunning { .. })
    ));

    // Reject internal citation markup in patient-facing reply.
    let markup_err = engine.complete_review(
        "conv-1",
        ReviewCompletion {
            review_id: rev_id,
            for_patient: for_pt.clone(),
            proposed_reply_text: Some("Please hold ibuprofen [1].".to_owned()),
            basis_citations: vec![SourceKey::new("src-meds-1").expect("src")],
            flagged_concern: Some("NSAID + ACE inhibitor renal risk".to_owned()),
            answered_by_human_reply: true,
        },
    );
    assert_eq!(markup_err, Err(ReplyError::InternalMarkupForbidden));

    let clean_reply =
        sanitize_patient_facing_reply("Please avoid ibuprofen [1] and use acetaminophen instead.");
    assert_eq!(
        clean_reply,
        "Please avoid ibuprofen and use acetaminophen instead."
    );

    // Even if answered_by_human_reply=true is passed, AutoResponder alone cannot answer the patient message!
    let applied = engine
        .complete_review(
            "conv-1",
            ReviewCompletion {
                review_id: rev_id,
                for_patient: for_pt,
                proposed_reply_text: Some(clean_reply.clone()),
                basis_citations: vec![SourceKey::new("src-meds-1").expect("src")],
                flagged_concern: Some("NSAID + ACE inhibitor renal risk".to_owned()),
                answered_by_human_reply: true,
            },
        )
        .expect("complete review");
    assert!(applied);

    // OnlyWithAccess rejects unauthorized physician.
    assert!(matches!(
        engine.derive_reply_card(&d_no_access, "conv-1", has_access),
        Err(ReplyError::Unauthorized { .. })
    ));

    // YourDraft + CopiesWhatYouSee: D1 edits personal draft; D2 still sees system reply & copies exact visible text.
    engine
        .edit_personal_draft(
            &d1,
            "conv-1",
            "Personal note from Dr. 1: please avoid ibuprofen and call if dizzy.".to_owned(),
            has_access,
        )
        .expect("d1 edits draft");
    let d1_card = engine
        .derive_reply_card(&d1, "conv-1", has_access)
        .expect("d1 card");
    let d2_card = engine
        .derive_reply_card(&d2, "conv-1", has_access)
        .expect("d2 card");
    assert_eq!(d1_card.open_messages.len(), 1);
    assert_eq!(
        d1_card.visible_reply_origin,
        VisibleReplyOrigin::PersonalDraft
    );
    assert_eq!(
        d2_card.visible_reply_origin,
        VisibleReplyOrigin::SystemReply
    );
    assert_eq!(
        engine
            .copy_reply(&d2, "conv-1", has_access)
            .expect("d2 copy"),
        clean_reply
    );
    assert_eq!(
        engine
            .copy_reply(&d1, "conv-1", has_access)
            .expect("d1 copy"),
        "Personal note from Dr. 1: please avoid ibuprofen and call if dizzy."
    );

    // Restore system reply discards only D1's personal draft.
    engine
        .restore_system_reply(&d1, "conv-1", has_access)
        .expect("restore");
    assert_eq!(
        engine
            .copy_reply(&d1, "conv-1", has_access)
            .expect("copy after restore"),
        clean_reply
    );

    // NoteWhileDisconnected: notice appears iff open reply AND channel disconnected.
    assert!(!d1_card.show_channel_disconnected_notice);
    engine.set_channel_connected("conv-1", false);
    let disconnected_card = engine
        .derive_reply_card(&d1, "conv-1", has_access)
        .expect("card");
    assert!(disconnected_card.show_channel_disconnected_notice);

    // ConcernStays + UndoBringsBack: closing reply preserves flagged concern; Undo restores closed reply & open messages.
    engine
        .close_reply(&d1, "conv-1", has_access)
        .expect("close reply");
    let closed_card = engine
        .derive_reply_card(&d1, "conv-1", has_access)
        .expect("closed card");
    assert!(closed_card.open_messages.is_empty());
    assert!(closed_card.flagged_concern.is_some());
    assert!(closed_card.undo_available);
    engine
        .undo_close_reply(&d1, "conv-1", has_access)
        .expect("undo close");
    let undone_card = engine
        .derive_reply_card(&d1, "conv-1", has_access)
        .expect("undone card");
    assert_eq!(undone_card.open_messages.len(), 1);
    assert_eq!(
        undone_card.visible_reply_text.as_deref(),
        Some(clean_reply.as_str())
    );

    // WithheldAfterMove: start review on Patient A, move to Patient B -> immediately withheld & stale A review discarded!
    let (stale_rev, stale_pt) = engine.start_review("conv-1").expect("start stale review");
    engine
        .on_conversation_moved("conv-1", MoveDestination::Patient(pt_b))
        .expect("move to B");
    let stale_applied = engine
        .complete_review(
            "conv-1",
            ReviewCompletion {
                review_id: stale_rev,
                for_patient: stale_pt,
                proposed_reply_text: Some("Stale reply for patient A".to_owned()),
                basis_citations: Vec::new(),
                flagged_concern: None,
                answered_by_human_reply: false,
            },
        )
        .expect("stale discarded cleanly");
    assert!(!stale_applied);
    let moved_card = engine
        .derive_reply_card(&d1, "conv-1", has_access)
        .expect("moved card");
    assert_eq!(moved_card.reply_state_label, "withheld");
    assert!(moved_card.show_channel_disconnected_notice);

    // Worker restart returns Running review to Woken cleanly.
    let (_r2, _p2) = engine.start_review("conv-1").expect("start review on B");
    assert_eq!(engine.recover_interrupted_reviews_on_restart(), 1);
    assert_eq!(
        engine
            .derive_reply_card(&d1, "conv-1", has_access)
            .expect("card")
            .review_state,
        ReviewExecutionState::Woken
    );

    // Moving to NotPatient clears both reply and flagged concern (ConcernStays).
    engine
        .on_conversation_moved("conv-1", MoveDestination::NotPatient)
        .expect("move to not_patient");
    assert_eq!(engine.flagged_concern("conv-1"), None);
}

#[test]
fn scn_m3_0005_patient_memory_rounds_watermark_quiet_noop_disagreement_and_retirement() {
    let ws = WorkspaceId::new("pw-mem-1").expect("ws");
    let mut store = PatientMemoryStore::new(ws);

    let pref_id = store.record_entry(
        "anticoagulation_preference",
        "Physician prefers apixaban 5mg BID over warfarin due to lab adherence.",
        "2026-01-10",
        MemoryAuthority::PhysicianInstruction,
    );
    let old_condition_id = store.record_entry(
        "acute_bronchitis",
        "Acute bronchitis managed conservatively.",
        "2025-11-01",
        MemoryAuthority::ChartRecord,
    );
    let mutations_before_noop = store.mutation_count();

    // 1. Quiet no-op run with zero new observations makes ZERO edits.
    assert_eq!(
        store.run_rounds(&[], false),
        RoundsRunOutcome::QuietNoOp { watermark: 0 }
    );
    assert_eq!(store.mutation_count(), mutations_before_noop);

    // 2. Aborted run mid-execution does NOT advance rounds watermark!
    let conflicting_chart = vec![RoundsObservation {
        record_seq: 10,
        topic: "anticoagulation_preference".to_owned(),
        statement: "External discharge summary lists warfarin 5mg daily.".to_owned(),
        observed_at: "2026-01-18".to_owned(),
        authority: MemoryAuthority::ChartRecord,
    }];
    assert_eq!(
        store.run_rounds(&conflicting_chart, true),
        RoundsRunOutcome::Aborted {
            unchanged_watermark: 0
        }
    );
    assert_eq!(store.rounds_watermark(), 0);

    // 3. Completed run advances watermark to 10 AND preserves both sides of disagreement with distinct authorities!
    assert_eq!(
        store.run_rounds(&conflicting_chart, false),
        RoundsRunOutcome::Updated {
            new_watermark: 10,
            added_entries: 1,
            disagreements_preserved: 1,
        }
    );
    assert_eq!(store.rounds_watermark(), 10);
    let active_anticoag = store.active_entries_for_topic("anticoagulation_preference");
    assert_eq!(active_anticoag.len(), 2);
    assert!(
        active_anticoag
            .iter()
            .any(|e| e.authority == MemoryAuthority::PhysicianInstruction)
    );
    assert!(
        active_anticoag
            .iter()
            .any(|e| e.authority == MemoryAuthority::ChartRecord)
    );

    // Re-running with same watermark produces QuietNoOp.
    assert_eq!(
        store.run_rounds(&conflicting_chart, false),
        RoundsRunOutcome::QuietNoOp { watermark: 10 }
    );

    // 4. History-preserving retirement retires old condition without touching unrelated active entries.
    assert!(store.retire_entry(
        &old_condition_id,
        "2026-01-20",
        "Resolved on follow-up exam"
    ));
    let retired = store
        .all_entries()
        .iter()
        .find(|e| e.entry_id == old_condition_id)
        .expect("retired entry still retained");
    assert!(matches!(retired.state, MemoryEntryState::Retired { .. }));
    let pref = store
        .all_entries()
        .iter()
        .find(|e| e.entry_id == pref_id)
        .expect("unrelated entry intact");
    assert_eq!(pref.state, MemoryEntryState::Active);

    // 5. Model-facing text hygiene check rejects tracker IDs & unoffered tool refs.
    assert!(
        verify_model_facing_text_hygiene(
            "Use tool:search_chart and tool:read_records for ISO-8601 dates.",
            &["search_chart", "read_records"]
        )
        .is_ok()
    );
    let bad =
        verify_model_facing_text_hygiene("Ref ABC-999 and tool:delete_chart", &["search_chart"])
            .expect_err("hygiene must reject tracker ID and unoffered tool");
    assert_eq!(bad.len(), 2);
}

#[test]
fn scn_m3_0006_multi_format_ehr_mapping_native_bytes_part2_and_binding_checks() {
    // Verify pure-Rust FIPS 180-4 SHA-256 against NIST test vectors and closed 16 buckets.
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(ClinicalRecordBucket::ALL.len(), 16);

    // 1. SMART on FHIR R4 PKCE S256 + metadata fallback + same-origin pagination + AssistedOnboarding.
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    assert_eq!(
        pkce_s256_challenge(verifier),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
    let launch = build_smart_authorize_request(
        SmartFhirVendor::EpicMyChart,
        "https://fhir.synthetic.invalid/r4/",
        None,
        Some((
            "https://fhir.synthetic.invalid/oauth/authorize",
            "https://fhir.synthetic.invalid/oauth/token",
        )),
        SmartScopeVersion::V2Cruds,
        verifier,
    )
    .expect("smart launch");
    assert!(launch.uses_metadata_fallback);
    assert_eq!(launch.aud_base_url, "https://fhir.synthetic.invalid/r4");
    assert!(is_same_origin_bundle_next(
        "https://fhir.synthetic.invalid/r4",
        "https://fhir.synthetic.invalid/r4/Observation?page=2"
    ));
    assert!(!is_same_origin_bundle_next(
        "https://fhir.synthetic.invalid/r4",
        "https://attacker.invalid/r4/Observation?page=2"
    ));
    assert!(
        AssistedOnboardingVendor::SimplePractice
            .onboarding_guidance()
            .contains("export")
    );

    // FHIR R4 valid ingestion preserves untouched bytes & rejects mismatched patient binding.
    let fhir_bytes = br#"{"resourceType":"Observation","id":"obs-1","meta":{"versionId":"3"},"subject":{"reference":"Patient/pat-1"},"category":[{"coding":[{"code":"vital-signs"}]}],"effectiveDateTime":"2026-01-12"}"#;
    let stored_fhir =
        ingest_fhir_r4_resource("pat-1", fhir_bytes, "2026-01-15T10:00:00Z").expect("valid FHIR");
    assert_eq!(stored_fhir.bucket, ClinicalRecordBucket::Vitals);
    assert_eq!(stored_fhir.native_bytes, fhir_bytes);
    assert_eq!(stored_fhir.sha256_hex, sha256_hex(fhir_bytes));
    assert!(matches!(
        ingest_fhir_r4_resource("pat-2", fhir_bytes, "2026-01-15T10:00:00Z"),
        Err(IngestionError::PatientBindingMismatch { .. })
    ));

    // 2. KipuEMR + 42 CFR Part 2 SUD & SUD Counseling Notes + § 2.32(a) notice + export gating.
    assert_eq!(
        KipuApiAuthMode::ApiV3HmacSha256.accept_header(),
        "application/vnd.kipusystems+json; version=3"
    );
    let kipu_sud = br#"{"id":"kipu-10","casefile_id":"case-1","record_kind":"note","part2_program":true,"sud_counseling_note":true,"recorded_at":"2026-01-14"}"#;
    let stored_kipu = ingest_kipu_record(
        "case-1",
        kipu_sud,
        "2026-01-15T10:00:00Z",
        "SUD counseling scope for care coordination",
    )
    .expect("valid kipu");
    assert_eq!(
        stored_kipu.confidentiality,
        ConfidentialityTag::Part2SudCounselingNotes
    );
    assert_eq!(
        stored_kipu
            .part2_notice
            .as_ref()
            .expect("notice")
            .statutory_notice_2_32_a,
        PART2_STATUTORY_REDISCLOSURE_NOTICE_2_32_A
    );
    assert!(!stored_kipu.confidentiality.permitted_in_export(true, false));
    assert!(stored_kipu.confidentiality.permitted_in_export(true, true));
    assert!(matches!(
        ingest_kipu_record("case-other", kipu_sud, "2026-01-15T10:00:00Z", "scope"),
        Err(IngestionError::PatientBindingMismatch { .. })
    ));

    // 3. C-CDA R2.1 XML: valid sibling sections commit while <nonXMLBody> isolates into FailedPart.
    let ccda = br#"<ClinicalDocument><recordTarget><patientRole><id extension="pat-ccda-1"/></patientRole></recordTarget><component><nonXMLBody>BASE64CORRUPT</nonXMLBody><structuredBody><component><section><code code="11450-4" codeSystem="2.16.840.1.11383.6.1"/><text>Hypertension</text></section></component><component><section nullFlavor="NI"><code code="48765-2"/></section></component></structuredBody></component></ClinicalDocument>"#;
    let ccda_res =
        ingest_ccda_r21_xml("pat-ccda-1", ccda, "2026-01-15T10:00:00Z").expect("valid ccda");
    assert_eq!(ccda_res.admitted.len(), 1);
    assert_eq!(ccda_res.admitted[0].bucket, ClinicalRecordBucket::Problems);
    assert_eq!(ccda_res.failed_parts.len(), 2);
    assert!(matches!(
        ingest_ccda_r21_xml("pat-wrong", ccda, "2026-01-15T10:00:00Z"),
        Err(IngestionError::PatientBindingMismatch { .. })
    ));

    // 4. HL7 v2.x ORU: escape decoding, PID-3 binding, OBX ED isolated into FailedPart.
    assert_eq!(
        decode_hl7_escapes(r"A\F\B\S\C\.br\Line2", '|', '^', '~', '\\', '&'),
        "A|B^C\nLine2"
    );
    let hl7 = b"MSH|^~\\&|LAB|HOSP|CURIE|CLINIC|20260115||ORU^R01|MSG1|P|2.5\rPID|1||pat-hl7-1^^^MRN||Synthetic^Patient\rOBX|1|NM|2160-0^Creatinine||1.02|mg/dL\rOBX|2|ED|PDF^Attachment||^Application^PDF^Base64^AAAA\r";
    let hl7_res =
        ingest_hl7_v2_message("pat-hl7-1", hl7, "2026-01-15T10:00:00Z").expect("valid hl7");
    assert_eq!(hl7_res.admitted.len(), 2);
    assert_eq!(hl7_res.admitted[0].bucket, ClinicalRecordBucket::Patient);
    assert_eq!(hl7_res.admitted[1].bucket, ClinicalRecordBucket::Results);
    assert_eq!(hl7_res.failed_parts.len(), 1);
    assert!(matches!(
        ingest_hl7_v2_message("pat-wrong", hl7, "2026-01-15T10:00:00Z"),
        Err(IngestionError::PatientBindingMismatch { .. })
    ));

    // 5. SMART Health Links (`shlink:/`) validation + Gmail/Spruce adapters.
    let key32 = [7u8; 32];
    assert!(
        validate_shlink_payload(
            "shlink:/eyJ1cmwiOiJodHRwczovL2V4YW1wbGUuY29tIn0",
            "https://records.synthetic.invalid/manifest.json",
            &key32,
            "L",
            Some(2_000),
            1_000,
        )
        .is_ok()
    );
    assert!(
        validate_shlink_payload(
            "shlink:/abc",
            "http://records.synthetic.invalid/manifest.json",
            &key32,
            "L",
            None,
            1_000,
        )
        .is_err()
    );
    assert!(
        validate_shlink_payload(
            "shlink:/abc",
            "https://127.0.0.1/manifest.json",
            &key32,
            "L",
            None,
            1_000,
        )
        .is_err()
    );

    let spruce = ClinicChannelAdapter::new(
        "spruce-clinic",
        ClinicChannelKind::SpruceMessaging,
        ChannelHealthStatus::TempError {
            detail: "rate limited".to_owned(),
        },
    );
    assert!(!spruce.auto_send_permitted());
    assert!(matches!(
        spruce.today_banner(),
        TodayChannelBanner::TemporaryTrouble {
            asks_to_reconnect: false,
            ..
        }
    ));
}
