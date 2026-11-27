//! Durable clinical chat command envelope, `AnswerState` & `ExchangeSteps` wire contracts,
//! incremental SSE delta events, worker leases & bounded 5-failure retry, and
//! optimistic-concurrency composer draft autosave (`409 Conflict` resolution).

use std::collections::{BTreeMap, VecDeque};

use ef_core::{
    AttemptId, ChatId, IdempotencyKey, ResponseId, RunId, SourceKey, WorkItemId, WorkspaceId,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::workspace::AnswerSegment;

/// Default worker lease duration: `60 s`.
pub const WORKER_LEASE_SECS: u64 = 60;

/// Worker stall reclaim threshold: `300 s`.
pub const WORKER_STALL_RECLAIM_SECS: u64 = 300;

/// Hosted exponential backoff initial delay: `10 s`.
pub const INITIAL_RETRY_BACKOFF_SECS: u64 = 10;

/// Hosted exponential backoff ceiling: `600 s`.
pub const MAX_RETRY_BACKOFF_SECS: u64 = 600;

/// Maximum automatic retry failures before permanent transition to `CouldntFinish`.
pub const MAX_FAILURE_ATTEMPTS: u32 = 5;

/// SSE keep-alive comment interval: `15 s`.
pub const SSE_KEEP_ALIVE_SECS: u64 = 15;

/// Client SSE silence timeout before abort and reconnect: `30 s` past keep-alive.
pub const SSE_SILENCE_RECONNECT_SECS: u64 = 30;

/// Message delivery preference for `message.submit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    SendNow,
    SendNext,
}

/// Payload for `message.submit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitMessagePayload {
    pub patient_workspace_id: WorkspaceId,
    pub chat_id: ChatId,
    pub text: String,
    #[serde(default = "default_utc")]
    pub physician_time_zone: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<DeliveryMode>,
}

fn default_utc() -> String {
    "UTC".to_owned()
}

/// Payload for `work.stop`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StopWorkPayload {
    pub patient_workspace_id: WorkspaceId,
    pub chat_id: ChatId,
    pub work_item_id: WorkItemId,
}

/// Payload for `work.resume`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResumeWorkPayload {
    pub patient_workspace_id: WorkspaceId,
    pub chat_id: ChatId,
    pub work_item_id: WorkItemId,
    pub expected_run_id: RunId,
}

/// Typed command variant inside [`CommandEnvelope`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandPayload {
    MessageSubmit(SubmitMessagePayload),
    WorkStop(StopWorkPayload),
    WorkResume(ResumeWorkPayload),
}

/// Wire-compatible command envelope matching `contracts/chat-command-and-answer.schema.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandEnvelope {
    pub idempotency_key: IdempotencyKey,
    pub command: CommandPayload,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCommandEnvelope {
    #[serde(rename = "type")]
    kind: String,
    idempotency_key: IdempotencyKey,
    payload: serde_json::Value,
}

impl Serialize for CommandEnvelope {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let (kind, payload_val) = match &self.command {
            CommandPayload::MessageSubmit(p) => (
                "message.submit".to_owned(),
                serde_json::to_value(p).map_err(serde::ser::Error::custom)?,
            ),
            CommandPayload::WorkStop(p) => (
                "work.stop".to_owned(),
                serde_json::to_value(p).map_err(serde::ser::Error::custom)?,
            ),
            CommandPayload::WorkResume(p) => (
                "work.resume".to_owned(),
                serde_json::to_value(p).map_err(serde::ser::Error::custom)?,
            ),
        };
        RawCommandEnvelope {
            kind,
            idempotency_key: self.idempotency_key.clone(),
            payload: payload_val,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CommandEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawCommandEnvelope::deserialize(deserializer)?;
        let command = match raw.kind.as_str() {
            "message.submit" => CommandPayload::MessageSubmit(
                serde_json::from_value(raw.payload).map_err(serde::de::Error::custom)?,
            ),
            "work.stop" => CommandPayload::WorkStop(
                serde_json::from_value(raw.payload).map_err(serde::de::Error::custom)?,
            ),
            "work.resume" => CommandPayload::WorkResume(
                serde_json::from_value(raw.payload).map_err(serde::de::Error::custom)?,
            ),
            other => {
                return Err(serde::de::Error::custom(format!(
                    "unknown command type '{other}'"
                )));
            }
        };
        Ok(Self {
            idempotency_key: raw.idempotency_key,
            command,
        })
    }
}

