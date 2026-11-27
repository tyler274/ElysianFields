//! Product-neutral headless agent execution harness, `ModelProvider`, `AgentTool`,
//! and `TurnStateAdapter` traits, deterministic `StubProvider` fixture, and
//! allowlisted content-free telemetry spans.
//!
//! Strictly free of product dependencies (`ef-clinic-*`, `ef-mobile-*`), persistence
//! crates, and rendering engine dependencies (`bevy`).

use std::sync::atomic::{AtomicBool, Ordering};

use ef_core::{AttemptId, ChatId, ResponseId, SourceKey, WorkItemId, WorkspaceId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Default maximum consecutive transient failures before transitioning to `CouldntFinish`.
pub const DEFAULT_MAX_FAILURE_ATTEMPTS: u32 = 5;

/// Structured request passed to a [`ModelProvider`] for one execution round.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRequest {
    pub workspace_id: WorkspaceId,
    pub chat_id: ChatId,
    pub work_item_id: WorkItemId,
    pub round: u32,
    pub attempt: u32,
    pub prompt_text: String,
    pub tool_results: Vec<ToolOutput>,
}

/// Deterministic tool call emitted by a model round (`fixed-<round>-<index>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub call_id: String,
    pub tool_name: String,
    pub workspace_id: WorkspaceId,
    pub arguments: serde_json::Value,
}

/// Result returned by executing an [`AgentTool`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolOutput {
    pub call_id: String,
    pub tool_name: String,
    /// Product-owned structured step payload recorded by the [`TurnStateAdapter`].
    pub step_payload: serde_json::Value,
    pub cited_sources: Vec<SourceKey>,
    pub summary: String,
}

/// Neutral citation reference returned in a completed [`FinalAnswerDraft`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CitedPassage {
    pub source_key: SourceKey,
    pub number: u16,
    pub label: String,
    pub quote: String,
}

/// Product-neutral final answer payload produced by a [`ModelProvider`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinalAnswerDraft {
    pub response_id: ResponseId,
    pub text: String,
    pub citations: Vec<CitedPassage>,
}

/// Outcome returned by a single [`ModelProvider::execute_round`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderRoundOutcome {
    /// Model requested one or more tool calls before generating final prose.
    ToolCalls(Vec<ToolCall>),
    /// Model finished streaming its answer with verified inline citations.
    FinalAnswer(FinalAnswerDraft),
    /// Stream was cancelled mid-flight; partial chunks already delivered remain kept.
    Cancelled,
}

/// Typed provider failure distinguishing retryable transient faults from non-retryable rejections.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProviderError {
    #[error("transient provider failure (HTTP 5xx / mid-stream drop): {message}")]
    Transient { message: String },
    #[error("non-retryable provider rejection: {message}")]
    NonRetryableRejection { message: String },
}

/// Typed error returned by [`AgentTool::execute`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ToolError {
    #[error("access denied for tool call: {message}")]
    AccessDenied { message: String },
    #[error("invalid tool arguments: {message}")]
    InvalidArguments { message: String },
    #[error("unknown agent tool '{name}'")]
    UnknownTool { name: String },
}

