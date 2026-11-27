//! Headless clinical agent execution loop, `ModelProvider` and `ClinicalTool` traits,
//! deterministic `StubProvider` fixture (Chapter 3 §3.6), and allowlisted content-free
//! telemetry spans.
//!
//! Strictly free of rendering engine dependencies (`bevy` is forbidden).

use std::sync::atomic::{AtomicBool, Ordering};

use ef_clinic_domain::chat::{
    ChatCommandError, ChatEngine, ExchangeStep, MAX_FAILURE_ATTEMPTS, StepExecutionState,
};
use ef_clinic_domain::workspace::{
    AnswerSegment, CitationTarget, WorkspaceAccessError, WorkspaceEvidenceStore,
};
use ef_core::{
    AttemptId, ChatId, PhysicianId, PracticeId, ResponseId, SourceKey, WorkItemId, WorkspaceId,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

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

/// Result returned by executing a [`ClinicalTool`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolOutput {
    pub call_id: String,
    pub tool_name: String,
    pub step: ExchangeStep,
    pub cited_sources: Vec<SourceKey>,
    pub summary: String,
}

/// Outcome returned by a single [`ModelProvider::execute_round`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderRoundOutcome {
    /// Model requested one or more clinical tool calls before generating final prose.
    ToolCalls(Vec<ToolCall>),
    /// Model finished streaming its clinical answer with verified inline citations.
    FinalAnswer {
        response_id: ResponseId,
        segments: Vec<AnswerSegment>,
    },
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

/// Typed error returned by [`ClinicalTool::execute`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ToolError {
    #[error("workspace access denied for tool call: {0}")]
    Access(#[from] WorkspaceAccessError),
    #[error("invalid tool arguments: {message}")]
    InvalidArguments { message: String },
    #[error("unknown clinical tool '{name}'")]
    UnknownTool { name: String },
}

/// Top-level error returned by [`AgentLoop::run_attempt`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HarnessError {
    #[error("chat state transition failed: {0}")]
    Chat(#[from] ChatCommandError),
    #[error("provider failed: {0}")]
    Provider(#[from] ProviderError),
    #[error("tool execution failed: {0}")]
    Tool(#[from] ToolError),
}

/// Sink receiving incremental streamed chunks during a provider turn.
pub trait StreamSink {
    fn on_delta(&mut self, text_delta: &str) -> Result<(), ChatCommandError>;
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

/// Scoped clinical tool abstraction (e.g., `search_chart`, `read_records`).
pub trait ClinicalTool {
    fn name(&self) -> &'static str;
    fn execute(&self, call: &ToolCall) -> Result<ToolOutput, ToolError>;
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

/// Deterministic provider fixture satisfying Chapter 3 §3.6:
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
    /// Creates a standard 2-tool-round (`search_chart`, `read_records`) + streamed clinical answer fixture.
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

        let mut segments = vec![AnswerSegment::Text {
            text: self.answer_chunks.concat(),
        }];
        for (source_key, number, label, quote) in &self.answer_citations {
            segments.push(AnswerSegment::Citation {
                source_key: source_key.clone(),
                number: *number,
                label: label.clone(),
                quote: quote.clone(),
                target: CitationTarget::Passage {
                    quote: quote.clone(),
                },
            });
        }

        Ok(ProviderRoundOutcome::FinalAnswer {
            response_id,
            segments,
        })
    }
}

/// Workspace-scoped `search_chart` tool implementation backed by [`WorkspaceEvidenceStore`].
pub struct SearchChartTool<'a> {
    pub practice_id: PracticeId,
    pub physician_id: PhysicianId,
    pub store: &'a WorkspaceEvidenceStore,
}

impl ClinicalTool for SearchChartTool<'_> {
    fn name(&self) -> &'static str {
        "search_chart"
    }

    fn execute(&self, call: &ToolCall) -> Result<ToolOutput, ToolError> {
        self.store.require_clinical_access(
            &self.practice_id,
            &self.physician_id,
            &call.workspace_id,
        )?;
        let query = call
            .arguments
            .get("query")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("chart")
            .to_owned();
        let group = call
            .arguments
            .get("group")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("all")
            .to_owned();
        let page = call
            .arguments
            .get("page")
            .and_then(serde_json::Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(1);

        Ok(ToolOutput {
            call_id: call.call_id.clone(),
            tool_name: self.name().to_owned(),
            step: ExchangeStep::SearchedChart {
                query,
                group,
                page,
                state: StepExecutionState::Completed,
            },
            cited_sources: Vec::new(),
            summary: "chart search completed".to_owned(),
        })
    }
}

/// Workspace-scoped `read_records` tool enforcing cross-patient and withdrawn-source isolation.
pub struct ReadRecordsTool<'a> {
    pub practice_id: PracticeId,
    pub physician_id: PhysicianId,
    pub store: &'a WorkspaceEvidenceStore,
}