/// Handle exposed by `AnswerState::Working` to stop an active exchange.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StopHandle {
    pub work_item_id: WorkItemId,
}

/// Fenced handle exposed by `Stopped`, `CutOff`, and `CouldntFinish` for `work.resume` / `Try again`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResumeHandle {
    pub work_item_id: WorkItemId,
    pub expected_run_id: RunId,
}

/// Wire-compatible `AnswerState` matching `contracts/chat-command-and-answer.schema.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AnswerState {
    Working {
        stop: StopHandle,
        writing: bool,
    },
    Finished {
        response_id: ResponseId,
        attempt_id: AttemptId,
        finished_at: String,
        text: String,
        segments: Vec<AnswerSegment>,
    },
    Stopped {
        resume: Option<ResumeHandle>,
    },
    CutOff {
        try_again: Option<ResumeHandle>,
    },
    CouldntFinish {
        try_again: Option<ResumeHandle>,
    },
}

/// Execution state for an individual tool/evidence step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepExecutionState {
    InProgress,
    Completed,
    Failed,
}

/// Ordered clinical tool/evidence step kinds surfaced during and after streaming.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExchangeStep {
    SearchedChart {
        query: String,
        group: String,
        page: u32,
        state: StepExecutionState,
    },
    ReadRecords {
        requested: Vec<SourceKey>,
        state: StepExecutionState,
    },
    ReadConversation {
        state: StepExecutionState,
    },
    SavedMemoryNote {
        state: StepExecutionState,
    },
    EditedMemory {
        state: StepExecutionState,
    },
    CreatedFollowUp {
        state: StepExecutionState,
    },
}

/// Explicit tagged step trace (`Read` vs `Unavailable` — never silently dropped).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExchangeSteps {
    Read { steps: Vec<ExchangeStep> },
    Unavailable,
}

/// Incremental SSE event delivered over `GET /patient-workspaces/events`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ChatStreamEvent {
    DeltaChunk {
        event_id: u64,
        work_item_id: WorkItemId,
        seq: u64,
        text_delta: String,
    },
    StepRecorded {
        event_id: u64,
        work_item_id: WorkItemId,
        step: ExchangeStep,
    },
    Completed {
        event_id: u64,
        work_item_id: WorkItemId,
        response_id: ResponseId,
    },
    Cancelled {
        event_id: u64,
        work_item_id: WorkItemId,
        expected_run_id: RunId,
    },
    CouldntFinish {
        event_id: u64,
        work_item_id: WorkItemId,
        expected_run_id: RunId,
        attempts: u32,
    },
    ResyncRequired {
        event_id: u64,
        reason: String,
    },
    KeepAlive,
}

/// Retry timing policy (`Hosted` enforces 10s..600s doubling; `DeterministicTest` allows 0s).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryBackoffMode {
    Hosted,
    DeterministicTest,
}

/// Computes bounded retry backoff in seconds for failure number `attempt_number` (`1..=5`).
///
/// Hosted mode: `10 s`, `20 s`, `40 s`, `80 s`, `160 s`, ..., capped at `600 s`.
#[must_use]
pub const fn compute_retry_backoff_secs(attempt_number: u32, mode: RetryBackoffMode) -> u64 {
    match mode {
        RetryBackoffMode::DeterministicTest => 0,
        RetryBackoffMode::Hosted => {
            let raw_shift = attempt_number.saturating_sub(1);
            let shift = if raw_shift > 10 { 10 } else { raw_shift };
            let raw = INITIAL_RETRY_BACKOFF_SECS.saturating_mul(1u64 << shift);
            if raw > MAX_RETRY_BACKOFF_SECS {
                MAX_RETRY_BACKOFF_SECS
            } else {
                raw
            }
        }
    }
}

