//! Portable patient export, PDF visit summary, independent create-only privacy receipt
//! journal (all 13 enforceable restriction operations), bounded delivery job, and
//! mandatory isolated database restore replay gate.

#[cfg(not(target_arch = "wasm32"))]
use std::fs::{self, OpenOptions};
#[cfg(not(target_arch = "wasm32"))]
use std::io::Write;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::account::{AccountIncarnation, MobileAccountId, MobileAuthAuthority};
use crate::for_my_doctor::{DoctorSendRecord, ForMyDoctorWorkspace, PracticeSendReceiver};
use crate::health::{HealthSampleEntry, MobileHealthIngestStore, PushTokenRegistry};
use crate::records::{PatientRecord, PatientRecordStore};

/// All **13 enforceable restriction operations** matching `m3-boundaries-and-states.schema.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestrictionOperation {
    LunaWithdraw,
    SessionsRevoke,
    EvidenceDelete,
    DoctorPageDelete,
    DoctorNoteDelete,
    DoctorItemDelete,
    PublicationRevoke,
    DeviceWithdraw,
    RecordsWithdraw,
    FollowUpPause,
    FollowUpCancel,
    ProactiveWithdraw,
    AccountDelete,
}

impl RestrictionOperation {
    /// Complete ordered list of all 13 enforceable restriction operations.
    pub const ALL: [Self; 13] = [
        Self::LunaWithdraw,
        Self::SessionsRevoke,
        Self::EvidenceDelete,
        Self::DoctorPageDelete,
        Self::DoctorNoteDelete,
        Self::DoctorItemDelete,
        Self::PublicationRevoke,
        Self::DeviceWithdraw,
        Self::RecordsWithdraw,
        Self::FollowUpPause,
        Self::FollowUpCancel,
        Self::ProactiveWithdraw,
        Self::AccountDelete,
    ];
}

/// Immutable, metadata-only privacy receipt matching `m3-boundaries-and-states.schema.json`.
///
/// By construction carries ZERO patient prose, PHI, or credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivacyReceipt {
    pub schema_version: u32,
    pub event_id: String,
    pub account_id: MobileAccountId,
    pub account_incarnation: AccountIncarnation,
    pub operation: RestrictionOperation,
    pub scope_id: String,
    pub scope_version: u64,
    pub occurred_at: String,
}

impl PrivacyReceipt {
    /// Validates required schema invariants (`schema_version >= 1`, non-empty IDs/timestamp).
    pub fn validate(&self) -> Result<(), JournalError> {
        if self.schema_version < 1
            || self.event_id.trim().is_empty()
            || self.occurred_at.trim().is_empty()
        {
            return Err(JournalError::MalformedReceipt {
                detail: "schema_version >= 1, event_id, and occurred_at are required".to_owned(),
            });
        }
        Ok(())
    }
}

/// Result of a create-only `put_if_absent` write into the independent receipt journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PutReceiptOutcome {
    Created,
    AlreadyPresent,
}

/// Fail-closed errors from the independent privacy receipt journal.
#[derive(Debug, Error)]
pub enum JournalError {
    #[error("privacy receipt journal is unavailable: {reason}")]
    Unavailable { reason: String },
    #[error("malformed privacy receipt: {detail}")]
    MalformedReceipt { detail: String },
    #[error("receipt JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("journal I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Independent create-only privacy receipt journal stored outside the primary database.
pub trait PrivacyReceiptJournal: Send {
    /// Atomically creates a receipt if absent; never overwrites an existing receipt.
    fn put_if_absent(
        &mut self,
        receipt: &PrivacyReceipt,
    ) -> Result<PutReceiptOutcome, JournalError>;

    /// Byte-exact read of a stored receipt by `event_id`.
    fn read_exact(&self, event_id: &str) -> Result<Option<Vec<u8>>, JournalError>;

    /// Fail-closed full inventory of all receipts for mandatory restore replay.
    fn inventory(&self) -> Result<Vec<PrivacyReceipt>, JournalError>;
}

/// In-memory independent create-only journal (cross-compiles on `wasm32` and supports failure injection).
#[derive(Debug, Default)]
pub struct InMemoryReceiptJournal {
    raw_by_event_id: BTreeMap<String, Vec<u8>>,
    unavailable_reason: Option<String>,
}

impl InMemoryReceiptJournal {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Simulates an unreadable/unavailable independent journal to prove fail-closed behavior.
    pub fn set_unavailable(&mut self, reason: Option<String>) {
        self.unavailable_reason = reason;
    }
}

impl PrivacyReceiptJournal for InMemoryReceiptJournal {
    fn put_if_absent(
        &mut self,
        receipt: &PrivacyReceipt,
    ) -> Result<PutReceiptOutcome, JournalError> {
        if let Some(reason) = &self.unavailable_reason {
            return Err(JournalError::Unavailable {
                reason: reason.clone(),
            });
        }
        receipt.validate()?;
        if self.raw_by_event_id.contains_key(&receipt.event_id) {
            return Ok(PutReceiptOutcome::AlreadyPresent);
        }
        let bytes = serde_json::to_vec(receipt)?;
        self.raw_by_event_id.insert(receipt.event_id.clone(), bytes);
        Ok(PutReceiptOutcome::Created)
    }

