//! Durable atomic append-only WAL / JSONL persistence for Curie Clinic M2 state.
//!
//! Every acknowledged mutation (sessions, accounts, workspaces, citations,
//! idempotent commands, steps, streamed deltas, retries, and drafts) is appended
//! as one JSONL frame and `fsync`ed (`sync_all`) before the HTTP response returns.
//! Server restart replays the WAL through the exact same domain owners used on the
//! live path so live and recovery semantics cannot drift apart.

#[cfg(not(target_arch = "wasm32"))]
use std::fs::{self, File, OpenOptions};
#[cfg(not(target_arch = "wasm32"))]
use std::io::{BufRead, BufReader, Write};
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

use ef_clinic_domain::auth::{EmailAddress, PhysicianAccount, VerificationTokenKind};
use ef_clinic_domain::chat::{CommandEnvelope, ExchangeStep};
use ef_clinic_domain::workspace::{AnswerSegment, CitedSourcePayload};
use ef_clinic_workflows::matching::MoveDestination;
use ef_clinic_workflows::memory::{MemoryAuthority, RoundsObservation};
use ef_clinic_workflows::today::{ChannelHealthStatus, ScheduledVisitEntry, TodayQueueItem};
use ef_connected_care::{CreateInvitationRequest, PairKey};
use ef_core::{
    AttemptId, ChatId, PhysicianId, PracticeId, ResponseId, SessionId, SourceKey, WorkItemId,
    WorkspaceId,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// WAL file name created inside the configured durable data directory.
pub const WAL_FILE_NAME: &str = "clinic-wal.jsonl";

/// Serializable mirror of [`ef_clinic_workflows::matching::AutomatedMatchSignal`] for WAL frames.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "signal", rename_all = "snake_case")]
pub enum WalAutoMatchSignal {
    UnambiguousSingle { patient: WorkspaceId },
    AmbiguousOrContradicted { suggested: Option<WorkspaceId> },
    LikelyNonPatient,
}

/// JSON-safe mirror of [`MoveDestination`] for HTTP wire & WAL frames.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WalMoveDestination {
    Patient { workspace_id: WorkspaceId },
    NotPatient,
}

impl From<WalMoveDestination> for MoveDestination {
    fn from(dest: WalMoveDestination) -> Self {
        match dest {
            WalMoveDestination::Patient { workspace_id } => Self::Patient(workspace_id),
            WalMoveDestination::NotPatient => Self::NotPatient,
        }
    }
}

impl From<&MoveDestination> for WalMoveDestination {
    fn from(dest: &MoveDestination) -> Self {
        match dest {
            MoveDestination::Patient(workspace_id) => Self::Patient {
                workspace_id: workspace_id.clone(),
            },
            MoveDestination::NotPatient => Self::NotPatient,
        }
    }
}

/// Serializable mirror of [`VerificationTokenKind`] for WAL frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WalTokenKind {
    SignInLink,
    ColleagueInvite,
}

impl From<VerificationTokenKind> for WalTokenKind {
    fn from(kind: VerificationTokenKind) -> Self {
        match kind {
            VerificationTokenKind::SignInLink => Self::SignInLink,
            VerificationTokenKind::ColleagueInvite => Self::ColleagueInvite,
        }
    }
}

impl From<WalTokenKind> for VerificationTokenKind {
    fn from(kind: WalTokenKind) -> Self {
        match kind {
            WalTokenKind::SignInLink => Self::SignInLink,
            WalTokenKind::ColleagueInvite => Self::ColleagueInvite,
        }
    }
}