/// Active worker lease protecting single-claim execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkLease {
    pub worker_id: String,
    pub claimed_at: u64,
    pub lease_expires_at: u64,
    pub last_progress_at: u64,
}

impl WorkLease {
    /// Returns true if the lease expired (`60 s`) or worker stalled past `300 s`.
    #[must_use]
    pub const fn is_reclaimable_at(&self, now_secs: u64) -> bool {
        now_secs >= self.lease_expires_at
            || now_secs
                >= self
                    .last_progress_at
                    .saturating_add(WORKER_STALL_RECLAIM_SECS)
    }
}

/// Durable exchange record stored in PostgreSQL / authoritative chat snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExchangeRecord {
    pub work_item_id: WorkItemId,
    pub workspace_id: WorkspaceId,
    pub chat_id: ChatId,
    pub current_run_id: RunId,
    pub prompt_text: String,
    pub kept_partial_text: String,
    pub steps: ExchangeSteps,
    pub answer: AnswerState,
    pub failed_attempts: u32,
    pub next_retry_at: Option<u64>,
    pub lease: Option<WorkLease>,
    pub frozen_recipe_version: String,
    pub next_delta_seq: u64,
}

/// Outcome returned for an accepted `POST /commands` call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandAcceptance {
    pub work_item_id: WorkItemId,
    pub deduplicated: bool,
    pub returned_messages: Vec<String>,
}

/// Errors returned by `ChatEngine::execute_command`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ChatCommandError {
    #[error("validation failed: {message}")]
    Rejected { message: String },
    #[error("conflict: {message}")]
    Conflict { message: String },
    #[error("work item '{work_item_id}' not found")]
    NotFound { work_item_id: String },
}

impl ChatCommandError {
    #[must_use]
    pub const fn http_status_code(&self) -> u16 {
        match self {
            Self::Rejected { .. } => 400,
            Self::Conflict { .. } => 409,
            Self::NotFound { .. } => 404,
        }
    }
}

/// Durable clinical chat engine enforcing idempotency, cancel/resume fencing,
/// bounded 5-failure retry, worker lease reclaim, and incremental SSE emission.
#[derive(Debug, Clone)]
pub struct ChatEngine {
    retry_mode: RetryBackoffMode,
    exchanges: BTreeMap<WorkItemId, ExchangeRecord>,
    exchange_order: Vec<WorkItemId>,
    queued_follow_ups: VecDeque<String>,
    idempotency_receipts: BTreeMap<IdempotencyKey, CommandAcceptance>,
    event_log: Vec<ChatStreamEvent>,
    next_work_seq: u64,
    next_run_seq: u64,
    next_event_id: u64,
}

impl ChatEngine {
    #[must_use]
    pub fn new(retry_mode: RetryBackoffMode) -> Self {
        Self {
            retry_mode,
            exchanges: BTreeMap::new(),
            exchange_order: Vec::new(),
            queued_follow_ups: VecDeque::new(),
            idempotency_receipts: BTreeMap::new(),
            event_log: Vec::new(),
            next_work_seq: 1,
            next_run_seq: 1,
            next_event_id: 1,
        }
    }

    /// Executes a typed `CommandEnvelope` with full idempotency and conflict fencing.
    pub fn execute_command(
        &mut self,
        envelope: CommandEnvelope,
        now_secs: u64,
    ) -> Result<CommandAcceptance, ChatCommandError> {
        if let Some(existing) = self.idempotency_receipts.get(&envelope.idempotency_key) {
            let mut replay = existing.clone();
            replay.deduplicated = true;
            return Ok(replay);
        }

        let acceptance = match envelope.command {
            CommandPayload::MessageSubmit(payload) => self.handle_submit(payload, now_secs)?,
            CommandPayload::WorkStop(payload) => self.handle_stop(payload)?,
            CommandPayload::WorkResume(payload) => self.handle_resume(payload, now_secs)?,
        };

        self.idempotency_receipts
            .insert(envelope.idempotency_key, acceptance.clone());
        Ok(acceptance)
    }

