//! Integration tests verifying Curie Clinic adapters (`TurnStateAdapter for ChatEngine`,
//! `SearchChartTool`, `ReadRecordsTool`) wired to product-neutral `ef-harness`.

use ef_clinic_domain::chat::{
    AnswerState, ChatEngine, CommandEnvelope, CommandPayload, DeliveryMode, ExchangeSteps,
    MAX_FAILURE_ATTEMPTS, ResumeWorkPayload, RetryBackoffMode, StopWorkPayload,
    SubmitMessagePayload,
};
use ef_clinic_domain::harness_adapter::{ReadRecordsTool, SearchChartTool};
use ef_clinic_domain::workspace::{WorkspaceEvidenceStore, synthetic_5000_row_lab_source};
use ef_core::{ChatId, IdempotencyKey, PhysicianId, PracticeId, SourceKey, WorkspaceId};
use ef_harness::{
    AgentLoop, AgentTool, CancellationToken, HarnessError, StubProvider, ToolError,
    TurnExecutionOutcome,
};

fn build_fixture_store() -> (
    PracticeId,
    PhysicianId,
    WorkspaceId,
    WorkspaceId,
    SourceKey,
    SourceKey,
    WorkspaceEvidenceStore,
) {
    let prac_a = PracticeId::new("prac-a").expect("prac a");
    let prac_b = PracticeId::new("prac-b").expect("prac b");
    let phys_1 = PhysicianId::new("phys-1").expect("phys 1");
    let phys_2 = PhysicianId::new("phys-2").expect("phys 2");
    let ws_a1 = WorkspaceId::new("pw-flagship-1").expect("ws a1");
    let ws_b1 = WorkspaceId::new("pw-other-b1").expect("ws b1");
    let src_a1 = SourceKey::new("src-lab-12").expect("src a1");
    let src_b1 = SourceKey::new("src-b1-secret").expect("src b1");

    let mut store = WorkspaceEvidenceStore::new();
    store.register_workspace(
        ws_a1.clone(),
        prac_a.clone(),
        "Synthetic A1",
        vec![phys_1.clone()],
    );
    store.register_workspace(ws_b1.clone(), prac_b, "Synthetic B1", vec![phys_2]);
    store.insert_source(synthetic_5000_row_lab_source(ws_a1.clone(), src_a1.clone()));
    store.insert_source(synthetic_5000_row_lab_source(ws_b1.clone(), src_b1.clone()));
    (prac_a, phys_1, ws_a1, ws_b1, src_a1, src_b1, store)
}

#[test]
fn clinic_chat_engine_runs_two_tool_rounds_and_emits_content_free_telemetry() {
    let (prac_a, phys_1, ws_a1, _, src_a1, _, store) = build_fixture_store();
    let chat = ChatId::new("chat-1").expect("chat");
    let mut engine = ChatEngine::new(RetryBackoffMode::DeterministicTest);
    let acc = engine
        .execute_command(
            CommandEnvelope {
                idempotency_key: IdempotencyKey::new("idem-ch1").expect("idem"),
                command: CommandPayload::MessageSubmit(SubmitMessagePayload {
                    patient_workspace_id: ws_a1,
                    chat_id: chat,
                    text: "Check renal stability".to_owned(),
                    physician_time_zone: "UTC".to_owned(),
                    delivery: Some(DeliveryMode::SendNow),
                }),
            },
            100,
        )
        .expect("submit");

    let search = SearchChartTool {
        practice_id: prac_a.clone(),
        physician_id: phys_1.clone(),
        store: &store,
    };
    let read = ReadRecordsTool {
        practice_id: prac_a,
        physician_id: phys_1,
        store: &store,
    };
    let tools: [&dyn AgentTool; 2] = [&search, &read];

    let mut stub = StubProvider::clinical_two_round_fixture(src_a1);
    let mut agent = AgentLoop::new();
    let cancel = CancellationToken::new();

    let outcome = agent
        .run_turn_attempt(
            &mut engine,
            &acc.work_item_id,
            &mut stub,
            &tools,
            &cancel,
            105,
        )
        .expect("attempt succeeds");
    assert!(matches!(outcome, TurnExecutionOutcome::Finished { .. }));

    assert_eq!(stub.recorded_requests_jsonl().len(), 3);
    let exchange = engine.exchange(&acc.work_item_id).expect("exchange");
    assert!(matches!(
        exchange.steps,
        ExchangeSteps::Read { ref steps } if steps.len() == 2
    ));
    assert!(matches!(exchange.answer, AnswerState::Finished { .. }));

    let telemetry_json = serde_json::to_string(agent.telemetry_spans()).expect("serialize spans");
    assert!(!telemetry_json.contains("Check renal stability"));
    assert!(!telemetry_json.contains("Synthetic renal clearance"));
}

