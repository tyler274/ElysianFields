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
}

pub trait MobileWalSink: Send {
    fn append(&mut self, entry: &MobileWalEntry) -> Result<(), MobileWalError>;
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