    fn handle_submit(
        &mut self,
        payload: SubmitMessagePayload,
        now_secs: u64,
    ) -> Result<CommandAcceptance, ChatCommandError> {
        if payload.text.trim().is_empty() {
            return Err(ChatCommandError::Rejected {
                message: "message text must not be empty".to_owned(),
            });
        }

        // Queue if delivery == SendNext and another exchange is active.
        if payload.delivery == Some(DeliveryMode::SendNext)
            && let Some(active_id) = self.active_work_item_id()
        {
            self.queued_follow_ups.push_back(payload.text);
            return Ok(CommandAcceptance {
                work_item_id: active_id,
                deduplicated: false,
                returned_messages: Vec::new(),
            });
        }

        // Submitting a newer message supersedes earlier turns' resume / try_again handles.
        for exchange in self.exchanges.values_mut() {
            match &mut exchange.answer {
                AnswerState::Stopped { resume } => *resume = None,
                AnswerState::CutOff { try_again } | AnswerState::CouldntFinish { try_again } => {
                    *try_again = None;
                }
                AnswerState::Working { .. } | AnswerState::Finished { .. } => {}
            }
        }

        let work_item_id = WorkItemId::new(format!("wi-{}", self.next_work_seq)).map_err(|e| {
            ChatCommandError::Rejected {
                message: e.to_string(),
            }
        })?;
        self.next_work_seq = self.next_work_seq.saturating_add(1);
        let run_id = self.mint_run_id()?;

        let record = ExchangeRecord {
            work_item_id: work_item_id.clone(),
            workspace_id: payload.patient_workspace_id,
            chat_id: payload.chat_id,
            current_run_id: run_id,
            prompt_text: payload.text,
            kept_partial_text: String::new(),
            steps: ExchangeSteps::Read { steps: Vec::new() },
            answer: AnswerState::Working {
                stop: StopHandle {
                    work_item_id: work_item_id.clone(),
                },
                writing: false,
            },
            failed_attempts: 0,
            next_retry_at: Some(now_secs),
            lease: None,
            frozen_recipe_version: "m2-clinic-v1".to_owned(),
            next_delta_seq: 1,
        };
        self.exchanges.insert(work_item_id.clone(), record);
        self.exchange_order.push(work_item_id.clone());

        Ok(CommandAcceptance {
            work_item_id,
            deduplicated: false,
            returned_messages: Vec::new(),
        })
    }

    fn handle_stop(
        &mut self,
        payload: StopWorkPayload,
    ) -> Result<CommandAcceptance, ChatCommandError> {
        let new_run_id = self.mint_run_id()?;
        let Some(exchange) = self.exchanges.get_mut(&payload.work_item_id) else {
            return Err(ChatCommandError::NotFound {
                work_item_id: payload.work_item_id.as_str().to_owned(),
            });
        };
        if !matches!(exchange.answer, AnswerState::Working { .. }) {
            return Err(ChatCommandError::Conflict {
                message: "no matching active turn to stop".to_owned(),
            });
        }

        exchange.current_run_id = new_run_id.clone();
        exchange.lease = None;
        exchange.next_retry_at = None;
        exchange.answer = AnswerState::Stopped {
            resume: Some(ResumeHandle {
                work_item_id: payload.work_item_id.clone(),
                expected_run_id: new_run_id.clone(),
            }),
        };

        let returned_messages: Vec<String> = self.queued_follow_ups.drain(..).collect();
        let event_id = self.alloc_event_id();
        self.event_log.push(ChatStreamEvent::Cancelled {
            event_id,
            work_item_id: payload.work_item_id.clone(),
            expected_run_id: new_run_id,
        });

        Ok(CommandAcceptance {
            work_item_id: payload.work_item_id,
            deduplicated: false,
            returned_messages,
        })
    }