/// Top-level error returned by [`AgentLoop::run_turn_attempt`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HarnessError {
    #[error("turn state transition failed: {message}")]
    StateTransition { message: String },
    #[error("provider failed: {0}")]
    Provider(#[from] ProviderError),
    #[error("tool execution failed: {0}")]
    Tool(#[from] ToolError),
}

/// Sink receiving incremental streamed chunks during a provider turn.
pub trait StreamSink {
    fn on_delta(&mut self, text_delta: &str) -> Result<(), HarnessError>;
}

/// Cooperative cancellation token checked between rounds and streamed chunks.
#[derive(Debug, Default)]
pub struct CancellationToken {
    cancelled: AtomicBool,
}

impl CancellationToken {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// Model provider abstraction implemented by live providers (in hosted runtime) and [`StubProvider`].
pub trait ModelProvider {
    fn execute_round(
        &mut self,
        request: &ProviderRequest,
        sink: &mut dyn StreamSink,
        cancel: &CancellationToken,
    ) -> Result<ProviderRoundOutcome, ProviderError>;
}

/// Scoped product-neutral agent tool abstraction (e.g., chart search, record read).
pub trait AgentTool {
    fn name(&self) -> &'static str;
    fn execute(&self, call: &ToolCall) -> Result<ToolOutput, ToolError>;
}

/// Snapshot of the active work item supplied by a product's [`TurnStateAdapter`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnContext {
    pub workspace_id: WorkspaceId,
    pub chat_id: ChatId,
    pub work_item_id: WorkItemId,
    pub prompt_text: String,
    pub failed_attempts: u32,
}

/// Product-owned state adapter bridging [`AgentLoop`] to any product's turn engine
/// without coupling `ef-harness` to product domain types.
pub trait TurnStateAdapter {
    fn load_turn_context(&self, work_item_id: &WorkItemId) -> Result<TurnContext, HarnessError>;

    fn append_delta(
        &mut self,
        work_item_id: &WorkItemId,
        text_delta: &str,
        now_secs: u64,
    ) -> Result<(), HarnessError>;

    fn record_tool_output(
        &mut self,
        work_item_id: &WorkItemId,
        output: &ToolOutput,
        now_secs: u64,
    ) -> Result<(), HarnessError>;

    fn complete_turn(
        &mut self,
        work_item_id: &WorkItemId,
        answer: FinalAnswerDraft,
        attempt_id: AttemptId,
        now_secs: u64,
    ) -> Result<(), HarnessError>;

    fn record_attempt_failure(
        &mut self,
        work_item_id: &WorkItemId,
        now_secs: u64,
    ) -> Result<u32, HarnessError>;

    fn max_failure_attempts(&self) -> u32 {
        DEFAULT_MAX_FAILURE_ATTEMPTS
    }
}

/// Allowlisted content-free telemetry span emitted by [`AgentLoop`].
///
/// By construction this struct carries ONLY structured operational metadata and
/// NEVER clinical prompts, patient names, record quotes, or streamed answer text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelemetrySpan {
    pub span_name: &'static str,
    pub workspace_id: WorkspaceId,
    pub work_item_id: WorkItemId,
    pub attempt: u32,
    pub tool_name: Option<String>,
    pub chunk_count: u64,
    pub outcome: &'static str,
}

/// Scripted tool round definition for [`StubProvider`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptedToolRound {
    pub tool_name: String,
    pub arguments: serde_json::Value,
}

/// Deterministic provider fixture:
/// - Scripted tool rounds with `fixed-<round>-<index>` call IDs
/// - Streamed answer chunks with cooperative mid-stream cancellation
/// - Countdown `remaining_failures` (transient) and `remaining_rejections` (non-retryable)
/// - JSONL request transcript capture for verifier assertions
#[derive(Debug, Clone)]
pub struct StubProvider {
    pub tool_rounds: Vec<Vec<ScriptedToolRound>>,
    pub answer_chunks: Vec<String>,
    pub answer_citations: Vec<(SourceKey, u16, String, String)>,
    pub remaining_failures: u32,
    pub remaining_rejections: u32,
    pub cancel_after_chunks: Option<usize>,
    recorded_requests_jsonl: Vec<String>,
    next_response_seq: u64,
}

impl StubProvider {
    /// Creates a standard 2-tool-round (`search_chart`, `read_records`) + streamed answer fixture.
    #[must_use]
    pub fn clinical_two_round_fixture(cited_source: SourceKey) -> Self {
        Self {
            tool_rounds: vec![
                vec![ScriptedToolRound {
                    tool_name: "search_chart".to_owned(),
                    arguments: serde_json::json!({ "query": "renal panel", "group": "labs", "page": 1 }),
                }],
                vec![ScriptedToolRound {
                    tool_name: "read_records".to_owned(),
                    arguments: serde_json::json!({ "source_keys": [cited_source.as_str()] }),
                }],
            ],
            answer_chunks: vec![
                "Synthetic renal clearance ".to_owned(),
                "remains stable ".to_owned(),
                "within dosing thresholds ".to_owned(),
                "per verified laboratory review [1].".to_owned(),
            ],
            answer_citations: vec![(
                cited_source,
                1,
                "Lab Panel [1]".to_owned(),
                "Creatinine 1.02 mg/dL".to_owned(),
            )],
            remaining_failures: 0,
            remaining_rejections: 0,
            cancel_after_chunks: None,
            recorded_requests_jsonl: Vec::new(),
            next_response_seq: 1,
        }
    }

    /// Returns the captured JSONL transcript of all provider requests.
    #[must_use]
    pub fn recorded_requests_jsonl(&self) -> &[String] {
        &self.recorded_requests_jsonl
    }
}

