use std::collections::BTreeMap;
use std::thread;

use ef_clinical_lib::{
    ArticleId, ArticleState, CandidateVersionId, ClinicalLibraryNightly, ConceptQuery,
    CutOffReason, ExecutionQuotaTracker, FailureCode, FrozenSkillPackage, GuardrailEvent,
    GuardrailTerminalStatus, JsonPropertyType, ModelRolePolicy, NightOutcome, NightRunId,
    NightlyError, OPERATIONAL_LOG_BODY, OPS_TELEMETRY_QUEUE_SIZE, OperatorAlert, OpsRecordKind,
    OpsTelemetryPipeline, PUBMED_MIN_SPACING_MS, PipelineHaltState, PubMedError,
    PubMedRecordPayload, PubMedRefreshDecision, PubMedRefresher, PubMedStatus,
    PubMedTransportOutcome, ReaderVersion, ReasoningEffort, RecoveryTrigger,
    SharedPubMedRateLimiter, SkillViewError, SlotHolderStatus, ToolSchemaSpec, ToolValidationError,
    sha256_hex,
};

#[test]
fn scn_m3_0014_shared_harness_skill_view_tool_validation_cutoff_and_content_free_telemetry() {
    // 1. Frozen in-memory SHA-256-verified skill package (`skill_view`, zero disk I/O).
    let mut resources = BTreeMap::new();
    resources.insert(
        "references/renal-dosing.md".to_owned(),
        "Verify eGFR before metformin adjustment.".to_owned(),
    );
    let mut entries = BTreeMap::new();
    entries.insert(
        "clinical-renal".to_owned(),
        ("Renal dosing reference".to_owned(), resources),
    );
    let pkg = FrozenSkillPackage::freeze(entries);
    let digest = pkg.package_sha256().to_owned();
    assert_eq!(digest.len(), 64);
    pkg.verify_digest(&digest).expect("digest matches");
    assert!(matches!(
        pkg.verify_digest("0000"),
        Err(SkillViewError::DigestMismatch { .. })
    ));

    // Catalog exposes metadata only (never body content).
    let catalog = pkg.catalog();
    assert_eq!(catalog.len(), 1);
    assert_eq!(catalog[0].name, "clinical-renal");
    assert_eq!(
        catalog[0].resource_paths,
        vec!["references/renal-dosing.md"]
    );

    // Valid skill_view reads solely from frozen memory; path traversal & unknown skill fail cleanly.
    assert_eq!(
        pkg.skill_view("clinical-renal", "references/renal-dosing.md")
            .expect("valid skill_view"),
        "Verify eGFR before metformin adjustment."
    );
    assert!(matches!(
        pkg.skill_view("../etc", "passwd"),
        Err(SkillViewError::PathTraversalRejected { .. })
    ));
    assert!(matches!(
        pkg.skill_view("clinical-renal", "../../secret.txt"),
        Err(SkillViewError::PathTraversalRejected { .. })
    ));
    assert!(matches!(
        pkg.skill_view("unknown-skill", "SKILL.md"),
        Err(SkillViewError::UnknownSkill { .. })
    ));

    // 2. Typed JSON Schema tool argument validation returns structured errors.
    let mut required = BTreeMap::new();
    required.insert("query".to_owned(), JsonPropertyType::String);
    required.insert("page".to_owned(), JsonPropertyType::Integer);
    let schema = ToolSchemaSpec {
        tool_name: "search_chart".to_owned(),
        required_fields: required,
        optional_fields: BTreeMap::new(),
        reject_unknown_fields: true,
    };
    schema
        .validate(&serde_json::json!({ "query": "creatinine", "page": 1 }))
        .expect("valid args");
    assert!(matches!(
        schema.validate(&serde_json::json!({ "query": "creatinine", "page": "one" })),
        Err(ToolValidationError::InvalidFieldType { ref field, .. }) if field == "page"
    ));
    assert!(matches!(
        schema.validate(&serde_json::json!({ "query": "creatinine", "page": 1, "extra": true })),
        Err(ToolValidationError::UnknownField { ref field, .. }) if field == "extra"
    ));

    // 3. Explicit model role settings, bounded CutOff preserving kept work, and exact event reconstruction.
    let policy = ModelRolePolicy {
        role_name: "clinical-assistant".to_owned(),
        model_id: "stub-clinical-v1".to_owned(),
        reasoning_effort: ReasoningEffort::Medium,
        working_window_tokens: 8_192,
        input_token_limit: 4_096,
        max_turns: 2,
        max_tool_steps: 2,
    };
    let mut tracker =
        ExecutionQuotaTracker::start(policy.clone(), 1_700_000_000_000).expect("start");
    assert!(tracker.begin_turn(500).is_none());
    assert!(tracker.record_tool_step("search_chart").is_none());
    tracker.keep_delta("Kept clinical partial prose.");
    assert!(tracker.record_tool_step("read_records").is_none());
    // 3rd tool step exceeds max_tool_steps=2 -> CutOff while preserving kept output!
    let terminal = tracker
        .record_tool_step("extra_tool")
        .expect("must hit tool step cutoff");
    assert_eq!(
        terminal,
        GuardrailTerminalStatus::CutOff {
            reason: CutOffReason::ToolStepLimitReached
        }
    );
    assert_eq!(tracker.kept_output(), "Kept clinical partial prose.");
    assert!(matches!(
        tracker.events().first(),
        Some(GuardrailEvent::WindowOpened {
            unix_ms: 1_700_000_000_000
        })
    ));

    // Exact restart reconstruction solely from admitted durable events.
    let reconstructed =
        ExecutionQuotaTracker::reconstruct_from_events(policy, tracker.events()).expect("rebuild");
    assert_eq!(reconstructed.kept_output(), "Kept clinical partial prose.");
    assert_eq!(reconstructed.terminal(), tracker.terminal());

    // 4. Content-free operational telemetry pipeline (strict allowlist + non-blocking 1000-slot queue).
    let mut ops = OpsTelemetryPipeline::default();
    let mut raw_attrs = BTreeMap::new();
    raw_attrs.insert("service.name".to_owned(), "ef-clinic-server".to_owned());
    raw_attrs.insert("run.id".to_owned(), "run-42".to_owned());
    raw_attrs.insert("outcome".to_owned(), "cut_off".to_owned());
    raw_attrs.insert("tokens.input".to_owned(), "500".to_owned());
    // Forbidden clinical content fields MUST be stripped by allowlist.
    raw_attrs.insert("prompt".to_owned(), "Sensitive patient history".to_owned());
    raw_attrs.insert(
        "completion".to_owned(),
        "Private clinical advice".to_owned(),
    );
    raw_attrs.insert("patient.name".to_owned(), "Synthetic Patient".to_owned());

    assert!(ops.emit(
        OpsRecordKind::LogEvent,
        "free-form error containing clinical text",
        &raw_attrs
    ));
    let exported = ops.drain_exported();
    assert_eq!(exported.len(), 1);
    assert_eq!(exported[0].body, OPERATIONAL_LOG_BODY);
    assert!(exported[0].status_message.is_empty());
    assert!(!exported[0].attributes.contains_key("prompt"));
    assert!(!exported[0].attributes.contains_key("completion"));
    assert!(!exported[0].attributes.contains_key("patient.name"));
    assert_eq!(
        exported[0].attributes.get("run.id").map(String::as_str),
        Some("run-42")
    );

    // Non-blocking overflow protection (`queue_size = 1000`, `block_on_overflow = false`).
    for _ in 0..(OPS_TELEMETRY_QUEUE_SIZE + 25) {
        let _ = ops.emit(OpsRecordKind::TraceSpan, "", &raw_attrs);
    }
    assert_eq!(ops.queued_len(), OPS_TELEMETRY_QUEUE_SIZE);
    assert_eq!(ops.dropped_overflow(), 25);
}