    fn handle_resume(
        &mut self,
        payload: ResumeWorkPayload,
        now_secs: u64,
    ) -> Result<CommandAcceptance, ChatCommandError> {
        let next_run_id = self.mint_run_id()?;
        let Some(exchange) = self.exchanges.get_mut(&payload.work_item_id) else {
            return Err(ChatCommandError::NotFound {
                work_item_id: payload.work_item_id.as_str().to_owned(),
            });
        };

        let valid_handle = match &exchange.answer {
            AnswerState::Stopped {
                resume: Some(handle),
            }
            | AnswerState::CutOff {
                try_again: Some(handle),
            }
            | AnswerState::CouldntFinish {
                try_again: Some(handle),
            } => handle.expected_run_id == payload.expected_run_id,
            _ => false,
        };

        if !valid_handle {
            return Err(ChatCommandError::Conflict {
                message: "This turn can no longer be resumed.".to_owned(),
            });
        }

        exchange.current_run_id = next_run_id;
        exchange.next_retry_at = Some(now_secs);
        exchange.lease = None;
        exchange.answer = AnswerState::Working {
            stop: StopHandle {
                work_item_id: payload.work_item_id.clone(),
            },
            writing: false,
        };

        Ok(CommandAcceptance {
            work_item_id: payload.work_item_id,
            deduplicated: false,
            returned_messages: Vec::new(),
        })
    }

    /// Claims a due `Working` exchange via a `60 s` lease (or reclaims an expired/stalled lease).
    pub fn claim_due_work(
        &mut self,
        worker_id: &str,
        now_secs: u64,
    ) -> Option<(WorkItemId, RunId)> {
        for work_id in &self.exchange_order {
            let Some(exchange) = self.exchanges.get_mut(work_id) else {
                continue;
            };
            if !matches!(exchange.answer, AnswerState::Working { .. }) {
                continue;
            }
            if let Some(due_at) = exchange.next_retry_at
                && now_secs < due_at
            {
                continue;
            }
            let can_claim = match &exchange.lease {
                None => true,
                Some(lease) => lease.is_reclaimable_at(now_secs),
            };
            if can_claim {
                exchange.lease = Some(WorkLease {
                    worker_id: worker_id.to_owned(),
                    claimed_at: now_secs,
                    lease_expires_at: now_secs.saturating_add(WORKER_LEASE_SECS),
                    last_progress_at: now_secs,
                });
                return Some((
                    exchange.work_item_id.clone(),
                    exchange.current_run_id.clone(),
                ));
            }
        }
        None
    }

    /// Records an ordered clinical tool/evidence step onto a working exchange.
    pub fn record_step(
        &mut self,
        work_item_id: &WorkItemId,
        step: ExchangeStep,
        now_secs: u64,
    ) -> Result<(), ChatCommandError> {
        let event_id = self.alloc_event_id();
        let Some(exchange) = self.exchanges.get_mut(work_item_id) else {
            return Err(ChatCommandError::NotFound {
                work_item_id: work_item_id.as_str().to_owned(),
            });
        };
        if let ExchangeSteps::Read { steps } = &mut exchange.steps {
            steps.push(step.clone());
        }
        if let Some(lease) = &mut exchange.lease {
            lease.last_progress_at = now_secs;
            lease.lease_expires_at = now_secs.saturating_add(WORKER_LEASE_SECS);
        }
        self.event_log.push(ChatStreamEvent::StepRecorded {
            event_id,
            work_item_id: work_item_id.clone(),
            step,
        });
        Ok(())
    }

    /// Appends an incremental streamed text delta to the active turn (O(1) tail append).
    pub fn append_delta(
        &mut self,
        work_item_id: &WorkItemId,
        text_delta: &str,
        now_secs: u64,
    ) -> Result<u64, ChatCommandError> {
        let event_id = self.alloc_event_id();
        let Some(exchange) = self.exchanges.get_mut(work_item_id) else {
            return Err(ChatCommandError::NotFound {
                work_item_id: work_item_id.as_str().to_owned(),
            });
        };
        if let AnswerState::Working { writing, .. } = &mut exchange.answer {
            *writing = true;
        }
        exchange.kept_partial_text.push_str(text_delta);
        let seq = exchange.next_delta_seq;
        exchange.next_delta_seq = seq.saturating_add(1);
        if let Some(lease) = &mut exchange.lease {
            lease.last_progress_at = now_secs;
            lease.lease_expires_at = now_secs.saturating_add(WORKER_LEASE_SECS);
        }
        self.event_log.push(ChatStreamEvent::DeltaChunk {
            event_id,
            work_item_id: work_item_id.clone(),
            seq,
            text_delta: text_delta.to_owned(),
        });
        Ok(seq)
    }

