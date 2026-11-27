//! Durable atomic append-only WAL persistence for Curie Mobile state.

#[cfg(not(target_arch = "wasm32"))]
use std::fs::{self, File, OpenOptions};
#[cfg(not(target_arch = "wasm32"))]
use std::io::{BufRead, BufReader, Write};
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

use ef_mobile_domain::{
    AccountFencedHealthBatch, AccountIncarnation, DevicePushRegistration, IncomingBatchItem,
    MobileAccountId, MobileSessionId,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MOBILE_WAL_FILE_NAME: &str = "mobile-wal.jsonl";

/// Authoritative WAL mutation frame for Curie Mobile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum MobileWalEntry {
    IssueSignInLink {
        raw_email: String,
        now_secs: u64,
    },
    VerifySignIn {
        raw_email: String,
        raw_token: String,
        now_secs: u64,
    },
    RevokeSession {
        session_id: MobileSessionId,
    },
    IngestRecords {
        items: Vec<IncomingBatchItem>,
    },
    IngestHealthBatch {
        account_id: MobileAccountId,
        batch: AccountFencedHealthBatch,
    },
    RegisterPushToken {
        registration: DevicePushRegistration,
    },
    RevokePushDevice {
        device_id: String,
    },
    DeleteAccount {
        account_id: MobileAccountId,
        incarnation: AccountIncarnation,
    },
}

#[derive(Debug, Error)]
pub enum MobileWalError {
    #[error("mobile WAL I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("mobile WAL JSON error at line {line}: {source}")]
    Json {
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("mobile WAL PostgreSQL error: {0}")]
    Postgres(#[from] ef_pgwire::PgError),
}

pub trait MobileWalSink: Send {
    fn append(&mut self, entry: &MobileWalEntry) -> Result<(), MobileWalError>;

    /// Reads any newly appended WAL entries written by peer instances since the last sync.
    fn sync_new_entries(&mut self) -> Result<Vec<MobileWalEntry>, MobileWalError> {
        Ok(Vec::new())
    }

    /// Verifies live backing-store liveness (e.g., `SELECT 1` ping for PostgreSQL).
    fn check_health(&mut self) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct InMemoryMobileWal {
    entries: Vec<MobileWalEntry>,
}

impl InMemoryMobileWal {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl MobileWalSink for InMemoryMobileWal {
    fn append(&mut self, entry: &MobileWalEntry) -> Result<(), MobileWalError> {
        self.entries.push(entry.clone());
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct DurableMobileStore {
    wal_path: PathBuf,
    file: File,
}

#[cfg(not(target_arch = "wasm32"))]
impl DurableMobileStore {
    pub fn open(data_dir: &Path) -> Result<(Self, Vec<MobileWalEntry>), MobileWalError> {
        fs::create_dir_all(data_dir)?;
        let wal_path = data_dir.join(MOBILE_WAL_FILE_NAME);
        let mut entries = Vec::new();
        if wal_path.exists() {
            let reader = BufReader::new(File::open(&wal_path)?);
            for (idx, line_res) in reader.lines().enumerate() {
                let line = line_res?;
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let entry: MobileWalEntry =
                    serde_json::from_str(trimmed).map_err(|source| MobileWalError::Json {
                        line: idx + 1,
                        source,
                    })?;
                entries.push(entry);
            }
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&wal_path)?;
        Ok((Self { wal_path, file }, entries))
    }

    #[must_use]
    pub fn wal_path(&self) -> &Path {
        &self.wal_path
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl MobileWalSink for DurableMobileStore {
    fn append(&mut self, entry: &MobileWalEntry) -> Result<(), MobileWalError> {
        let mut line = serde_json::to_string(entry)
            .map_err(|source| MobileWalError::Json { line: 0, source })?;
        line.push('\n');
        self.file.write_all(line.as_bytes())?;
        self.file.flush()?;
        self.file.sync_all()?;
        Ok(())
    }
}

/// PostgreSQL 18 durable WAL store for Curie Mobile backed by [`ef_pgwire::PgClient`].
///
/// Applies `migrations/0001_m3_mobile.sql` idempotently on startup and persists
/// every mutation into `ef_mobile_wal_events` using parameterized Extended Query statements.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct PgMobileStore {
    client: ef_pgwire::PgClient,
    last_synced_seq: i64,
}

#[cfg(not(target_arch = "wasm32"))]
impl PgMobileStore {
    /// Connects to PostgreSQL, applies the Mobile migration + WAL journal DDL idempotently,
    /// and returns all previously persisted entries in exact commit order.
    pub fn open(
        config: &ef_pgwire::PgConnectionConfig,
    ) -> Result<(Self, Vec<MobileWalEntry>), MobileWalError> {
        let mut client = ef_pgwire::PgClient::connect(config)?;
        client.execute_batch_ddl(include_str!("../migrations/0001_m3_mobile.sql"))?;
        client.execute_batch_ddl(
            "CREATE TABLE IF NOT EXISTS ef_mobile_wal_events (\
                seq BIGSERIAL PRIMARY KEY, \
                payload_json TEXT NOT NULL, \
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()\
            );",
        )?;

        let rows = client.query(
            "SELECT seq, payload_json FROM ef_mobile_wal_events WHERE seq > $1 ORDER BY seq ASC",
            &[ef_pgwire::PgParam::Int64(0)],
        )?;
        let mut entries = Vec::with_capacity(rows.len());
        let mut last_synced_seq = 0i64;
        for row in rows {
            let seq = row.get_i64(0)?.unwrap_or(last_synced_seq);
            let raw = row.get_text_required(1)?;
            let line = usize::try_from(seq).unwrap_or(0);
            let entry: MobileWalEntry = serde_json::from_str(&raw)
                .map_err(|source| MobileWalError::Json { line, source })?;
            last_synced_seq = last_synced_seq.max(seq);
            entries.push(entry);
        }

        Ok((
            Self {
                client,
                last_synced_seq,
            },
            entries,
        ))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl MobileWalSink for PgMobileStore {
    fn append(&mut self, entry: &MobileWalEntry) -> Result<(), MobileWalError> {
        let json = serde_json::to_string(entry)
            .map_err(|source| MobileWalError::Json { line: 0, source })?;
        let rows = self.client.query(
            "INSERT INTO ef_mobile_wal_events (payload_json) VALUES ($1) RETURNING seq",
            &[ef_pgwire::PgParam::Text(&json)],
        )?;
        if let Some(row) = rows.first()
            && let Some(seq) = row.get_i64(0)?
        {
            self.last_synced_seq = self.last_synced_seq.max(seq);
        }
        Ok(())
    }

    fn sync_new_entries(&mut self) -> Result<Vec<MobileWalEntry>, MobileWalError> {
        let rows = self.client.query(
            "SELECT seq, payload_json FROM ef_mobile_wal_events WHERE seq > $1 ORDER BY seq ASC",
            &[ef_pgwire::PgParam::Int64(self.last_synced_seq)],
        )?;
        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            let seq = row.get_i64(0)?.unwrap_or(self.last_synced_seq);
            let raw = row.get_text_required(1)?;
            let line = usize::try_from(seq).unwrap_or(0);
            let entry: MobileWalEntry = serde_json::from_str(&raw)
                .map_err(|source| MobileWalError::Json { line, source })?;
            self.last_synced_seq = self.last_synced_seq.max(seq);
            entries.push(entry);
        }
        Ok(entries)
    }

    fn check_health(&mut self) -> Result<(), String> {
        self.client.ping().map_err(|err| err.to_string())
    }
}
