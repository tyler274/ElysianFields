//! Product-neutral integration tests for `ef-harness` proving multi-round tool calls,
//! incremental delta streaming, mid-stream cancellation, bounded 5-failure retry exhaustion,
//! fail-closed tool access rejection, JSONL transcript capture, and content-free telemetry
//! with zero product-specific crate dependencies.

use std::collections::BTreeMap;

use ef_core::{AttemptId, ChatId, SourceKey, WorkItemId, WorkspaceId};
use ef_harness::{
    AgentLoop, AgentTool, CancellationToken, DEFAULT_MAX_FAILURE_ATTEMPTS, FinalAnswerDraft,
    HarnessError, StubProvider, ToolCall, ToolError, ToolOutput, TurnContext, TurnExecutionOutcome,
    TurnStateAdapter,
};

struct NeutralTurnRecord {
    context: TurnContext,
    deltas: String,
    recorded_steps: Vec<serde_json::Value>,
    completed_answer: Option<FinalAnswerDraft>,
}

#[derive(Default)]
struct InMemoryTurnAdapter {
    turns: BTreeMap<WorkItemId, NeutralTurnRecord>,
}

impl InMemoryTurnAdapter {
    fn insert_turn(
        &mut self,
        workspace_id: WorkspaceId,
        chat_id: ChatId,
        work_item_id: WorkItemId,
        prompt_text: impl Into<String>,
    ) {
        self.turns.insert(
            work_item_id.clone(),
            NeutralTurnRecord {
                context: TurnContext {
                    workspace_id,
                    chat_id,
                    work_item_id,
                    prompt_text: prompt_text.into(),
                    failed_attempts: 0,
                },
                deltas: String::new(),
                recorded_steps: Vec::new(),
                completed_answer: None,
            },
        );
    }
}

impl TurnStateAdapter for InMemoryTurnAdapter {
    fn load_turn_context(&self, work_item_id: &WorkItemId) -> Result<TurnContext, HarnessError> {
        self.turns
            .get(work_item_id)
            .map(|r| r.context.clone())
            .ok_or_else(|| HarnessError::StateTransition {
                message: format!("work item '{}' not found", work_item_id.as_str()),
            })
    }

    fn append_delta(
        &mut self,
        work_item_id: &WorkItemId,
        text_delta: &str,
        _now_secs: u64,
    ) -> Result<(), HarnessError> {
        let rec =
            self.turns
                .get_mut(work_item_id)
                .ok_or_else(|| HarnessError::StateTransition {
                    message: "missing turn".to_owned(),
                })?;
        rec.deltas.push_str(text_delta);
        Ok(())
    }

    fn record_tool_output(
        &mut self,
        work_item_id: &WorkItemId,
        output: &ToolOutput,
        _now_secs: u64,
    ) -> Result<(), HarnessError> {
        let rec =
            self.turns
                .get_mut(work_item_id)
                .ok_or_else(|| HarnessError::StateTransition {
                    message: "missing turn".to_owned(),
                })?;
        rec.recorded_steps.push(output.step_payload.clone());
        Ok(())
    }

    fn complete_turn(
        &mut self,
        work_item_id: &WorkItemId,
        answer: FinalAnswerDraft,
        _attempt_id: AttemptId,
        _now_secs: u64,
    ) -> Result<(), HarnessError> {
        let rec =
            self.turns
                .get_mut(work_item_id)
                .ok_or_else(|| HarnessError::StateTransition {
                    message: "missing turn".to_owned(),
                })?;
        rec.completed_answer = Some(answer);
        Ok(())
    }

    fn record_attempt_failure(
        &mut self,
        work_item_id: &WorkItemId,
        _now_secs: u64,
    ) -> Result<u32, HarnessError> {
        let rec =
            self.turns
                .get_mut(work_item_id)
                .ok_or_else(|| HarnessError::StateTransition {
                    message: "missing turn".to_owned(),
                })?;
        rec.context.failed_attempts = rec.context.failed_attempts.saturating_add(1);
        Ok(rec.context.failed_attempts)
    }
}

struct StubSearchTool;

impl AgentTool for StubSearchTool {
    fn name(&self) -> &'static str {
        "search_chart"
    }

    fn execute(&self, call: &ToolCall) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput {
            call_id: call.call_id.clone(),
            tool_name: self.name().to_owned(),
            step_payload: serde_json::json!({ "kind": "search", "args": call.arguments }),
            cited_sources: Vec::new(),
            summary: "search completed".to_owned(),
        })
    }
}

struct StubScopedReadTool {
    allowed_source: SourceKey,
}

impl AgentTool for StubScopedReadTool {
    fn name(&self) -> &'static str {
        "read_records"
    }

    fn execute(&self, call: &ToolCall) -> Result<ToolOutput, ToolError> {
        let keys = call
            .arguments
            .get("source_keys")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| ToolError::InvalidArguments {
                message: "missing source_keys".to_owned(),
            })?;
        let mut verified = Vec::new();
        for item in keys {
            let raw = item.as_str().ok_or_else(|| ToolError::InvalidArguments {
                message: "non-string source_key".to_owned(),
            })?;
            let key = SourceKey::new(raw).map_err(|e| ToolError::InvalidArguments {
                message: e.to_string(),
            })?;
            if key != self.allowed_source {
                return Err(ToolError::AccessDenied {
                    message: format!("source '{}' outside workspace scope", key.as_str()),
                });
            }
            verified.push(key);
        }
        Ok(ToolOutput {
            call_id: call.call_id.clone(),
            tool_name: self.name().to_owned(),
            step_payload: serde_json::json!({ "kind": "read", "sources": verified }),
            cited_sources: verified,
            summary: "records read".to_owned(),
        })
    }
}