    /// Marks an exchange `Finished` with complete text and inline citation segments.
    pub fn complete_exchange(
        &mut self,
        work_item_id: &WorkItemId,
        response_id: ResponseId,
        attempt_id: AttemptId,
        finished_at: String,
        segments: Vec<AnswerSegment>,
    ) -> Result<(), ChatCommandError> {
        let event_id = self.alloc_event_id();
        let Some(exchange) = self.exchanges.get_mut(work_item_id) else {
            return Err(ChatCommandError::NotFound {
                work_item_id: work_item_id.as_str().to_owned(),
            });
        };
        let full_text = exchange.kept_partial_text.clone();
        exchange.lease = None;
        exchange.next_retry_at = None;
        exchange.answer = AnswerState::Finished {
            response_id: response_id.clone(),
            attempt_id,
            finished_at,
            text: full_text,
            segments,
        };
        self.event_log.push(ChatStreamEvent::Completed {
            event_id,
            work_item_id: work_item_id.clone(),
            response_id,
        });
        Ok(())
    }

    /// Records a counted execution failure and enforces the bounded 5-failure ceiling.
    ///
    /// - Failures `1..=4`: schedules automatic retry with exponential backoff (`10s..600s`).
    /// - Failure `5`: transitions permanently to `CouldntFinish` with explicit `Try again` handle
    ///   and stops all background retries.
    pub fn record_attempt_failure(
        &mut self,
        work_item_id: &WorkItemId,
        now_secs: u64,
    ) -> Result<u32, ChatCommandError> {
        let retry_mode = self.retry_mode;
        let next_run_id = self.mint_run_id()?;
        let event_id = self.alloc_event_id();

        let Some(exchange) = self.exchanges.get_mut(work_item_id) else {
            return Err(ChatCommandError::NotFound {
                work_item_id: work_item_id.as_str().to_owned(),
            });
        };
        exchange.failed_attempts = exchange.failed_attempts.saturating_add(1);
        exchange.lease = None;

        if exchange.failed_attempts >= MAX_FAILURE_ATTEMPTS {
            exchange.current_run_id = next_run_id.clone();
            exchange.next_retry_at = None;
            exchange.answer = AnswerState::CouldntFinish {
                try_again: Some(ResumeHandle {
                    work_item_id: work_item_id.clone(),
                    expected_run_id: next_run_id.clone(),
                }),
            };
            self.event_log.push(ChatStreamEvent::CouldntFinish {
                event_id,
                work_item_id: work_item_id.clone(),
                expected_run_id: next_run_id,
                attempts: exchange.failed_attempts,
            });
        } else {
            let delay = compute_retry_backoff_secs(exchange.failed_attempts, retry_mode);
            exchange.next_retry_at = Some(now_secs.saturating_add(delay));
        }

        Ok(exchange.failed_attempts)
    }

    /// Emits a `ResyncRequired` event (e.g., on lossy Valkey/Redis restart or subscriber lag).
    pub fn emit_resync(&mut self, reason: impl Into<String>) {
        let event_id = self.alloc_event_id();
        self.event_log.push(ChatStreamEvent::ResyncRequired {
            event_id,
            reason: reason.into(),
        });
    }

    /// Looks up a persisted exchange by ID.
    #[must_use]
    pub fn exchange(&self, id: &WorkItemId) -> Option<&ExchangeRecord> {
        self.exchanges.get(id)
    }

    /// Ordered snapshot of all exchanges in this chat.
    #[must_use]
    pub fn transcript_snapshot(&self) -> Vec<&ExchangeRecord> {
        self.exchange_order
            .iter()
            .filter_map(|id| self.exchanges.get(id))
            .collect()
    }

    /// All emitted SSE events.
    #[must_use]
    pub fn events(&self) -> &[ChatStreamEvent] {
        &self.event_log
    }