#[test]
fn scn_m3_0015_all_9_nightly_guarantees_and_deidentified_concept_queries() {
    // De-identified concept query guard rejects patient identifiers.
    assert!(ConceptQuery::parse("chronic kidney disease metformin dosing").is_ok());
    assert!(ConceptQuery::parse("patient_id=12345 creatinine").is_err());
    assert!(ConceptQuery::parse("MRN:998811 hypertension").is_err());
    assert!(ConceptQuery::parse("123-45-6789 labs").is_err());

    let mut nightly = ClinicalLibraryNightly::new();
    let art_wait = ArticleId::new("art-wait-1").expect("id");
    let art_review = ArticleId::new("art-review-1").expect("id");
    let code_parse = FailureCode::new("parse_xml_truncated").expect("code");
    let code_net = FailureCode::new("upstream_timeout").expect("code");

    nightly.insert_article(
        art_wait.clone(),
        ArticleState::WaitingToRetry {
            last_failure: code_parse.clone(),
            attempts: 1,
            reader_version: ReaderVersion(1),
        },
    );
    nightly.insert_article(
        art_review.clone(),
        ArticleState::HumanReview {
            reason: "ambiguous citation date".to_owned(),
            reader_version: ReaderVersion(1),
        },
    );

    // Rule 2: Retry scope selects WaitingToRetry ONLY (not HumanReview).
    assert_eq!(nightly.due_retry_articles(), vec![art_wait.clone()]);

    // Rule 6 & Rule 7: Cannot take over running or unreadable holder; 2nd consecutive unreadable triggers operator alert!
    assert!(matches!(
        nightly.acquire_slot(
            NightRunId::new("night-0").expect("id"),
            SlotHolderStatus::Running {
                holder: NightRunId::new("prior-active").expect("id")
            }
        ),
        Err(NightlyError::PriorHolderStillRunning { .. })
    ));
    assert!(matches!(
        nightly.acquire_slot(
            NightRunId::new("night-u1").expect("id"),
            SlotHolderStatus::Unreadable {
                detail: "corrupt slot lock".to_owned()
            }
        ),
        Err(NightlyError::PriorHolderUnreadable { .. })
    ));
    assert!(nightly.alerts().is_empty());
    assert!(matches!(
        nightly.acquire_slot(
            NightRunId::new("night-u2").expect("id"),
            SlotHolderStatus::Unreadable {
                detail: "corrupt slot lock".to_owned()
            }
        ),
        Err(NightlyError::PriorHolderUnreadable { .. })
    ));
    assert_eq!(
        nightly.alerts(),
        &[OperatorAlert::UnreadableSlotHolder {
            consecutive_nights: 2,
            detail: "corrupt slot lock".to_owned()
        }]
    );

    // Rule 4: Same failure code across 2 nights halts automatic retries until explicit operator resolution.
    let l1 = nightly
        .acquire_slot(
            NightRunId::new("night-1").expect("id"),
            SlotHolderStatus::Empty,
        )
        .expect("slot 1");
    nightly
        .finish_night(
            l1,
            NightOutcome::Failed {
                code: code_parse.clone(),
            },
        )
        .expect("finish 1");
    assert_eq!(nightly.halt_state(), &PipelineHaltState::Running);

    let l2 = nightly
        .acquire_slot(
            NightRunId::new("night-2").expect("id"),
            SlotHolderStatus::Ended {
                holder: NightRunId::new("night-1").expect("id"),
            },
        )
        .expect("slot 2");
    nightly
        .finish_night(
            l2,
            NightOutcome::Failed {
                code: code_parse.clone(),
            },
        )
        .expect("finish 2");
    assert_eq!(
        nightly.halt_state(),
        &PipelineHaltState::HaltedRepeatedFailureCode {
            code: code_parse.clone(),
            nights_observed: 2
        }
    );
    assert!(matches!(
        nightly.acquire_slot(
            NightRunId::new("night-blocked").expect("id"),
            SlotHolderStatus::Empty
        ),
        Err(NightlyError::PipelineHalted { .. })
    ));
    nightly.resolve_operator_halt("op-1");

    // Rule 5: 3 consecutive failed/zero-progress deferred nights halt nightly until resolved.
    for idx in 1..=3u32 {
        let lease = nightly
            .acquire_slot(
                NightRunId::new(format!("night-idle-{idx}")).expect("id"),
                SlotHolderStatus::Empty,
            )
            .expect("slot");
        let outcome = if idx == 1 {
            NightOutcome::Failed {
                code: code_net.clone(),
            }
        } else {
            NightOutcome::Deferred {
                articles_processed: 0,
                remaining_due: 1,
            }
        };
        nightly.finish_night(lease, outcome).expect("finish");
    }
    assert_eq!(
        nightly.halt_state(),
        &PipelineHaltState::HaltedConsecutiveFailedOrIdle {
            consecutive_nights: 3
        }
    );
    nightly.resolve_operator_halt("op-1");

    // Rule 8 (Lost-slot isolation) & Rule 9 (Single checked active version + write-once folders).
    let stale_run = NightRunId::new("night-stale").expect("id");
    let stale_lease = nightly
        .acquire_slot(stale_run.clone(), SlotHolderStatus::Empty)
        .expect("stale slot");
    let v1 = CandidateVersionId::new("ver-2026-06-15").expect("id");
    let v2 = CandidateVersionId::new("ver-2026-06-16").expect("id");

    // Valid takeover after prior holder confirmed ended invalidates `stale_lease`!
    let live_run = NightRunId::new("night-live").expect("id");
    let live_lease = nightly
        .acquire_slot(
            live_run,
            SlotHolderStatus::Ended {
                holder: stale_run.clone(),
            },
        )
        .expect("live takeover");

    // Lost-slot worker writes ZERO candidate files, ZERO DB rows, ZERO activations!
    assert!(matches!(
        nightly.write_candidate_artifact(&stale_lease, &v1, "index.bin", vec![1, 2, 3]),
        Err(NightlyError::SlotLeaseLost { .. })
    ));
    assert!(matches!(
        nightly.retry_waiting_article(&stale_lease, &art_wait, v1.clone(), "abc".to_owned()),
        Err(NightlyError::SlotLeaseLost { .. })
    ));
    assert_eq!(nightly.db_writes_for_run(&stale_run), 0);
    assert!(nightly.candidate(&v1).is_none());

    // Live worker writes write-once candidate folder, checks it, and activates exactly one version.
    let _ = nightly
        .write_candidate_artifact(&live_lease, &v1, "articles.jsonl", b"row1".to_vec())
        .expect("write once");
    assert!(matches!(
        nightly.write_candidate_artifact(&live_lease, &v1, "articles.jsonl", b"overwrite".to_vec()),
        Err(NightlyError::CandidateFolderWriteOnce { .. })
    ));
    let handle_v1 = nightly
        .check_and_mint_activation_handle(&live_lease, &v1)
        .expect("checked v1");
    nightly.activate_version(handle_v1).expect("activate v1");
    assert_eq!(nightly.active_version(), Some(&v1));
    assert_eq!(nightly.active_version_count(), 1);

    // Activating v2 atomically replaces v1 so active_version_count stays strictly 1.
    let _ = nightly
        .write_candidate_artifact(&live_lease, &v2, "articles.jsonl", b"row2".to_vec())
        .expect("write v2");
    let handle_v2 = nightly
        .check_and_mint_activation_handle(&live_lease, &v2)
        .expect("checked v2");
    nightly.activate_version(handle_v2).expect("activate v2");
    assert_eq!(nightly.active_version(), Some(&v2));
    assert_eq!(nightly.active_version_count(), 1);

    // Rule 2 & Rule 3: Cannot retry HumanReview directly; cannot mark Succeeded while due items remain.
    assert!(matches!(
        nightly.retry_waiting_article(&live_lease, &art_review, v2.clone(), "xyz".to_owned()),
        Err(NightlyError::NotWaitingToRetry { .. })
    ));
    assert!(matches!(
        nightly.finish_night(
            live_lease,
            NightOutcome::Succeeded {
                articles_processed: 0
            }
        ),
        Err(NightlyError::IncompleteNightCannotSucceed { remaining_due: 1 })
    ));

    // Rule 1: Only explicit operator/new-content/new-reader-version moves HumanReview to WaitingToRetry.
    assert!(matches!(
        nightly.recover_article(
            &art_review,
            RecoveryTrigger::NewReaderVersion {
                reader_version: ReaderVersion(1)
            },
            code_parse.clone(),
        ),
        Err(NightlyError::InvalidArticleTransition { .. })
    ));
    nightly
        .recover_article(
            &art_review,
            RecoveryTrigger::NewReaderVersion {
                reader_version: ReaderVersion(2),
            },
            code_parse,
        )
        .expect("strictly newer reader version recovers article");

    // Re-acquire after ending, finish both waiting articles, then mark night Succeeded cleanly.
    let final_lease = nightly
        .acquire_slot(
            NightRunId::new("night-final").expect("id"),
            SlotHolderStatus::Ended {
                holder: NightRunId::new("night-live").expect("id"),
            },
        )
        .expect("final slot");
    for id in [art_wait, art_review] {
        nightly
            .retry_waiting_article(
                &final_lease,
                &id,
                v2.clone(),
                sha256_hex(id.as_str().as_bytes()),
            )
            .expect("retry");
    }
    nightly
        .finish_night(
            final_lease,
            NightOutcome::Succeeded {
                articles_processed: 2,
            },
        )
        .expect("complete night succeeds");
}