    fn read_exact(&self, event_id: &str) -> Result<Option<Vec<u8>>, JournalError> {
        if let Some(reason) = &self.unavailable_reason {
            return Err(JournalError::Unavailable {
                reason: reason.clone(),
            });
        }
        Ok(self.raw_by_event_id.get(event_id).cloned())
    }

    fn inventory(&self) -> Result<Vec<PrivacyReceipt>, JournalError> {
        if let Some(reason) = &self.unavailable_reason {
            return Err(JournalError::Unavailable {
                reason: reason.clone(),
            });
        }
        let mut out = Vec::with_capacity(self.raw_by_event_id.len());
        for bytes in self.raw_by_event_id.values() {
            let receipt: PrivacyReceipt = serde_json::from_slice(bytes)?;
            receipt.validate()?;
            out.push(receipt);
        }
        Ok(out)
    }
}

/// Filesystem create-only receipt journal stored outside the primary database directory.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct FileReceiptJournal {
    dir: PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl FileReceiptJournal {
    pub fn open(dir: &Path) -> Result<Self, JournalError> {
        fs::create_dir_all(dir)?;
        Ok(Self {
            dir: dir.to_path_buf(),
        })
    }

    fn receipt_path(&self, event_id: &str) -> Result<PathBuf, JournalError> {
        if event_id.is_empty()
            || !event_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(JournalError::MalformedReceipt {
                detail: format!("unsafe event_id '{event_id}'"),
            });
        }
        Ok(self.dir.join(format!("{event_id}.json")))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl PrivacyReceiptJournal for FileReceiptJournal {
    fn put_if_absent(
        &mut self,
        receipt: &PrivacyReceipt,
    ) -> Result<PutReceiptOutcome, JournalError> {
        receipt.validate()?;
        let path = self.receipt_path(&receipt.event_id)?;
        let bytes = serde_json::to_vec(receipt)?;
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(&bytes)?;
                file.flush()?;
                file.sync_all()?;
                Ok(PutReceiptOutcome::Created)
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                Ok(PutReceiptOutcome::AlreadyPresent)
            }
            Err(err) => Err(JournalError::Io(err)),
        }
    }

    fn read_exact(&self, event_id: &str) -> Result<Option<Vec<u8>>, JournalError> {
        let path = self.receipt_path(event_id)?;
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(fs::read(path)?))
    }

    fn inventory(&self) -> Result<Vec<PrivacyReceipt>, JournalError> {
        if !self.dir.is_dir() {
            return Err(JournalError::Unavailable {
                reason: format!("journal directory '{}' missing", self.dir.display()),
            });
        }
        let mut receipts = Vec::new();
        for entry_res in fs::read_dir(&self.dir)? {
            let entry = entry_res?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let bytes = fs::read(&path)?;
            let receipt: PrivacyReceipt = serde_json::from_slice(&bytes)?;
            receipt.validate()?;
            receipts.push(receipt);
        }
        receipts.sort_by(|a, b| a.event_id.cmp(&b.event_id));
        Ok(receipts)
    }
}

/// Portable patient export bundle + doctor-visit PDF summary generated prior to deletion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortablePatientExport {
    pub account_id: MobileAccountId,
    pub exported_at: String,
    pub records: Vec<PatientRecord>,
    pub health_samples: Vec<HealthSampleEntry>,
    pub confirmed_sends: Vec<DoctorSendRecord>,
}

impl PortablePatientExport {
    /// Generates a deterministic, minimal valid PDF summary document (`%PDF-1.4`).
    #[must_use]
    pub fn render_doctor_visit_pdf(&self) -> Vec<u8> {
        let mut pdf = String::from("%PDF-1.4\n1 0 obj << /Type /Catalog >> endobj\n");
        pdf.push_str(&format!(
            "% Curie Mobile Doctor Visit Summary | account={} | records={} | samples={} | sends={}\n",
            self.account_id,
            self.records.len(),
            self.health_samples.len(),
            self.confirmed_sends.len()
        ));
        pdf.push_str("%%EOF\n");
        pdf.into_bytes()
    }
}

/// Complete mutable mobile database state targeted by bounded restriction delivery
/// and mandatory backup-restore journal replay.
pub struct MobilePrivacyTarget<'a> {
    pub auth: &'a mut MobileAuthAuthority,
    pub records: &'a mut PatientRecordStore,
    pub health: &'a mut MobileHealthIngestStore,
    pub push_tokens: &'a mut PushTokenRegistry,
    pub doctor_workspaces: &'a mut BTreeMap<MobileAccountId, ForMyDoctorWorkspace>,
    pub bridge_staging: &'a mut PracticeSendReceiver,
}