    fn active_work_item_id(&self) -> Option<WorkItemId> {
        self.exchange_order.iter().rev().find_map(|id| {
            self.exchanges
                .get(id)
                .filter(|ex| matches!(ex.answer, AnswerState::Working { .. }))
                .map(|ex| ex.work_item_id.clone())
        })
    }

    fn mint_run_id(&mut self) -> Result<RunId, ChatCommandError> {
        let id = RunId::new(format!("run-{}", self.next_run_seq)).map_err(|e| {
            ChatCommandError::Rejected {
                message: e.to_string(),
            }
        })?;
        self.next_run_seq = self.next_run_seq.saturating_add(1);
        Ok(id)
    }

    fn alloc_event_id(&mut self) -> u64 {
        let id = self.next_event_id;
        self.next_event_id = self.next_event_id.saturating_add(1);
        id
    }
}

/// Persisted optimistic-concurrency draft snapshot on the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedDraft {
    pub text: String,
    pub revision: u64,
}

/// Result of `PUT /physician-state/drafts/{workspace_id}/{chat_id}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftSaveOutcome {
    Saved(PersistedDraft),
    /// HTTP `409 Conflict` returning the server's newer revision.
    Conflict {
        current: PersistedDraft,
    },
    /// Transient backend/network failure (`Not saved`).
    TransientFailure {
        message: String,
    },
}

/// Server-side optimistic-concurrency draft store.
#[derive(Debug, Clone, Default)]
pub struct DraftStore {
    drafts: BTreeMap<(WorkspaceId, String), PersistedDraft>,
}

impl DraftStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads the current persisted draft (or empty revision 0 if absent).
    #[must_use]
    pub fn get(&self, workspace_id: &WorkspaceId, chat_target: &str) -> PersistedDraft {
        self.drafts
            .get(&(workspace_id.clone(), chat_target.to_owned()))
            .cloned()
            .unwrap_or(PersistedDraft {
                text: String::new(),
                revision: 0,
            })
    }

    /// Saves a draft only if `expected_revision` matches the current stored revision.
    pub fn put(
        &mut self,
        workspace_id: &WorkspaceId,
        chat_target: &str,
        text: String,
        expected_revision: u64,
    ) -> DraftSaveOutcome {
        let key = (workspace_id.clone(), chat_target.to_owned());
        let current = self.drafts.get(&key).cloned().unwrap_or(PersistedDraft {
            text: String::new(),
            revision: 0,
        });

        if current.revision != expected_revision {
            return DraftSaveOutcome::Conflict { current };
        }

        let next = PersistedDraft {
            text,
            revision: current.revision.saturating_add(1),
        };
        self.drafts.insert(key, next.clone());
        DraftSaveOutcome::Saved(next)
    }
}

/// Client-side synchronization status for composer draft autosave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftSyncState {
    Idle,
    Saving {
        in_flight_generation: u64,
        submitted_text: String,
    },
    Conflict {
        server_draft: PersistedDraft,
    },
    NotSaved {
        message: String,
    },
}

/// Single-owner composer draft controller enforcing:
/// - `is_dirty()` derived solely from `local_text != saved_baseline_text`
/// - single in-flight save with generation fencing
/// - `409 Conflict` pausing autosave with explicit `resolve_keep_mine` / `resolve_use_newer`
/// - submit clearing only the submitted prefix while preserving post-Send keystrokes
#[derive(Debug, Clone)]
pub struct ComposerDraftController {
    local_text: String,
    saved_baseline_text: String,
    saved_revision: u64,
    generation: u64,
    sync_state: DraftSyncState,
}

impl ComposerDraftController {
    /// Initializes the controller from a loaded server snapshot.
    #[must_use]
    pub fn from_persisted(draft: PersistedDraft) -> Self {
        Self {
            local_text: draft.text.clone(),
            saved_baseline_text: draft.text,
            saved_revision: draft.revision,
            generation: 0,
            sync_state: DraftSyncState::Idle,
        }
    }