#[test]
fn neutral_harness_runs_two_rounds_streams_answer_and_emits_content_free_telemetry() {
    let ws = WorkspaceId::new("ws-neutral-1").expect("ws");
    let chat = ChatId::new("chat-1").expect("chat");
    let wi = WorkItemId::new("wi-1").expect("wi");
    let src = SourceKey::new("src-1").expect("src");

    let mut adapter = InMemoryTurnAdapter::default();
    adapter.insert_turn(ws, chat, wi.clone(), "Confidential prompt prose");

    let search = StubSearchTool;
    let read = StubScopedReadTool {
        allowed_source: src.clone(),
    };
    let tools: [&dyn AgentTool; 2] = [&search, &read];

    let mut stub = StubProvider::clinical_two_round_fixture(src);
    let mut agent = AgentLoop::new();
    let cancel = CancellationToken::new();

    let outcome = agent
        .run_turn_attempt(&mut adapter, &wi, &mut stub, &tools, &cancel, 100)
        .expect("turn succeeds");
    assert!(matches!(outcome, TurnExecutionOutcome::Finished { .. }));
    assert_eq!(stub.recorded_requests_jsonl().len(), 3);

    let rec = adapter.turns.get(&wi).expect("record");
    assert_eq!(rec.recorded_steps.len(), 2);
    assert!(rec.completed_answer.is_some());
    assert!(rec.deltas.contains("Synthetic renal clearance"));

    // Telemetry spans must NEVER carry prompt or answer text!
    let telemetry_json = serde_json::to_string(agent.telemetry_spans()).expect("serialize spans");
    assert_eq!(agent.telemetry_spans().len(), 3);
    assert!(!telemetry_json.contains("Confidential prompt prose"));
    assert!(!telemetry_json.contains("Synthetic renal clearance"));
}

#[test]
fn neutral_harness_handles_cancel_5_failure_exhaustion_and_access_denial() {
    let ws = WorkspaceId::new("ws-neutral-2").expect("ws");
    let chat = ChatId::new("chat-2").expect("chat");
    let allowed = SourceKey::new("src-allowed").expect("allowed");
    let forbidden = SourceKey::new("src-forbidden").expect("forbidden");

    let search = StubSearchTool;
    let read = StubScopedReadTool {
        allowed_source: allowed.clone(),
    };
    let tools: [&dyn AgentTool; 2] = [&search, &read];
    let mut adapter = InMemoryTurnAdapter::default();
    let mut agent = AgentLoop::new();

    // 1. Mid-stream cancellation keeps partial deltas.
    let wi_cancel = WorkItemId::new("wi-cancel").expect("wi");
    adapter.insert_turn(ws.clone(), chat.clone(), wi_cancel.clone(), "Cancel me");
    let mut cancel_stub = StubProvider::clinical_two_round_fixture(allowed.clone());
    cancel_stub.cancel_after_chunks = Some(2);
    let cancel_tok = CancellationToken::new();
    let res = agent
        .run_turn_attempt(
            &mut adapter,
            &wi_cancel,
            &mut cancel_stub,
            &tools,
            &cancel_tok,
            200,
        )
        .expect("cancel outcome");
    assert_eq!(res, TurnExecutionOutcome::StoppedMidStream);
    assert!(
        adapter.turns[&wi_cancel]
            .deltas
            .contains("Synthetic renal clearance remains stable")
    );

    // 2. Five transient failures exhaust retry budget; 6th attempt succeeds.
    let wi_retry = WorkItemId::new("wi-retry").expect("wi");
    adapter.insert_turn(ws.clone(), chat.clone(), wi_retry.clone(), "Retry me");
    let mut fail_stub = StubProvider::clinical_two_round_fixture(allowed);
    fail_stub.remaining_failures = DEFAULT_MAX_FAILURE_ATTEMPTS;
    let fresh_cancel = CancellationToken::new();

    for expected in 1..DEFAULT_MAX_FAILURE_ATTEMPTS {
        let out = agent
            .run_turn_attempt(
                &mut adapter,
                &wi_retry,
                &mut fail_stub,
                &tools,
                &fresh_cancel,
                300,
            )
            .expect("retryable");
        assert_eq!(
            out,
            TurnExecutionOutcome::ScheduledRetry {
                failed_attempts: expected
            }
        );
    }
    let fifth = agent
        .run_turn_attempt(
            &mut adapter,
            &wi_retry,
            &mut fail_stub,
            &tools,
            &fresh_cancel,
            305,
        )
        .expect("exhausted");
    assert_eq!(
        fifth,
        TurnExecutionOutcome::ExhaustedCouldntFinish {
            failed_attempts: DEFAULT_MAX_FAILURE_ATTEMPTS
        }
    );
    let sixth = agent
        .run_turn_attempt(
            &mut adapter,
            &wi_retry,
            &mut fail_stub,
            &tools,
            &fresh_cancel,
            310,
        )
        .expect("recovers on 6th");
    assert!(matches!(sixth, TurnExecutionOutcome::Finished { .. }));

    // 3. Tool access denial fails closed.
    let wi_denied = WorkItemId::new("wi-denied").expect("wi");
    adapter.insert_turn(ws, chat, wi_denied.clone(), "Cross-scope read");
    let mut bad_stub = StubProvider::clinical_two_round_fixture(forbidden);
    let err = agent
        .run_turn_attempt(
            &mut adapter,
            &wi_denied,
            &mut bad_stub,
            &tools,
            &fresh_cancel,
            400,
        )
        .expect_err("must fail closed");
    assert!(matches!(
        err,
        HarnessError::Tool(ToolError::AccessDenied { .. })
    ));
}
