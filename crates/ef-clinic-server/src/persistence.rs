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
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, Instant};

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
    #[error("WAL PostgreSQL error: {0}")]
    Postgres(#[from] ef_pgwire::PgError),
}

/// Cross-instance RAII lock guard serializing multi-instance WAL access on native hosts.
#[derive(Debug)]
pub struct WalLockGuard {
    #[cfg(not(target_arch = "wasm32"))]
    lock_path: PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for WalLockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.lock_path);
    }
}

/// Sink abstraction implemented by [`DurableClinicStore`] on native hosts and [`InMemoryWal`] in unit/wasm tests.
pub trait WalSink: Send {
    fn append(&mut self, entry: &WalEntry) -> Result<(), WalError>;

    /// Reads any newly appended WAL entries written by peer instances since the last sync.
    fn sync_new_entries(&mut self) -> Result<Vec<WalEntry>, WalError> {
        Ok(Vec::new())
    }

    /// Acquires an optional cross-instance directory lock for serializing concurrent multi-instance requests.
    fn acquire_cross_instance_lock(&self) -> Result<Option<WalLockGuard>, WalError> {
        Ok(None)
    }
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
    lock_path: PathBuf,
    file: File,
    synced_offset: u64,
    lines_read: usize,
}

#[cfg(not(target_arch = "wasm32"))]
impl DurableClinicStore {
    /// Opens (or creates) the WAL inside `data_dir` and returns all previously
    /// committed entries in exact append order for deterministic replay.
    pub fn open(data_dir: &Path) -> Result<(Self, Vec<WalEntry>), WalError> {
        fs::create_dir_all(data_dir)?;
        let wal_path = data_dir.join(WAL_FILE_NAME);
        let lock_path = data_dir.join(".clinic-wal.lock");

        let mut existing_entries = Vec::new();
        let mut synced_offset = 0u64;
        let mut lines_read = 0usize;
        if wal_path.exists() {
            let reader_file = File::open(&wal_path)?;
            let mut reader = BufReader::new(reader_file);
            let mut buf = String::new();
            loop {
                buf.clear();
                let bytes = reader.read_line(&mut buf)?;
                if bytes == 0 {
                    break;
                }
                synced_offset = synced_offset.saturating_add(bytes as u64);
                lines_read = lines_read.saturating_add(1);
                let trimmed = buf.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let entry: WalEntry =
                    serde_json::from_str(trimmed).map_err(|source| WalError::Json {
                        line: lines_read,
                        source,
                    })?;
                existing_entries.push(entry);
            }
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&wal_path)?;

        Ok((
            Self {
                wal_path,
                lock_path,
                file,
                synced_offset,
                lines_read,
            },
            existing_entries,
        ))
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
        let len_before = self.file.metadata()?.len();
        let mut line =
            serde_json::to_string(entry).map_err(|source| WalError::Json { line: 0, source })?;
        line.push('\n');
        self.file.write_all(line.as_bytes())?;
        self.file.flush()?;
        self.file.sync_all()?;
        if len_before == self.synced_offset {
            self.synced_offset = len_before.saturating_add(line.len() as u64);
            self.lines_read = self.lines_read.saturating_add(1);
        }
        Ok(())
    }

    fn sync_new_entries(&mut self) -> Result<Vec<WalEntry>, WalError> {
        if !self.wal_path.exists() {
            return Ok(Vec::new());
        }
        let file_len = fs::metadata(&self.wal_path)?.len();
        if file_len <= self.synced_offset {
            return Ok(Vec::new());
        }

        let mut reader_file = File::open(&self.wal_path)?;
        reader_file.seek(SeekFrom::Start(self.synced_offset))?;
        let mut reader = BufReader::new(reader_file);
        let mut new_entries = Vec::new();
        let mut buf = String::new();
        loop {
            buf.clear();
            let bytes = reader.read_line(&mut buf)?;
            if bytes == 0 {
                break;
            }
            if !buf.ends_with('\n') {
                // Partial uncommitted line: wait for peer fsync.
                break;
            }
            self.synced_offset = self.synced_offset.saturating_add(bytes as u64);
            self.lines_read = self.lines_read.saturating_add(1);
            let trimmed = buf.trim();
            if trimmed.is_empty() {
                continue;
            }
            let entry: WalEntry =
                serde_json::from_str(trimmed).map_err(|source| WalError::Json {
                    line: self.lines_read,
                    source,
                })?;
            new_entries.push(entry);
        }
        Ok(new_entries)
    }