    /// Single owner for unsaved dirty derivation: `local_text != saved_baseline_text`.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.local_text != self.saved_baseline_text
    }

    /// Applies local editing and increments the generation counter.
    pub fn edit(&mut self, new_text: impl Into<String>) {
        self.local_text = new_text.into();
        self.generation = self.generation.saturating_add(1);
        if matches!(self.sync_state, DraftSyncState::NotSaved { .. }) {
            self.sync_state = DraftSyncState::Idle;
        }
    }

    /// Appends text typed by the physician (e.g., after pressing Send).
    pub fn append_typing(&mut self, suffix: &str) {
        self.local_text.push_str(suffix);
        self.generation = self.generation.saturating_add(1);
    }

    /// Starts a single in-flight save if dirty and not blocked on conflict or another save.
    /// Returns `(text_to_save, expected_revision, generation)`.
    pub fn begin_autosave(&mut self) -> Option<(String, u64, u64)> {
        if !self.is_dirty() {
            return None;
        }
        match self.sync_state {
            DraftSyncState::Idle => {
                let text = self.local_text.clone();
                let generation = self.generation;
                self.sync_state = DraftSyncState::Saving {
                    in_flight_generation: generation,
                    submitted_text: text.clone(),
                };
                Some((text, self.saved_revision, generation))
            }
            DraftSyncState::Saving { .. }
            | DraftSyncState::Conflict { .. }
            | DraftSyncState::NotSaved { .. } => None,
        }
    }

    /// Applies the server response for an in-flight save fenced by `request_generation`.
    pub fn finish_autosave(&mut self, request_generation: u64, outcome: DraftSaveOutcome) {
        let matches_in_flight = matches!(
            self.sync_state,
            DraftSyncState::Saving {
                in_flight_generation,
                ..
            } if in_flight_generation == request_generation
        );
        if !matches_in_flight {
            return;
        }

        match outcome {
            DraftSaveOutcome::Saved(saved) => {
                // Generation fencing: NEVER overwrite newer local_text typed while save was in flight.
                self.saved_baseline_text = saved.text;
                self.saved_revision = saved.revision;
                self.sync_state = DraftSyncState::Idle;
            }
            DraftSaveOutcome::Conflict { current } => {
                self.sync_state = DraftSyncState::Conflict {
                    server_draft: current,
                };
            }
            DraftSaveOutcome::TransientFailure { message } => {
                self.sync_state = DraftSyncState::NotSaved { message };
            }
        }
    }

    /// Explicit physician choice **Retry (keep mine)**: adopts server revision so next save writes local text.
    pub fn resolve_keep_mine(&mut self) -> Option<(String, u64, u64)> {
        let server_rev = match &self.sync_state {
            DraftSyncState::Conflict { server_draft } => server_draft.revision,
            _ => return None,
        };
        self.saved_revision = server_rev;
        let text = self.local_text.clone();
        let generation = self.generation;
        self.sync_state = DraftSyncState::Saving {
            in_flight_generation: generation,
            submitted_text: text.clone(),
        };
        Some((text, server_rev, generation))
    }

    /// Explicit physician choice **Use newer text**: replaces local text with server's newer revision.
    pub fn resolve_use_newer(&mut self) -> bool {
        let server = match &self.sync_state {
            DraftSyncState::Conflict { server_draft } => server_draft.clone(),
            _ => return false,
        };
        self.local_text = server.text.clone();
        self.saved_baseline_text = server.text;
        self.saved_revision = server.revision;
        self.generation = self.generation.saturating_add(1);
        self.sync_state = DraftSyncState::Idle;
        true
    }

    /// Clears ONLY the submitted prefix after `message.submit` is accepted, preserving
    /// any text typed by the clinician after pressing Send.
    pub fn clear_submitted_prefix(&mut self, submitted_text: &str) {
        if let Some(remainder) = self.local_text.strip_prefix(submitted_text) {
            self.local_text = remainder.to_owned();
            self.generation = self.generation.saturating_add(1);
        }
    }

    #[must_use]
    pub fn local_text(&self) -> &str {
        &self.local_text
    }

    #[must_use]
    pub const fn saved_revision(&self) -> u64 {
        self.saved_revision
    }

    #[must_use]
    pub const fn sync_state(&self) -> &DraftSyncState {
        &self.sync_state
    }
}