impl ModelProvider for StubProvider {
    fn execute_round(
        &mut self,
        request: &ProviderRequest,
        sink: &mut dyn StreamSink,
        cancel: &CancellationToken,
    ) -> Result<ProviderRoundOutcome, ProviderError> {
        if let Ok(line) = serde_json::to_string(request) {
            self.recorded_requests_jsonl.push(line);
        }

        if self.remaining_rejections > 0 {
            self.remaining_rejections -= 1;
            return Err(ProviderError::NonRetryableRejection {
                message: "scripted non-retryable tool refusal".to_owned(),
            });
        }

        if self.remaining_failures > 0 {
            self.remaining_failures -= 1;
            return Err(ProviderError::Transient {
                message: "scripted transient 503 upstream failure".to_owned(),
            });
        }

        let round_idx = usize::try_from(request.round.saturating_sub(1)).unwrap_or(usize::MAX);
        if let Some(scripted_calls) = self.tool_rounds.get(round_idx) {
            let calls = scripted_calls
                .iter()
                .enumerate()
                .map(|(idx, spec)| ToolCall {
                    call_id: format!("fixed-{}-{}", request.round, idx + 1),
                    tool_name: spec.tool_name.clone(),
                    workspace_id: request.workspace_id.clone(),
                    arguments: spec.arguments.clone(),
                })
                .collect();
            return Ok(ProviderRoundOutcome::ToolCalls(calls));
        }

        for (idx, chunk) in self.answer_chunks.iter().enumerate() {
            if cancel.is_cancelled() {
                return Ok(ProviderRoundOutcome::Cancelled);
            }
            sink.on_delta(chunk).map_err(|e| ProviderError::Transient {
                message: e.to_string(),
            })?;
            if self.cancel_after_chunks == Some(idx + 1) {
                cancel.cancel();
                return Ok(ProviderRoundOutcome::Cancelled);
            }
        }

        let response_id = ResponseId::new(format!("resp-stub-{}", self.next_response_seq))
            .map_err(|e| ProviderError::Transient {
                message: e.to_string(),
            })?;
        self.next_response_seq = self.next_response_seq.saturating_add(1);

        let citations = self
            .answer_citations
            .iter()
            .map(|(source_key, number, label, quote)| CitedPassage {
                source_key: source_key.clone(),
                number: *number,
                label: label.clone(),
                quote: quote.clone(),
            })
            .collect();

        Ok(ProviderRoundOutcome::FinalAnswer(FinalAnswerDraft {
            response_id,
            text: self.answer_chunks.concat(),
            citations,
        }))
    }
}

/// Outcome of a single [`AgentLoop::run_turn_attempt`] execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnExecutionOutcome {
    Finished { response_id: ResponseId },
    StoppedMidStream,
    ScheduledRetry { failed_attempts: u32 },
    ExhaustedCouldntFinish { failed_attempts: u32 },
    RejectedNonRetryable { message: String },
}

struct AdapterStreamSink<'a> {
    adapter: &'a mut dyn TurnStateAdapter,
    work_item_id: &'a WorkItemId,
    now_secs: u64,
    chunks_emitted: u64,
}

impl StreamSink for AdapterStreamSink<'_> {
    fn on_delta(&mut self, text_delta: &str) -> Result<(), HarnessError> {
        self.adapter
            .append_delta(self.work_item_id, text_delta, self.now_secs)?;
        self.chunks_emitted = self.chunks_emitted.saturating_add(1);
        Ok(())
    }
}

/// Headless agent execution loop driving tool calls, incremental streaming, cancel,
/// bounded retry exhaustion, and content-free telemetry spans.
#[derive(Debug, Default)]
pub struct AgentLoop {
    telemetry: Vec<TelemetrySpan>,
}