#[test]
fn scn_m3_0015_pubmed_100ms_concurrent_pacing_verbatim_hash_and_auth_fail_fast() {
    let limiter = SharedPubMedRateLimiter::default();
    let mut handles = Vec::new();
    for _ in 0..8 {
        let lim = limiter.clone();
        handles.push(thread::spawn(move || {
            lim.acquire_start_ms(1_000).expect("slot")
        }));
    }
    for h in handles {
        let _ = h.join().expect("thread join");
    }
    let starts = limiter.granted_starts_ms();
    assert_eq!(starts.len(), 8);
    for pair in starts.windows(2) {
        assert!(
            pair[1] - pair[0] >= PUBMED_MIN_SPACING_MS,
            "concurrent PubMed starts must pace >= 100 ms apart (got {} -> {})",
            pair[0],
            pair[1]
        );
    }

    let mut refresher = PubMedRefresher::new(SharedPubMedRateLimiter::default());
    refresher.register_book_pmid("NBK551234", "31985199");

    // First fetch inserts; identical second fetch re-confirms without replacing.
    let xml = "<PubmedArticle><PMID>39000001</PMID><PublicationType>Journal Article</PublicationType></PubmedArticle>";
    let (t1, d1) = refresher
        .refresh_record(
            5_000,
            PubMedTransportOutcome::Payload(PubMedRecordPayload {
                pmid: Some("39000001".to_owned()),
                book_accession: None,
                publication_types: vec!["Journal Article".to_owned()],
                verbatim_notice: None,
                raw_xml: xml.to_owned(),
                deleted: false,
            }),
        )
        .expect("first fetch");
    assert_eq!(t1, 5_000);
    assert!(matches!(
        d1,
        PubMedRefreshDecision::Updated {
            status: PubMedStatus::Current,
            ..
        }
    ));

    let (t2, d2) = refresher
        .refresh_record(
            5_010,
            PubMedTransportOutcome::Payload(PubMedRecordPayload {
                pmid: Some("39000001".to_owned()),
                book_accession: None,
                publication_types: vec!["Journal Article".to_owned()],
                verbatim_notice: None,
                raw_xml: xml.to_owned(),
                deleted: false,
            }),
        )
        .expect("unchanged reconfirm");
    assert_eq!(t2, 5_100, "second start must pace >= 100 ms after 5000");
    assert!(matches!(
        d2,
        PubMedRefreshDecision::UnchangedReconfirmed { .. }
    ));
    assert_eq!(refresher.entry("39000001").expect("entry").replacements, 0);

    // Record lacking publication types classifies explicitly as Unreadable (never empty).
    let (_, d3) = refresher
        .refresh_record(
            5_200,
            PubMedTransportOutcome::Payload(PubMedRecordPayload {
                pmid: Some("39000002".to_owned()),
                book_accession: None,
                publication_types: Vec::new(),
                verbatim_notice: None,
                raw_xml: "<PubmedArticle/>".to_owned(),
                deleted: false,
            }),
        )
        .expect("unreadable");
    assert_eq!(
        d3,
        PubMedRefreshDecision::UnreadableMissingPublicationType {
            pmid: "39000002".to_owned()
        }
    );

    // Deleted citation returns DeletedEmpty; book record resolves PMID.
    let (_, d4) = refresher
        .refresh_record(
            5_300,
            PubMedTransportOutcome::Payload(PubMedRecordPayload {
                pmid: Some("39000003".to_owned()),
                book_accession: None,
                publication_types: Vec::new(),
                verbatim_notice: None,
                raw_xml: String::new(),
                deleted: true,
            }),
        )
        .expect("deleted");
    assert_eq!(d4, PubMedRefreshDecision::DeletedEmpty);

    let (_, d5) = refresher
        .refresh_record(
            5_400,
            PubMedTransportOutcome::Payload(PubMedRecordPayload {
                pmid: None,
                book_accession: Some("NBK551234".to_owned()),
                publication_types: vec!["Review".to_owned()],
                verbatim_notice: None,
                raw_xml: "<BookDocument>NBK551234</BookDocument>".to_owned(),
                deleted: false,
            }),
        )
        .expect("book");
    assert!(matches!(
        d5,
        PubMedRefreshDecision::BookResolved { ref pmid, .. } if pmid == "31985199"
    ));

    // Auth refusal fails immediately without retrying or exposing keys.
    let err = refresher
        .refresh_record(
            5_500,
            PubMedTransportOutcome::AuthRefused { http_status: 403 },
        )
        .expect_err("auth refusal must fail fast");
    assert_eq!(err, PubMedError::AuthorizationRefused { http_status: 403 });
    assert!(!err.to_string().contains("api_key"));
}
