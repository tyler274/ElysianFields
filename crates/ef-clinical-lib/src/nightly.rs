//! Nightly clinical reference library pipeline proving all 9 core failure/ingestion guarantees.
//!
//! Strictly headless (zero rendering engine deps, zero product deps) and enforces
//! de-identified concept-only queries.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::hash::sha256_hex;

/// Repeated identical failure code threshold before automatic retries halt (Rule 4).
pub const REPEATED_FAILURE_CODE_HALT_LIMIT: u32 = 2;

/// Consecutive failed or zero-progress deferred nights threshold before halt (Rule 5).
pub const CONSECUTIVE_FAILED_OR_IDLE_HALT_LIMIT: u32 = 3;

/// Consecutive unreadable slot holder observations triggering an operator alert (Rule 7).
pub const UNREADABLE_HOLDER_ALERT_THRESHOLD: u32 = 2;

/// Validated de-identified clinical concept query (never patient identifiers).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConceptQuery(String);

impl ConceptQuery {
    /// Validates that a library query carries strictly de-identified clinical concepts
    /// and rejects common patient identifier patterns (MRN, SSN, email, DOB).
    pub fn parse(raw: &str) -> Result<Self, NightlyError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(NightlyError::InvalidConceptQuery {
                reason: "concept query must not be empty".to_owned(),
            });
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.contains('@')
            || lower.contains("mrn:")
            || lower.contains("mrn=")
            || lower.contains("patient_id")
            || lower.contains("ssn")
            || lower.contains("dob:")
            || contains_ssn_pattern(trimmed)
        {
            return Err(NightlyError::InvalidConceptQuery {
                reason: "patient identifier detected in de-identified library query".to_owned(),
            });
        }
        Ok(Self(trimmed.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn contains_ssn_pattern(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() < 11 {
        return false;
    }
    for win in bytes.windows(11) {
        if win[3] == b'-'
            && win[6] == b'-'
            && win[0..3].iter().all(u8::is_ascii_digit)
            && win[4..6].iter().all(u8::is_ascii_digit)
            && win[7..11].iter().all(u8::is_ascii_digit)
        {
            return true;
        }
    }
    false
}

/// Validated article identifier in the clinical reference library.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ArticleId(String);

impl ArticleId {
    pub fn new(raw: impl Into<String>) -> Result<Self, NightlyError> {
        let val = raw.into();
        if val.trim().is_empty() {
            return Err(NightlyError::InvalidIdentifier {
                field: "article_id",
            });
        }
        Ok(Self(val))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Validated candidate library version identifier (write-once folder key).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CandidateVersionId(String);

impl CandidateVersionId {
    pub fn new(raw: impl Into<String>) -> Result<Self, NightlyError> {
        let val = raw.into();
        if val.trim().is_empty() || val.contains("..") || val.contains('/') || val.contains('\\') {
            return Err(NightlyError::InvalidIdentifier {
                field: "candidate_version_id",
            });
        }
        Ok(Self(val))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Validated nightly run identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NightRunId(String);

impl NightRunId {
    pub fn new(raw: impl Into<String>) -> Result<Self, NightlyError> {
        let val = raw.into();
        if val.trim().is_empty() {
            return Err(NightlyError::InvalidIdentifier {
                field: "night_run_id",
            });
        }
        Ok(Self(val))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Categorized failure code tracked across nights (Rule 4).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FailureCode(String);

impl FailureCode {
    pub fn new(raw: impl Into<String>) -> Result<Self, NightlyError> {
        let val = raw.into();
        if val.trim().is_empty() {
            return Err(NightlyError::InvalidIdentifier {
                field: "failure_code",
            });
        }
        Ok(Self(val))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Monotonic document reader/parser version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ReaderVersion(pub u32);

/// Explicit article lifecycle state (Rules 1 & 2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ArticleState {
    Active {
        version: CandidateVersionId,
        content_sha256: String,
        reader_version: ReaderVersion,
    },
    WaitingToRetry {
        last_failure: FailureCode,
        attempts: u32,
        reader_version: ReaderVersion,
    },
    HumanReview {
        reason: String,
        reader_version: ReaderVersion,
    },
}

/// Explicit recovery trigger required to leave `HumanReview` or unstick `WaitingToRetry` (Rule 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryTrigger {
    OperatorAction { operator_id: String, note: String },
    NewSourceContent { content_sha256: String },
    NewReaderVersion { reader_version: ReaderVersion },
}

/// Outcome recorded when a nightly run closes (Rule 3: Complete nights only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum NightOutcome {
    /// Only allowed when `remaining_due == 0` and all due work completed cleanly.
    Succeeded { articles_processed: usize },
    /// Partial or deferred run: NEVER recorded as `Succeeded`.
    Deferred {
        articles_processed: usize,
        remaining_due: usize,
    },
    /// Failed night with typed failure code.
    Failed { code: FailureCode },
}

/// Pipeline automatic-execution halt status (Rules 4 & 5).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PipelineHaltState {
    #[default]
    Running,
    HaltedRepeatedFailureCode {
        code: FailureCode,
        nights_observed: u32,
    },
    HaltedConsecutiveFailedOrIdle {
        consecutive_nights: u32,
    },
}

/// Observed status of the prior active slot holder before takeover (Rules 6 & 7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotHolderStatus {
    Empty,
    Running { holder: NightRunId },
    Ended { holder: NightRunId },
    Unreadable { detail: String },
}

/// Operator alert emitted on the 2nd consecutive unreadable slot holder (Rule 7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperatorAlert {
    UnreadableSlotHolder {
        consecutive_nights: u32,
        detail: String,
    },
}

/// Non-copyable slot lease capability token enforcing lost-slot isolation (Rule 8).
#[derive(Debug, PartialEq, Eq)]
pub struct SlotLease {
    run_id: NightRunId,
    lease_epoch: u64,
}

impl SlotLease {
    #[must_use]
    pub const fn run_id(&self) -> &NightRunId {
        &self.run_id
    }

    #[must_use]
    pub const fn lease_epoch(&self) -> u64 {
        self.lease_epoch
    }
}

/// Non-copyable single-use activation handle enforcing single checked active version (Rule 9).
#[derive(Debug, PartialEq, Eq)]
pub struct ActivationHandle {
    version_id: CandidateVersionId,
    run_id: NightRunId,
    lease_epoch: u64,
    manifest_sha256: String,
}

impl ActivationHandle {
    #[must_use]
    pub const fn version_id(&self) -> &CandidateVersionId {
        &self.version_id
    }

    #[must_use]
    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }
}

/// Typed errors enforcing all 9 nightly clinical reference library guarantees.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NightlyError {
    #[error("invalid de-identified concept query: {reason}")]
    InvalidConceptQuery { reason: String },
    #[error("invalid identifier for field '{field}'")]
    InvalidIdentifier { field: &'static str },
    #[error("nightly pipeline is halted: {state:?}")]
    PipelineHalted { state: PipelineHaltState },
    #[error("cannot take over slot while prior holder '{holder}' is still running")]
    PriorHolderStillRunning { holder: String },
    #[error("cannot take over slot while prior holder status is unreadable: {detail}")]
    PriorHolderUnreadable { detail: String },
    #[error("slot lease for run '{run_id}' (epoch {lease_epoch}) was lost; mutation refused")]
    SlotLeaseLost { run_id: String, lease_epoch: u64 },
    #[error("article '{article_id}' is not in waiting_to_retry state")]
    NotWaitingToRetry { article_id: String },
    #[error("invalid article transition for '{article_id}': {reason}")]
    InvalidArticleTransition { article_id: String, reason: String },
    #[error("cannot mark night succeeded while {remaining_due} due item(s) remain")]
    IncompleteNightCannotSucceed { remaining_due: usize },
    #[error("candidate folder '{version_id}/{artifact_path}' is write-once and already exists")]
    CandidateFolderWriteOnce {
        version_id: String,
        artifact_path: String,
    },
    #[error("candidate version '{version_id}' has not passed integrity checks")]
    CandidateUnchecked { version_id: String },
}

/// Metadata for a write-once candidate version folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateVersionRecord {
    pub version_id: CandidateVersionId,
    pub artifacts: BTreeMap<String, Vec<u8>>,
    pub checked_manifest_sha256: Option<String>,
    pub active: bool,
}

/// Authoritative clinical library nightly coordinator enforcing all 9 guarantees.
#[derive(Debug, Default)]
pub struct ClinicalLibraryNightly {
    articles: BTreeMap<ArticleId, ArticleState>,
    halt_state: PipelineHaltState,
    failure_code_nights: BTreeMap<FailureCode, u32>,
    consecutive_failed_or_idle_nights: u32,
    consecutive_unreadable_holder_nights: u32,
    alerts: Vec<OperatorAlert>,
    current_lease_epoch: u64,
    active_holder: Option<NightRunId>,
    candidates: BTreeMap<CandidateVersionId, CandidateVersionRecord>,
    active_version: Option<CandidateVersionId>,
    history: Vec<(NightRunId, NightOutcome)>,
    db_writes_by_run: BTreeMap<NightRunId, u64>,
}

impl ClinicalLibraryNightly {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Seeds or registers an article in an initial state.
    pub fn insert_article(&mut self, id: ArticleId, state: ArticleState) {
        self.articles.insert(id, state);
    }

    /// Acquires the nightly slot lease enforcing Rules 4, 5, 6, and 7.
    ///
    /// - Refuses automatic start if pipeline is halted (Rules 4 & 5).
    /// - Takes over ONLY when `SlotHolderStatus::Empty` or `SlotHolderStatus::Ended` (Rule 6).
    /// - Emits an operator alert on the 2nd consecutive `Unreadable` observation (Rule 7).
    pub fn acquire_slot(
        &mut self,
        run_id: NightRunId,
        prior_holder: SlotHolderStatus,
    ) -> Result<SlotLease, NightlyError> {
        if self.halt_state != PipelineHaltState::Running {
            return Err(NightlyError::PipelineHalted {
                state: self.halt_state.clone(),
            });
        }

        match prior_holder {
            SlotHolderStatus::Running { holder } => {
                self.consecutive_unreadable_holder_nights = 0;
                return Err(NightlyError::PriorHolderStillRunning {
                    holder: holder.as_str().to_owned(),
                });
            }
            SlotHolderStatus::Unreadable { detail } => {
                self.consecutive_unreadable_holder_nights =
                    self.consecutive_unreadable_holder_nights.saturating_add(1);
                if self.consecutive_unreadable_holder_nights >= UNREADABLE_HOLDER_ALERT_THRESHOLD {
                    self.alerts.push(OperatorAlert::UnreadableSlotHolder {
                        consecutive_nights: self.consecutive_unreadable_holder_nights,
                        detail: detail.clone(),
                    });
                }
                return Err(NightlyError::PriorHolderUnreadable { detail });
            }
            SlotHolderStatus::Empty | SlotHolderStatus::Ended { .. } => {
                self.consecutive_unreadable_holder_nights = 0;
            }
        }

        self.current_lease_epoch = self.current_lease_epoch.saturating_add(1);
        self.active_holder = Some(run_id.clone());
        Ok(SlotLease {
            run_id,
            lease_epoch: self.current_lease_epoch,
        })
    }

    /// Returns all articles currently eligible for nightly retry (Rule 2: `WaitingToRetry` only).
    #[must_use]
    pub fn due_retry_articles(&self) -> Vec<ArticleId> {
        self.articles
            .iter()
            .filter_map(|(id, state)| {
                matches!(state, ArticleState::WaitingToRetry { .. }).then_some(id.clone())
            })
            .collect()
    }

    /// Retries a single `WaitingToRetry` article under a live `SlotLease` (Rules 2 & 8).
    pub fn retry_waiting_article(
        &mut self,
        lease: &SlotLease,
        article_id: &ArticleId,
        target_version: CandidateVersionId,
        new_content_sha256: String,
    ) -> Result<(), NightlyError> {
        self.verify_live_lease(lease)?;
        let Some(current) = self.articles.get(article_id) else {
            return Err(NightlyError::NotWaitingToRetry {
                article_id: article_id.as_str().to_owned(),
            });
        };
        let reader_version = match current {
            ArticleState::WaitingToRetry { reader_version, .. } => *reader_version,
            ArticleState::Active { .. } | ArticleState::HumanReview { .. } => {
                return Err(NightlyError::NotWaitingToRetry {
                    article_id: article_id.as_str().to_owned(),
                });
            }
        };

        self.articles.insert(
            article_id.clone(),
            ArticleState::Active {
                version: target_version,
                content_sha256: new_content_sha256,
                reader_version,
            },
        );
        self.record_db_write(lease);
        Ok(())
    }

    /// Moves an article out of `HumanReview` or resets a `WaitingToRetry` item ONLY via
    /// explicit manual, new-content, or new-reader-version recovery trigger (Rule 1).
    pub fn recover_article(
        &mut self,
        article_id: &ArticleId,
        trigger: RecoveryTrigger,
        next_failure_code_placeholder: FailureCode,
    ) -> Result<(), NightlyError> {
        let Some(current) = self.articles.get(article_id).cloned() else {
            return Err(NightlyError::InvalidArticleTransition {
                article_id: article_id.as_str().to_owned(),
                reason: "unknown article".to_owned(),
            });
        };

        let next_reader = match (&current, &trigger) {
            (
                ArticleState::HumanReview { reader_version, .. }
                | ArticleState::WaitingToRetry { reader_version, .. },
                RecoveryTrigger::NewReaderVersion {
                    reader_version: newer,
                },
            ) => {
                if *newer <= *reader_version {
                    return Err(NightlyError::InvalidArticleTransition {
                        article_id: article_id.as_str().to_owned(),
                        reason: "reader version must strictly increase for recovery".to_owned(),
                    });
                }
                *newer
            }
            (
                ArticleState::HumanReview { reader_version, .. }
                | ArticleState::WaitingToRetry { reader_version, .. },
                RecoveryTrigger::OperatorAction { .. } | RecoveryTrigger::NewSourceContent { .. },
            ) => *reader_version,
            (ArticleState::Active { .. }, _) => {
                return Err(NightlyError::InvalidArticleTransition {
                    article_id: article_id.as_str().to_owned(),
                    reason: "active article does not need recovery".to_owned(),
                });
            }
        };

        self.articles.insert(
            article_id.clone(),
            ArticleState::WaitingToRetry {
                last_failure: next_failure_code_placeholder,
                attempts: 0,
                reader_version: next_reader,
            },
        );
        Ok(())
    }

    /// Writes a file into a write-once candidate folder under a live `SlotLease` (Rule 8).
    pub fn write_candidate_artifact(
        &mut self,
        lease: &SlotLease,
        version_id: &CandidateVersionId,
        artifact_path: &str,
        bytes: Vec<u8>,
    ) -> Result<String, NightlyError> {
        self.verify_live_lease(lease)?;
        let record = self
            .candidates
            .entry(version_id.clone())
            .or_insert_with(|| CandidateVersionRecord {
                version_id: version_id.clone(),
                artifacts: BTreeMap::new(),
                checked_manifest_sha256: None,
                active: false,
            });
        if record.artifacts.contains_key(artifact_path) {
            return Err(NightlyError::CandidateFolderWriteOnce {
                version_id: version_id.as_str().to_owned(),
                artifact_path: artifact_path.to_owned(),
            });
        }
        let digest = sha256_hex(&bytes);
        record.artifacts.insert(artifact_path.to_owned(), bytes);
        self.record_db_write(lease);
        Ok(digest)
    }

    /// Marks a candidate version integrity-checked and mints a non-copyable [`ActivationHandle`] (Rule 9).
    pub fn check_and_mint_activation_handle(
        &mut self,
        lease: &SlotLease,
        version_id: &CandidateVersionId,
    ) -> Result<ActivationHandle, NightlyError> {
        self.verify_live_lease(lease)?;
        let Some(record) = self.candidates.get_mut(version_id) else {
            return Err(NightlyError::CandidateUnchecked {
                version_id: version_id.as_str().to_owned(),
            });
        };
        if record.artifacts.is_empty() {
            return Err(NightlyError::CandidateUnchecked {
                version_id: version_id.as_str().to_owned(),
            });
        }
        let mut combined = Vec::new();
        for (path, payload) in &record.artifacts {
            combined.extend_from_slice(path.as_bytes());
            combined.push(b':');
            combined.extend_from_slice(sha256_hex(payload).as_bytes());
            combined.push(b'\n');
        }
        let manifest_sha256 = sha256_hex(&combined);
        record.checked_manifest_sha256 = Some(manifest_sha256.clone());
        self.record_db_write(lease);

        Ok(ActivationHandle {
            version_id: version_id.clone(),
            run_id: lease.run_id.clone(),
            lease_epoch: lease.lease_epoch,
            manifest_sha256,
        })
    }

    /// Consumes a non-copyable [`ActivationHandle`] to atomically activate at most ONE checked
    /// candidate version, rejecting lost leases (Rules 8 & 9).
    pub fn activate_version(&mut self, handle: ActivationHandle) -> Result<(), NightlyError> {
        if self.current_lease_epoch != handle.lease_epoch
            || self.active_holder.as_ref() != Some(&handle.run_id)
        {
            return Err(NightlyError::SlotLeaseLost {
                run_id: handle.run_id.as_str().to_owned(),
                lease_epoch: handle.lease_epoch,
            });
        }
        let is_checked = self
            .candidates
            .get(&handle.version_id)
            .and_then(|c| c.checked_manifest_sha256.as_ref())
            .is_some_and(|digest| digest == &handle.manifest_sha256);
        if !is_checked {
            return Err(NightlyError::CandidateUnchecked {
                version_id: handle.version_id.as_str().to_owned(),
            });
        }

        // Enforce single active checked version (partial unique index invariant).
        for rec in self.candidates.values_mut() {
            rec.active = false;
        }
        if let Some(target) = self.candidates.get_mut(&handle.version_id) {
            target.active = true;
        }
        self.active_version = Some(handle.version_id);
        *self.db_writes_by_run.entry(handle.run_id).or_insert(0) += 1;
        Ok(())
    }

    /// Closes a nightly run enforcing Rules 3, 4, 5, and 8.
    pub fn finish_night(
        &mut self,
        lease: SlotLease,
        outcome: NightOutcome,
    ) -> Result<(), NightlyError> {
        self.verify_live_lease(&lease)?;

        match &outcome {
            NightOutcome::Succeeded { .. } => {
                let remaining = self.due_retry_articles().len();
                if remaining > 0 {
                    return Err(NightlyError::IncompleteNightCannotSucceed {
                        remaining_due: remaining,
                    });
                }
                self.consecutive_failed_or_idle_nights = 0;
            }
            NightOutcome::Deferred {
                articles_processed, ..
            } => {
                if *articles_processed == 0 {
                    self.consecutive_failed_or_idle_nights =
                        self.consecutive_failed_or_idle_nights.saturating_add(1);
                } else {
                    self.consecutive_failed_or_idle_nights = 0;
                }
                if self.consecutive_failed_or_idle_nights >= CONSECUTIVE_FAILED_OR_IDLE_HALT_LIMIT {
                    self.halt_state = PipelineHaltState::HaltedConsecutiveFailedOrIdle {
                        consecutive_nights: self.consecutive_failed_or_idle_nights,
                    };
                }
            }
            NightOutcome::Failed { code } => {
                let count = self.failure_code_nights.entry(code.clone()).or_insert(0);
                *count = count.saturating_add(1);
                let observed = *count;
                self.consecutive_failed_or_idle_nights =
                    self.consecutive_failed_or_idle_nights.saturating_add(1);

                if observed >= REPEATED_FAILURE_CODE_HALT_LIMIT {
                    self.halt_state = PipelineHaltState::HaltedRepeatedFailureCode {
                        code: code.clone(),
                        nights_observed: observed,
                    };
                } else if self.consecutive_failed_or_idle_nights
                    >= CONSECUTIVE_FAILED_OR_IDLE_HALT_LIMIT
                {
                    self.halt_state = PipelineHaltState::HaltedConsecutiveFailedOrIdle {
                        consecutive_nights: self.consecutive_failed_or_idle_nights,
                    };
                }
            }
        }

        self.history.push((lease.run_id.clone(), outcome));
        self.active_holder = None;
        Ok(())
    }

    /// Explicit operator resolution clearing a halted pipeline state and resetting failure streaks.
    pub fn resolve_operator_halt(&mut self, _operator_id: &str) {
        self.halt_state = PipelineHaltState::Running;
        self.failure_code_nights.clear();
        self.consecutive_failed_or_idle_nights = 0;
    }

    #[must_use]
    pub const fn halt_state(&self) -> &PipelineHaltState {
        &self.halt_state
    }

    #[must_use]
    pub fn active_version(&self) -> Option<&CandidateVersionId> {
        self.active_version.as_ref()
    }

    /// Counts how many candidate versions are marked `active == true` (MUST always be `<= 1`).
    #[must_use]
    pub fn active_version_count(&self) -> usize {
        self.candidates.values().filter(|c| c.active).count()
    }

    #[must_use]
    pub fn alerts(&self) -> &[OperatorAlert] {
        &self.alerts
    }

    #[must_use]
    pub fn history(&self) -> &[(NightRunId, NightOutcome)] {
        &self.history
    }

    #[must_use]
    pub fn article(&self, id: &ArticleId) -> Option<&ArticleState> {
        self.articles.get(id)
    }

    #[must_use]
    pub fn db_writes_for_run(&self, run_id: &NightRunId) -> u64 {
        self.db_writes_by_run.get(run_id).copied().unwrap_or(0)
    }

    #[must_use]
    pub fn candidate(&self, version_id: &CandidateVersionId) -> Option<&CandidateVersionRecord> {
        self.candidates.get(version_id)
    }

    fn verify_live_lease(&self, lease: &SlotLease) -> Result<(), NightlyError> {
        if self.current_lease_epoch != lease.lease_epoch
            || self.active_holder.as_ref() != Some(&lease.run_id)
        {
            return Err(NightlyError::SlotLeaseLost {
                run_id: lease.run_id.as_str().to_owned(),
                lease_epoch: lease.lease_epoch,
            });
        }
        Ok(())
    }

    fn record_db_write(&mut self, lease: &SlotLease) {
        *self
            .db_writes_by_run
            .entry(lease.run_id.clone())
            .or_insert(0) += 1;
    }
}