    fn acquire_cross_instance_lock(&self) -> Result<Option<WalLockGuard>, WalError> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&self.lock_path)
            {
                Ok(_) => {
                    return Ok(Some(WalLockGuard {
                        lock_path: self.lock_path.clone(),
                    }));
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    if Instant::now() >= deadline {
                        return Err(WalError::Io(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "timed out waiting for shared clinic WAL lock",
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(err) => return Err(WalError::Io(err)),
            }
        }
    }
}

/// PostgreSQL 18 durable WAL store backed by [`ef_pgwire::PgClient`].
///
/// Applies `migrations/0001_m2_clinic_slice.sql` idempotently on startup and persists
/// every mutation into `ef_clinic_wal_events` using parameterized Extended Query statements.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct PgClinicStore {
    client: ef_pgwire::PgClient,
    last_synced_seq: i64,
}

#[cfg(not(target_arch = "wasm32"))]
impl PgClinicStore {
    /// Connects to PostgreSQL, applies the Clinic migration + WAL journal DDL idempotently,
    /// and returns all previously persisted entries in exact commit order.
    pub fn open(config: &ef_pgwire::PgConnectionConfig) -> Result<(Self, Vec<WalEntry>), WalError> {
        let mut client = ef_pgwire::PgClient::connect(config)?;
        client.execute_batch_ddl(include_str!("../migrations/0001_m2_clinic_slice.sql"))?;
        client.execute_batch_ddl(
            "CREATE TABLE IF NOT EXISTS ef_clinic_wal_events (\
                seq BIGSERIAL PRIMARY KEY, \
                payload_json TEXT NOT NULL, \
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()\
            );",
        )?;

        let mut store = Self {
            client,
            last_synced_seq: 0,
        };
        let entries = store.load_entries_after(0)?;
        Ok((store, entries))
    }

    fn load_entries_after(&mut self, after_seq: i64) -> Result<Vec<WalEntry>, WalError> {
        let rows = self.client.query(
            "SELECT seq, payload_json FROM ef_clinic_wal_events WHERE seq > $1 ORDER BY seq ASC",
            &[ef_pgwire::PgParam::Int64(after_seq)],
        )?;
        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            let seq = row.get_i64(0)?.unwrap_or(self.last_synced_seq);
            let raw = row.get_text_required(1)?;
            let line = usize::try_from(seq).unwrap_or(0);
            let entry: WalEntry =
                serde_json::from_str(&raw).map_err(|source| WalError::Json { line, source })?;
            self.last_synced_seq = self.last_synced_seq.max(seq);
            entries.push(entry);
        }
        Ok(entries)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl WalSink for PgClinicStore {
    fn append(&mut self, entry: &WalEntry) -> Result<(), WalError> {
        let json =
            serde_json::to_string(entry).map_err(|source| WalError::Json { line: 0, source })?;
        let rows = self.client.query(
            "INSERT INTO ef_clinic_wal_events (payload_json) VALUES ($1) RETURNING seq",
            &[ef_pgwire::PgParam::Text(&json)],
        )?;
        if let Some(row) = rows.first()
            && let Some(seq) = row.get_i64(0)?
        {
            self.last_synced_seq = self.last_synced_seq.max(seq);
        }
        Ok(())
    }

    fn sync_new_entries(&mut self) -> Result<Vec<WalEntry>, WalError> {
        self.load_entries_after(self.last_synced_seq)
    }
}