impl ClinicalTool for ReadRecordsTool<'_> {
    fn name(&self) -> &'static str {
        "read_records"
    }

    fn execute(&self, call: &ToolCall) -> Result<ToolOutput, ToolError> {
        let raw_keys = call
            .arguments
            .get("source_keys")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| ToolError::InvalidArguments {
                message: "missing source_keys array".to_owned(),
            })?;

        let mut verified_keys = Vec::with_capacity(raw_keys.len());
        for raw in raw_keys {
            let key_str = raw.as_str().ok_or_else(|| ToolError::InvalidArguments {
                message: "source_keys entries must be strings".to_owned(),
            })?;
            let key = SourceKey::new(key_str).map_err(|e| ToolError::InvalidArguments {
                message: e.to_string(),
            })?;
            let _payload = self.store.fetch_cited_source(
                &self.practice_id,
                &self.physician_id,
                &call.workspace_id,
                &key,
            )?;
            verified_keys.push(key);
        }

        Ok(ToolOutput {
            call_id: call.call_id.clone(),
            tool_name: self.name().to_owned(),
            step: ExchangeStep::ReadRecords {
                requested: verified_keys.clone(),
                state: StepExecutionState::Completed,
            },
            cited_sources: verified_keys,
            summary: "verified records read".to_owned(),
        })
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

struct EngineStreamSink<'a> {
    engine: &'a mut ChatEngine,
    work_item_id: &'a WorkItemId,
    now_secs: u64,
    chunks_emitted: u64,
}

impl StreamSink for EngineStreamSink<'_> {
    fn on_delta(&mut self, text_delta: &str) -> Result<(), ChatCommandError> {
        let _seq = self
            .engine
            .append_delta(self.work_item_id, text_delta, self.now_secs)?;
        self.chunks_emitted = self.chunks_emitted.saturating_add(1);
        Ok(())
    }
}

/// Headless agent execution loop driving tool calls, incremental streaming, cancel,
/// bounded 5-failure retry exhaustion, and content-free telemetry spans.
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

    /// Executes one attempt for a claimed `work_item_id` against `provider` and `tools`.
    pub fn run_turn_attempt(
        &mut self,
        engine: &mut ChatEngine,
        work_item_id: &WorkItemId,
        provider: &mut dyn ModelProvider,
        tools: &[&dyn ClinicalTool],
        cancel: &CancellationToken,
        now_secs: u64,
    ) -> Result<TurnExecutionOutcome, HarnessError> {
        let Some(exchange) = engine.exchange(work_item_id).cloned() else {
            return Err(HarnessError::Chat(ChatCommandError::NotFound {
                work_item_id: work_item_id.as_str().to_owned(),
            }));
        };

        let attempt_num = exchange.failed_attempts.saturating_add(1);
        let mut tool_results = Vec::new();
        let mut total_chunks = 0u64;

        for round in 1..=8u32 {
            let request = ProviderRequest {
                workspace_id: exchange.workspace_id.clone(),
                chat_id: exchange.chat_id.clone(),
                work_item_id: work_item_id.clone(),
                round,
                attempt: attempt_num,
                prompt_text: exchange.prompt_text.clone(),
                tool_results: tool_results.clone(),
            };

            let outcome = {
                let mut sink = EngineStreamSink {
                    engine,
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
                        engine.record_step(work_item_id, output.step.clone(), now_secs)?;
                        self.telemetry.push(TelemetrySpan {
                            span_name: "harness.tool.execute",
                            workspace_id: exchange.workspace_id.clone(),
                            work_item_id: work_item_id.clone(),
                            attempt: attempt_num,
                            tool_name: Some(call.tool_name),
                            chunk_count: 0,
                            outcome: "ok",
                        });
                        tool_results.push(output);
                    }
                }
                Ok(ProviderRoundOutcome::FinalAnswer {
                    response_id,
                    segments,
                }) => {
                    let attempt_id = AttemptId::new(format!("att-{attempt_num}")).map_err(|e| {
                        HarnessError::Chat(ChatCommandError::Rejected {
                            message: e.to_string(),
                        })
                    })?;
                    engine.complete_exchange(
                        work_item_id,
                        response_id.clone(),
                        attempt_id,
                        format!("epoch-{now_secs}"),
                        segments,
                    )?;
                    self.telemetry.push(TelemetrySpan {
                        span_name: "harness.turn.complete",
                        workspace_id: exchange.workspace_id,
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
                        workspace_id: exchange.workspace_id,
                        work_item_id: work_item_id.clone(),
                        attempt: attempt_num,
                        tool_name: None,
                        chunk_count: total_chunks,
                        outcome: "cancelled",
                    });
                    return Ok(TurnExecutionOutcome::StoppedMidStream);
                }
                Err(ProviderError::Transient { .. }) => {
                    let failures = engine.record_attempt_failure(work_item_id, now_secs)?;
                    let exhausted = failures >= MAX_FAILURE_ATTEMPTS;
                    self.telemetry.push(TelemetrySpan {
                        span_name: "harness.turn.failure",
                        workspace_id: exchange.workspace_id,
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
                        workspace_id: exchange.workspace_id,
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