#[test]
fn clinic_chat_engine_handles_mid_stream_cancel_and_5_failure_try_again() {
    let (prac_a, phys_1, ws_a1, _, src_a1, _, store) = build_fixture_store();
    let chat = ChatId::new("chat-2").expect("chat");
    let search = SearchChartTool {
        practice_id: prac_a.clone(),
        physician_id: phys_1.clone(),
        store: &store,
    };
    let read = ReadRecordsTool {
        practice_id: prac_a,
        physician_id: phys_1,
        store: &store,
    };
    let tools: [&dyn AgentTool; 2] = [&search, &read];

    let mut engine = ChatEngine::new(RetryBackoffMode::DeterministicTest);
    let acc = engine
        .execute_command(
            CommandEnvelope {
                idempotency_key: IdempotencyKey::new("idem-ch-cancel").expect("idem"),
                command: CommandPayload::MessageSubmit(SubmitMessagePayload {
                    patient_workspace_id: ws_a1.clone(),
                    chat_id: chat.clone(),
                    text: "Slow stream test".to_owned(),
                    physician_time_zone: "UTC".to_owned(),
                    delivery: None,
                }),
            },
            200,
        )
        .expect("submit");

    let mut stub = StubProvider::clinical_two_round_fixture(src_a1.clone());
    stub.cancel_after_chunks = Some(2);
    let mut agent = AgentLoop::new();
    let cancel = CancellationToken::new();
    let outcome = agent
        .run_turn_attempt(
            &mut engine,
            &acc.work_item_id,
            &mut stub,
            &tools,
            &cancel,
            201,
        )
        .expect("mid-stream cancel");
    assert_eq!(outcome, TurnExecutionOutcome::StoppedMidStream);
    assert!(
        engine
            .exchange(&acc.work_item_id)
            .expect("ex")
            .kept_partial_text
            .contains("Synthetic renal clearance remains stable")
    );

    engine
        .execute_command(
            CommandEnvelope {
                idempotency_key: IdempotencyKey::new("idem-ch-stop").expect("idem"),
                command: CommandPayload::WorkStop(StopWorkPayload {
                    patient_workspace_id: ws_a1.clone(),
                    chat_id: chat.clone(),
                    work_item_id: acc.work_item_id.clone(),
                }),
            },
            202,
        )
        .expect("stop");

    let acc_fail = engine
        .execute_command(
            CommandEnvelope {
                idempotency_key: IdempotencyKey::new("idem-ch-fail5").expect("idem"),
                command: CommandPayload::MessageSubmit(SubmitMessagePayload {
                    patient_workspace_id: ws_a1.clone(),
                    chat_id: chat.clone(),
                    text: "Fault injection test".to_owned(),
                    physician_time_zone: "UTC".to_owned(),
                    delivery: None,
                }),
            },
            300,
        )
        .expect("submit");

    let mut faulty_stub = StubProvider::clinical_two_round_fixture(src_a1);
    faulty_stub.remaining_failures = MAX_FAILURE_ATTEMPTS;
    let fresh_cancel = CancellationToken::new();

    for expected_fail in 1..MAX_FAILURE_ATTEMPTS {
        let res = agent
            .run_turn_attempt(
                &mut engine,
                &acc_fail.work_item_id,
                &mut faulty_stub,
                &tools,
                &fresh_cancel,
                300 + u64::from(expected_fail),
            )
            .expect("retryable failure");
        assert_eq!(
            res,
            TurnExecutionOutcome::ScheduledRetry {
                failed_attempts: expected_fail
            }
        );
    }

    let fifth = agent
        .run_turn_attempt(
            &mut engine,
            &acc_fail.work_item_id,
            &mut faulty_stub,
            &tools,
            &fresh_cancel,
            310,
        )
        .expect("5th failure");
    assert_eq!(
        fifth,
        TurnExecutionOutcome::ExhaustedCouldntFinish {
            failed_attempts: MAX_FAILURE_ATTEMPTS
        }
    );

    let expected_run_id = match &engine.exchange(&acc_fail.work_item_id).expect("ex").answer {
        AnswerState::CouldntFinish { try_again: Some(h) } => h.expected_run_id.clone(),
        other => panic!("expected CouldntFinish handle, got {other:?}"),
    };

    engine
        .execute_command(
            CommandEnvelope {
                idempotency_key: IdempotencyKey::new("idem-ch-tryagain").expect("idem"),
                command: CommandPayload::WorkResume(ResumeWorkPayload {
                    patient_workspace_id: ws_a1,
                    chat_id: chat,
                    work_item_id: acc_fail.work_item_id.clone(),
                    expected_run_id,
                }),
            },
            320,
        )
        .expect("try again accepted");

    let sixth = agent
        .run_turn_attempt(
            &mut engine,
            &acc_fail.work_item_id,
            &mut faulty_stub,
            &tools,
            &fresh_cancel,
            321,
        )
        .expect("attempt 6 succeeds");
    assert!(matches!(sixth, TurnExecutionOutcome::Finished { .. }));
}

#[test]
fn clinic_read_records_tool_enforces_cross_patient_isolation_fail_closed() {
    let (prac_a, phys_1, ws_a1, _, _, src_b1, store) = build_fixture_store();
    let chat = ChatId::new("chat-iso").expect("chat");
    let mut engine = ChatEngine::new(RetryBackoffMode::DeterministicTest);
    let acc = engine
        .execute_command(
            CommandEnvelope {
                idempotency_key: IdempotencyKey::new("idem-ch-iso").expect("idem"),
                command: CommandPayload::MessageSubmit(SubmitMessagePayload {
                    patient_workspace_id: ws_a1,
                    chat_id: chat,
                    text: "Attempt cross-patient source read".to_owned(),
                    physician_time_zone: "UTC".to_owned(),
                    delivery: None,
                }),
            },
            400,
        )
        .expect("submit");

    let search = SearchChartTool {
        practice_id: prac_a.clone(),
        physician_id: phys_1.clone(),
        store: &store,
    };
    let read = ReadRecordsTool {
        practice_id: prac_a,
        physician_id: phys_1,
        store: &store,
    };
    let tools: [&dyn AgentTool; 2] = [&search, &read];

    let mut bad_stub = StubProvider::clinical_two_round_fixture(src_b1);
    let mut agent = AgentLoop::new();
    let cancel = CancellationToken::new();

    let err = agent
        .run_turn_attempt(
            &mut engine,
            &acc.work_item_id,
            &mut bad_stub,
            &tools,
            &cancel,
            401,
        )
        .expect_err("cross-patient read must fail closed");
    assert!(matches!(
        err,
        HarnessError::Tool(ToolError::AccessDenied { .. })
    ));
}