impl AgentLoop {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            telemetry: Vec::new(),
        }
    }

    /// Executes one attempt for a claimed `work_item_id` against `adapter`, `provider`, and `tools`.
    pub fn run_turn_attempt(
        &mut self,
        adapter: &mut dyn TurnStateAdapter,
        work_item_id: &WorkItemId,
        provider: &mut dyn ModelProvider,
        tools: &[&dyn AgentTool],
        cancel: &CancellationToken,
        now_secs: u64,
    ) -> Result<TurnExecutionOutcome, HarnessError> {
        let context = adapter.load_turn_context(work_item_id)?;
        let attempt_num = context.failed_attempts.saturating_add(1);
        let max_failures = adapter.max_failure_attempts().max(1);
        let mut tool_results = Vec::new();
        let mut total_chunks = 0u64;

        for round in 1..=8u32 {
            let request = ProviderRequest {
                workspace_id: context.workspace_id.clone(),
                chat_id: context.chat_id.clone(),
                work_item_id: work_item_id.clone(),
                round,
                attempt: attempt_num,
                prompt_text: context.prompt_text.clone(),
                tool_results: tool_results.clone(),
            };

            let outcome = {
                let mut sink = AdapterStreamSink {
                    adapter,
                    work_item_id,
                    now_secs,
                    chunks_emitted: 0,
                };
                let res = provider.execute_round(&request, &mut sink, cancel);
                total_chunks = total_chunks.saturating_add(sink.chunks_emitted);
                res
            };

            match outcome {
                Ok(ProviderRoundOutcome::ToolCalls(calls)) => {
                    for call in calls {
                        let tool = tools
                            .iter()
                            .copied()
                            .find(|t| t.name() == call.tool_name)
                            .ok_or_else(|| ToolError::UnknownTool {
                                name: call.tool_name.clone(),
                            })?;
                        let output = tool.execute(&call)?;
                        adapter.record_tool_output(work_item_id, &output, now_secs)?;
                        self.telemetry.push(TelemetrySpan {
                            span_name: "harness.tool.execute",
                            workspace_id: context.workspace_id.clone(),
                            work_item_id: work_item_id.clone(),
                            attempt: attempt_num,
                            tool_name: Some(call.tool_name),
                            chunk_count: 0,
                            outcome: "ok",
                        });
                        tool_results.push(output);
                    }
                }
                Ok(ProviderRoundOutcome::FinalAnswer(answer)) => {
                    let response_id = answer.response_id.clone();
                    let attempt_id = AttemptId::new(format!("att-{attempt_num}")).map_err(|e| {
                        HarnessError::StateTransition {
                            message: e.to_string(),
                        }
                    })?;
                    adapter.complete_turn(work_item_id, answer, attempt_id, now_secs)?;
                    self.telemetry.push(TelemetrySpan {
                        span_name: "harness.turn.complete",
                        workspace_id: context.workspace_id,
                        work_item_id: work_item_id.clone(),
                        attempt: attempt_num,
                        tool_name: None,
                        chunk_count: total_chunks,
                        outcome: "finished",
                    });
                    return Ok(TurnExecutionOutcome::Finished { response_id });
                }
                Ok(ProviderRoundOutcome::Cancelled) => {
                    self.telemetry.push(TelemetrySpan {
                        span_name: "harness.turn.cancelled",
                        workspace_id: context.workspace_id,
                        work_item_id: work_item_id.clone(),
                        attempt: attempt_num,
                        tool_name: None,
                        chunk_count: total_chunks,
                        outcome: "cancelled",
                    });
                    return Ok(TurnExecutionOutcome::StoppedMidStream);
                }
                Err(ProviderError::Transient { .. }) => {
                    let failures = adapter.record_attempt_failure(work_item_id, now_secs)?;
                    let exhausted = failures >= max_failures;
                    self.telemetry.push(TelemetrySpan {
                        span_name: "harness.turn.failure",
                        workspace_id: context.workspace_id,
                        work_item_id: work_item_id.clone(),
                        attempt: attempt_num,
                        tool_name: None,
                        chunk_count: total_chunks,
                        outcome: if exhausted {
                            "couldnt_finish"
                        } else {
                            "retry_scheduled"
                        },
                    });
                    return Ok(if exhausted {
                        TurnExecutionOutcome::ExhaustedCouldntFinish {
                            failed_attempts: failures,
                        }
                    } else {
                        TurnExecutionOutcome::ScheduledRetry {
                            failed_attempts: failures,
                        }
                    });
                }
                Err(ProviderError::NonRetryableRejection { message }) => {
                    self.telemetry.push(TelemetrySpan {
                        span_name: "harness.turn.rejected",
                        workspace_id: context.workspace_id,
                        work_item_id: work_item_id.clone(),
                        attempt: attempt_num,
                        tool_name: None,
                        chunk_count: total_chunks,
                        outcome: "rejected",
                    });
                    return Ok(TurnExecutionOutcome::RejectedNonRetryable { message });
                }
            }
        }

        Err(HarnessError::Provider(
            ProviderError::NonRetryableRejection {
                message: "tool round budget exceeded".to_owned(),
            },
        ))
    }

    /// Allowlisted content-free telemetry spans emitted during execution.
    #[must_use]
    pub fn telemetry_spans(&self) -> &[TelemetrySpan] {
        &self.telemetry
    }
}