/// Typed durable WAL entry capturing every authoritative state transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum WalEntry {
    UpsertAccount {
        account: PhysicianAccount,
    },
    DeactivateAccount {
        email: EmailAddress,
    },
    SetSessionsEndedBefore {
        email: EmailAddress,
        fence_secs: u64,
    },
    InitiateSignIn {
        raw_email: String,
        kind: WalTokenKind,
        now_secs: u64,
    },
    VerifyToken {
        raw_email: String,
        raw_token: String,
        now_secs: u64,
        fixed_ends_at: Option<u64>,
    },
    TouchSession {
        session_id: SessionId,
        now_secs: u64,
    },
    SignOut {
        session_id: SessionId,
        now_secs: u64,
    },
    ExpireSession {
        session_id: SessionId,
        now_secs: u64,
    },
    RegisterWorkspace {
        workspace_id: WorkspaceId,
        practice_id: PracticeId,
        display_name: String,
        panel_physicians: Vec<PhysicianId>,
    },
    InsertSource {
        payload: CitedSourcePayload,
    },
    WithdrawSource {
        workspace_id: WorkspaceId,
        source_key: SourceKey,
    },
    ExecuteChatCommand {
        workspace_id: WorkspaceId,
        chat_id: ChatId,
        envelope: CommandEnvelope,
        now_secs: u64,
    },
    RecordStep {
        workspace_id: WorkspaceId,
        chat_id: ChatId,
        work_item_id: WorkItemId,
        step: ExchangeStep,
        now_secs: u64,
    },
    AppendDelta {
        workspace_id: WorkspaceId,
        chat_id: ChatId,
        work_item_id: WorkItemId,
        text_delta: String,
        now_secs: u64,
    },
    CompleteExchange {
        workspace_id: WorkspaceId,
        chat_id: ChatId,
        work_item_id: WorkItemId,
        response_id: ResponseId,
        attempt_id: AttemptId,
        finished_at: String,
        segments: Vec<AnswerSegment>,
    },
    RecordAttemptFailure {
        workspace_id: WorkspaceId,
        chat_id: ChatId,
        work_item_id: WorkItemId,
        now_secs: u64,
    },
    EmitResync {
        workspace_id: WorkspaceId,
        chat_id: ChatId,
        reason: String,
    },
    PutDraft {
        workspace_id: WorkspaceId,
        chat_id: ChatId,
        text: String,
        expected_revision: u64,
    },
    AddTodayItem {
        item: TodayQueueItem,
    },
    AddScheduledVisit {
        visit: ScheduledVisitEntry,
    },
    SetChannelHealth {
        channel_id: String,
        status: ChannelHealthStatus,
    },
    MarkTodaySeen {
        physician_id: PhysicianId,
        item_id: String,
        version_id: u64,
    },
    MarkTodayDone {
        physician_id: PhysicianId,
        item: TodayQueueItem,
    },
    IngestChannelMessage {
        channel_id: String,
        cursor_seq: u64,
        conversation_id: Option<String>,
        message_id: Option<String>,
        body: Option<String>,
        unreadable_reason: Option<String>,
        auto_signal: Option<WalAutoMatchSignal>,
        now_secs: u64,
    },
    DecideMessageMove {
        physician_id: PhysicianId,
        practice_id: PracticeId,
        conversation_id: String,
        destination: WalMoveDestination,
        now_secs: u64,
    },
    UndoMessageMove {
        conversation_id: String,
    },
    ReplyCompleteReview {
        conversation_id: String,
        for_patient: WorkspaceId,
        proposed_reply_text: Option<String>,
        basis_citations: Vec<SourceKey>,
        flagged_concern: Option<String>,
        answered_by_human_reply: bool,
    },
    ReplyEditDraft {
        physician_id: PhysicianId,
        practice_id: PracticeId,
        conversation_id: String,
        draft_text: String,
    },
    ReplyRestoreSystem {
        physician_id: PhysicianId,
        practice_id: PracticeId,
        conversation_id: String,
    },
    ReplyClose {
        physician_id: PhysicianId,
        practice_id: PracticeId,
        conversation_id: String,
    },
    ReplyUndoClose {
        physician_id: PhysicianId,
        practice_id: PracticeId,
        conversation_id: String,
    },
    MemoryRecordEntry {
        workspace_id: WorkspaceId,
        topic: String,
        statement: String,
        observed_at: String,
        authority: MemoryAuthority,
    },
    MemoryRunRounds {
        workspace_id: WorkspaceId,
        observations: Vec<RoundsObservation>,
        abort_mid_run: bool,
    },
    MemoryRetireEntry {
        workspace_id: WorkspaceId,
        entry_id: String,
        resolved_at: String,
        reason: String,
    },
    CreateConnectedCareInvitation {
        request: CreateInvitationRequest,
    },
    ResendConnectedCareInvitation {
        pair: PairKey,
        resent_at: String,
    },
    CancelConnectedCareInvitation {
        pair: PairKey,
        cancelled_at: String,
    },
}

/// Errors returned by durable WAL persistence operations.
#[derive(Debug, Error)]
pub enum WalError {
    #[error("WAL I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("WAL serialization/deserialization error at line {line}: {source}")]
    Json {
        line: usize,
        #[source]
        source: serde_json::Error,
    },
}

/// Sink abstraction implemented by [`DurableClinicStore`] on native hosts and [`InMemoryWal`] in unit/wasm tests.
pub trait WalSink: Send {
    fn append(&mut self, entry: &WalEntry) -> Result<(), WalError>;
}

/// Pure in-memory WAL sink used for cross-platform router unit checks.
#[derive(Debug, Default)]
pub struct InMemoryWal {
    entries: Vec<WalEntry>,
}

impl InMemoryWal {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    #[must_use]
    pub fn entries(&self) -> &[WalEntry] {
        &self.entries
    }
}

impl WalSink for InMemoryWal {
    fn append(&mut self, entry: &WalEntry) -> Result<(), WalError> {
        self.entries.push(entry.clone());
        Ok(())
    }
}

/// Atomic append-only JSONL WAL file store that fsyncs every entry to disk.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct DurableClinicStore {
    wal_path: PathBuf,
    file: File,
}

#[cfg(not(target_arch = "wasm32"))]
impl DurableClinicStore {
    /// Opens (or creates) the WAL inside `data_dir` and returns all previously
    /// committed entries in exact append order for deterministic replay.
    pub fn open(data_dir: &Path) -> Result<(Self, Vec<WalEntry>), WalError> {
        fs::create_dir_all(data_dir)?;
        let wal_path = data_dir.join(WAL_FILE_NAME);

        let mut existing_entries = Vec::new();
        if wal_path.exists() {
            let reader_file = File::open(&wal_path)?;
            let reader = BufReader::new(reader_file);
            for (idx, line_res) in reader.lines().enumerate() {
                let line = line_res?;
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let entry: WalEntry =
                    serde_json::from_str(trimmed).map_err(|source| WalError::Json {
                        line: idx + 1,
                        source,
                    })?;
                existing_entries.push(entry);
            }
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&wal_path)?;

        Ok((Self { wal_path, file }, existing_entries))
    }

    /// Returns the filesystem path of the active WAL file.
    #[must_use]
    pub fn wal_path(&self) -> &Path {
        &self.wal_path
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl WalSink for DurableClinicStore {
    fn append(&mut self, entry: &WalEntry) -> Result<(), WalError> {
        let mut line =
            serde_json::to_string(entry).map_err(|source| WalError::Json { line: 0, source })?;
        line.push('\n');
        self.file.write_all(line.as_bytes())?;
        self.file.flush()?;
        self.file.sync_all()?;
        Ok(())
    }
}
