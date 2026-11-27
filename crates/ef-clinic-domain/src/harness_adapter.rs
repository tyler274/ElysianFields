//! Curie Clinic adapters bridging product-neutral [`ef_harness`] onto [`ChatEngine`]
//! and [`WorkspaceEvidenceStore`] without coupling `ef-harness` to clinic domain types.

use ef_core::{AttemptId, PhysicianId, PracticeId, SourceKey, WorkItemId};
use ef_harness::{
    AgentTool, FinalAnswerDraft, HarnessError, ToolCall, ToolError, ToolOutput, TurnContext,
    TurnStateAdapter,
};

use crate::chat::{ChatEngine, ExchangeStep, MAX_FAILURE_ATTEMPTS, StepExecutionState};
use crate::workspace::{AnswerSegment, CitationTarget, WorkspaceEvidenceStore};

impl TurnStateAdapter for ChatEngine {
    fn load_turn_context(&self, work_item_id: &WorkItemId) -> Result<TurnContext, HarnessError> {
        let Some(exchange) = self.exchange(work_item_id) else {
            return Err(HarnessError::StateTransition {
                message: format!("work item '{}' not found", work_item_id.as_str()),
            });
        };
        Ok(TurnContext {
            workspace_id: exchange.workspace_id.clone(),
            chat_id: exchange.chat_id.clone(),
            work_item_id: work_item_id.clone(),
            prompt_text: exchange.prompt_text.clone(),
            failed_attempts: exchange.failed_attempts,
        })
    }

    fn append_delta(
        &mut self,
        work_item_id: &WorkItemId,
        text_delta: &str,
        now_secs: u64,
    ) -> Result<(), HarnessError> {
        let _seq = self
            .append_delta(work_item_id, text_delta, now_secs)
            .map_err(|err| HarnessError::StateTransition {
                message: err.to_string(),
            })?;
        Ok(())
    }

    fn record_tool_output(
        &mut self,
        work_item_id: &WorkItemId,
        output: &ToolOutput,
        now_secs: u64,
    ) -> Result<(), HarnessError> {
        let step: ExchangeStep =
            serde_json::from_value(output.step_payload.clone()).map_err(|err| {
                HarnessError::StateTransition {
                    message: format!("invalid clinic ExchangeStep payload: {err}"),
                }
            })?;
        self.record_step(work_item_id, step, now_secs)
            .map_err(|err| HarnessError::StateTransition {
                message: err.to_string(),
            })
    }

    fn complete_turn(
        &mut self,
        work_item_id: &WorkItemId,
        answer: FinalAnswerDraft,
        attempt_id: AttemptId,
        now_secs: u64,
    ) -> Result<(), HarnessError> {
        let mut segments = Vec::with_capacity(1 + answer.citations.len());
        segments.push(AnswerSegment::Text { text: answer.text });
        for cited in answer.citations {
            segments.push(AnswerSegment::Citation {
                source_key: cited.source_key,
                number: cited.number,
                label: cited.label,
                quote: cited.quote.clone(),
                target: CitationTarget::Passage { quote: cited.quote },
            });
        }
        self.complete_exchange(
            work_item_id,
            answer.response_id,
            attempt_id,
            format!("epoch-{now_secs}"),
            segments,
        )
        .map_err(|err| HarnessError::StateTransition {
            message: err.to_string(),
        })
    }

    fn record_attempt_failure(
        &mut self,
        work_item_id: &WorkItemId,
        now_secs: u64,
    ) -> Result<u32, HarnessError> {
        self.record_attempt_failure(work_item_id, now_secs)
            .map_err(|err| HarnessError::StateTransition {
                message: err.to_string(),
            })
    }

    fn max_failure_attempts(&self) -> u32 {
        MAX_FAILURE_ATTEMPTS
    }
}

/// Workspace-scoped `search_chart` tool implementation backed by [`WorkspaceEvidenceStore`].
pub struct SearchChartTool<'a> {
    pub practice_id: PracticeId,
    pub physician_id: PhysicianId,
    pub store: &'a WorkspaceEvidenceStore,
}

impl AgentTool for SearchChartTool<'_> {
    fn name(&self) -> &'static str {
        "search_chart"
    }

    fn execute(&self, call: &ToolCall) -> Result<ToolOutput, ToolError> {
        self.store
            .require_clinical_access(&self.practice_id, &self.physician_id, &call.workspace_id)
            .map_err(|err| ToolError::AccessDenied {
                message: err.to_string(),
            })?;
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

        let step = ExchangeStep::SearchedChart {
            query,
            group,
            page,
            state: StepExecutionState::Completed,
        };
        let step_payload =
            serde_json::to_value(&step).map_err(|err| ToolError::InvalidArguments {
                message: format!("failed to serialize search_chart step: {err}"),
            })?;

        Ok(ToolOutput {
            call_id: call.call_id.clone(),
            tool_name: self.name().to_owned(),
            step_payload,
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

impl AgentTool for ReadRecordsTool<'_> {
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
            let _payload = self
                .store
                .fetch_cited_source(
                    &self.practice_id,
                    &self.physician_id,
                    &call.workspace_id,
                    &key,
                )
                .map_err(|err| ToolError::AccessDenied {
                    message: err.to_string(),
                })?;
            verified_keys.push(key);
        }

        let step = ExchangeStep::ReadRecords {
            requested: verified_keys.clone(),
            state: StepExecutionState::Completed,
        };
        let step_payload =
            serde_json::to_value(&step).map_err(|err| ToolError::InvalidArguments {
                message: format!("failed to serialize read_records step: {err}"),
            })?;

        Ok(ToolOutput {
            call_id: call.call_id.clone(),
            tool_name: self.name().to_owned(),
            step_payload,
            cited_sources: verified_keys,
            summary: "verified records read".to_owned(),
        })
    }
}