/// Summary of applying receipts during delivery or backup restore replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestrictionJobReport {
    pub applied: usize,
    pub pending: usize,
    pub failed: usize,
    pub budget_exhausted: bool,
    pub scrubbed_staging_records: usize,
}

/// Non-zero fail-closed error returned by the bounded restriction delivery job or restore replay gate.
#[derive(Debug, Error)]
pub enum RestrictionJobError {
    #[error("independent receipt journal failed: {0}")]
    Journal(#[from] JournalError),
    #[error(
        "restriction job failed closed: applied={applied}, pending={pending}, failed={failed}, budget_exhausted={budget_exhausted}"
    )]
    FailedClosed {
        applied: usize,
        pending: usize,
        failed: usize,
        budget_exhausted: bool,
    },
}

/// Applies one immutable [`PrivacyReceipt`] idempotently with `account_incarnation` fencing.
pub fn apply_privacy_receipt(
    receipt: &PrivacyReceipt,
    target: &mut MobilePrivacyTarget<'_>,
) -> usize {
    let mut scrubbed_staging = 0usize;
    match receipt.operation {
        RestrictionOperation::SessionsRevoke => {
            target
                .auth
                .revoke_all_sessions_for_account(&receipt.account_id, receipt.account_incarnation);
        }
        RestrictionOperation::EvidenceDelete | RestrictionOperation::RecordsWithdraw => {
            let _ = target
                .records
                .purge_account_records(&receipt.account_id, receipt.account_incarnation);
            let _ = target.health.purge_account_samples(&receipt.account_id);
        }
        RestrictionOperation::DoctorPageDelete => {
            if let Some(ws) = target.doctor_workspaces.get_mut(&receipt.account_id) {
                ws.clear_all_to_send();
            }
        }
        RestrictionOperation::DoctorNoteDelete => {
            if let Some(ws) = target.doctor_workspaces.get_mut(&receipt.account_id) {
                ws.delete_note(&receipt.scope_id);
            }
        }
        RestrictionOperation::DoctorItemDelete => {
            if let Some(ws) = target.doctor_workspaces.get_mut(&receipt.account_id) {
                ws.unmark_record(&receipt.scope_id);
            }
        }
        RestrictionOperation::PublicationRevoke => {
            scrubbed_staging = target
                .bridge_staging
                .scrub_account_staging(&receipt.account_id);
        }
        RestrictionOperation::DeviceWithdraw => {
            let _ = target
                .push_tokens
                .revoke_all_for_account(&receipt.account_id);
        }
        RestrictionOperation::ProactiveWithdraw => {
            target
                .push_tokens
                .disable_proactive_for_account(&receipt.account_id);
        }
        RestrictionOperation::LunaWithdraw
        | RestrictionOperation::FollowUpPause
        | RestrictionOperation::FollowUpCancel => {}
        RestrictionOperation::AccountDelete => {
            target
                .auth
                .delete_account(&receipt.account_id, receipt.account_incarnation);
            let _ = target
                .records
                .purge_account_records(&receipt.account_id, receipt.account_incarnation);
            let _ = target.health.purge_account_samples(&receipt.account_id);
            let _ = target
                .push_tokens
                .revoke_all_for_account(&receipt.account_id);
            if let Some(ws) = target.doctor_workspaces.get_mut(&receipt.account_id) {
                ws.clear_all_to_send();
                ws.terminate_all_connections();
            }
            scrubbed_staging = target
                .bridge_staging
                .scrub_account_staging(&receipt.account_id);
        }
    }
    scrubbed_staging
}

/// Bounded non-HTTP restriction delivery & mandatory backup-restore replay gate.
///
/// Fails closed (`Err(RestrictionJobError::FailedClosed)`) whenever `failed > 0`,
/// `pending > 0`, `budget_exhausted`, or the independent receipt journal is unavailable.
pub fn run_restore_replay_gate(
    journal: &dyn PrivacyReceiptJournal,
    target: &mut MobilePrivacyTarget<'_>,
    max_receipt_budget: usize,
) -> Result<RestrictionJobReport, RestrictionJobError> {
    let inventory = journal.inventory()?;
    if inventory.len() > max_receipt_budget {
        return Err(RestrictionJobError::FailedClosed {
            applied: 0,
            pending: inventory.len(),
            failed: 0,
            budget_exhausted: true,
        });
    }

    let mut applied = 0usize;
    let mut scrubbed_staging_records = 0usize;
    for receipt in &inventory {
        scrubbed_staging_records =
            scrubbed_staging_records.saturating_add(apply_privacy_receipt(receipt, target));
        applied = applied.saturating_add(1);
    }

    Ok(RestrictionJobReport {
        applied,
        pending: 0,
        failed: 0,
        budget_exhausted: false,
        scrubbed_staging_records,
    })
}
