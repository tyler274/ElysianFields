//! Practice-scoped three-tier patient access control, honest patient roster states,
//! non-modal cited-source reading panel state machine, record status classification,
//! cross-patient isolation, withdrawn/stale redaction, and 5,000-row synthetic lab fixture.

use std::collections::BTreeMap;

use ef_core::{Locale, PhysicianId, PracticeId, RequestId, SourceKey, WorkspaceId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Three-tier workspace membership checked server-side on every clinical request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MembershipTier {
    /// Patient belongs to the physician's practice and is on this physician's panel.
    Allowed,
    /// Patient belongs to the physician's practice, demographic coordination only.
    ClinicDemographic,
    /// Patient belongs to another practice or does not exist (`404 Not Found` non-disclosure).
    Denied,
}

/// Errors returned by workspace access and citation evidence lookups.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WorkspaceAccessError {
    /// Cross-practice or missing resource: always returns `404 Not Found` (`Cache-Control: no-store`).
    #[error("resource not found")]
    NotFound,
    /// `ClinicDemographic` standing forbids clinical chart, transcript, or citation body disclosure.
    #[error("demographic-only membership cannot inspect clinical records or transcripts")]
    ClinicalDisclosureForbidden,
    /// Cited source was withdrawn or superseded after answer generation.
    #[error("cited source has been withdrawn or redacted")]
    SourceWithdrawn,
    /// Upstream authorization or persistence store outage (`503 Service Unavailable`).
    #[error("patient access store unavailable: {message}")]
    StoreUnavailable { message: String },
}

impl WorkspaceAccessError {
    /// Required HTTP status code for this error (`404`, `403`, or `503`).
    #[must_use]
    pub const fn http_status_code(&self) -> u16 {
        match self {
            Self::NotFound | Self::SourceWithdrawn => 404,
            Self::ClinicalDisclosureForbidden => 403,
            Self::StoreUnavailable { .. } => 503,
        }
    }
}

/// Patient roster entry authorized for the signed-in physician's practice/panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatientWorkspaceSummary {
    pub patient_workspace_id: WorkspaceId,
    pub practice_id: PracticeId,
    pub display_name: String,
    pub cursor: String,
    pub membership: MembershipTier,
}

/// Completeness flag for patient roster reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RosterAvailability {
    Complete,
    Incomplete { may_be_missing_someone: bool },
}

/// Honest UI state for the patient list / roster finder.
///
/// Failed backend reads (`Failed`) are a distinct variant and NEVER render as `Empty`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PatientRosterState {
    Loaded {
        patients: Vec<PatientWorkspaceSummary>,
        availability: RosterAvailability,
    },
    Empty {
        query: Option<String>,
    },
    Failed {
        message: String,
    },
}

/// Clinical status badge classification (settled vs flagged).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordStatusClass {
    Settled,
    Flagged,
}

/// Single owner classifying raw clinical record status strings (case-insensitive, `_` -> `-`).
#[must_use]
pub fn classify_record_status(raw: &str) -> RecordStatusClass {
    let normalized = raw.trim().to_ascii_lowercase().replace('_', "-");
    match normalized.as_str() {
        "preliminary" | "inactive" | "refuted" | "entered-in-error" => RecordStatusClass::Flagged,
        _ => RecordStatusClass::Settled,
    }
}

/// Origin provenance of a cited document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CitationOrigin {
    SourceRecord,
    PreparedByAssistant,
}

/// Target granularity for an inline citation chip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CitationTarget {
    WholeRecord,
    Passage { quote: String },
    PassageNotFound,
}

/// Structured segment inside a clinical assistant answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AnswerSegment {
    Text {
        text: String,
    },
    Citation {
        source_key: SourceKey,
        number: u16,
        label: String,
        quote: String,
        target: CitationTarget,
    },
}

/// Ordered readable section inside a cited clinical document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadablePart {
    pub label: String,
    pub text: String,
    /// When a specific attachment page is corrupted/unreadable, stays explicit without hiding siblings.
    pub unreadable_notice: Option<String>,
}

/// One row in a structured laboratory/clinical record table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredFieldRow {
    pub key: String,
    pub name: String,
    pub value: String,
    pub unit: String,
    pub range: String,
    pub status: String,
}

/// Full cited source payload returned by `GET /patient-workspaces/{ws}/chats/{chat}/sources/{key}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CitedSourcePayload {
    pub source_key: SourceKey,
    pub workspace_id: WorkspaceId,
    pub number: u16,
    pub source_label: String,
    pub title: String,
    pub record_type: String,
    pub recorded_at: String,
    pub record_status: String,
    pub status_class: RecordStatusClass,
    pub origin: CitationOrigin,
    pub original_language: Locale,
    pub quote: String,
    pub body: String,
    pub readable_parts: Vec<ReadablePart>,
    pub fields: Vec<StructuredFieldRow>,
    pub raw_content: Option<String>,
    pub external_url: Option<String>,
}

/// Citation chip handle selected by the clinician.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CitationChipRef {
    pub source_key: SourceKey,
    pub number: u16,
    pub label: String,
}

/// Fetch result delivered asynchronously to the non-modal citation reading panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CitationFetchOutcome {
    Loaded(Box<CitedSourcePayload>),
    Unavailable {
        eyebrow: String,
        title: String,
        reason: String,
    },
    Failed {
        message: String,
    },
}

/// Non-modal cited-source reading panel state machine from Chapter 2 §2.3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CitationPanelState {
    Idle,
    Loading {
        request_id: RequestId,
        chip: CitationChipRef,
    },
    Loaded {
        request_id: RequestId,
        chip: CitationChipRef,
        source: Box<CitedSourcePayload>,
    },
    /// Withdrawn, redacted, or revoked source: immediately clears all prior clinical text.
    Unavailable {
        eyebrow: String,
        title: String,
        reason: String,
    },
    Failed {
        request_id: RequestId,
        chip: CitationChipRef,
        message: String,
    },
}

/// Controller enforcing monotonic `request_id` fencing so rapid chip clicks never show stale records.
#[derive(Debug, Clone)]
pub struct CitationPanelController {
    next_request_id: RequestId,
    active_request_id: Option<RequestId>,
    state: CitationPanelState,
}

impl Default for CitationPanelController {
    fn default() -> Self {
        Self::new()
    }
}

impl CitationPanelController {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            next_request_id: RequestId::new(1),
            active_request_id: None,
            state: CitationPanelState::Idle,
        }
    }

    /// Begins loading a citation chip and returns its monotonic `RequestId`.
    pub fn open_chip(&mut self, chip: CitationChipRef) -> RequestId {
        let request_id = self.next_request_id;
        self.next_request_id = request_id.next();
        self.active_request_id = Some(request_id);
        self.state = CitationPanelState::Loading { request_id, chip };
        request_id
    }

    /// Applies an incoming citation fetch response; discards any stale `request_id`.
    ///
    /// Returns `true` when applied, `false` when discarded as stale.
    pub fn apply_response(&mut self, request_id: RequestId, outcome: CitationFetchOutcome) -> bool {
        if self.active_request_id != Some(request_id) {
            return false;
        }
        let chip = match &self.state {
            CitationPanelState::Loading { chip, .. }
            | CitationPanelState::Loaded { chip, .. }
            | CitationPanelState::Failed { chip, .. } => chip.clone(),
            CitationPanelState::Idle | CitationPanelState::Unavailable { .. } => return false,
        };

        self.state = match outcome {
            CitationFetchOutcome::Loaded(source) => CitationPanelState::Loaded {
                request_id,
                chip,
                source,
            },
            CitationFetchOutcome::Unavailable {
                eyebrow,
                title,
                reason,
            } => CitationPanelState::Unavailable {
                eyebrow,
                title,
                reason,
            },
            CitationFetchOutcome::Failed { message } => CitationPanelState::Failed {
                request_id,
                chip,
                message,
            },
        };
        true
    }

    /// Immediately clears any displayed clinical text upon access revocation or source withdrawal.
    pub fn mark_unavailable(&mut self, eyebrow: String, title: String, reason: String) {
        self.active_request_id = None;
        self.state = CitationPanelState::Unavailable {
            eyebrow,
            title,
            reason,
        };
    }

    /// Closes the non-modal reading panel (`Idle`).
    pub fn close(&mut self) {
        self.active_request_id = None;
        self.state = CitationPanelState::Idle;
    }

    #[must_use]
    pub const fn state(&self) -> &CitationPanelState {
        &self.state
    }
}

#[derive(Debug, Clone)]
struct StoredWorkspace {
    workspace_id: WorkspaceId,
    practice_id: PracticeId,
    display_name: String,
    panel_physicians: Vec<PhysicianId>,
}

#[derive(Debug, Clone)]
struct StoredCitation {
    payload: CitedSourcePayload,
    withdrawn: bool,
}

/// Authoritative practice-scoped workspace access and cited-source store.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceEvidenceStore {
    workspaces: BTreeMap<WorkspaceId, StoredWorkspace>,
    citations: BTreeMap<(WorkspaceId, SourceKey), StoredCitation>,
    store_outage: Option<String>,
}

impl WorkspaceEvidenceStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a patient workspace inside a practice with its assigned panel physicians.
    pub fn register_workspace(
        &mut self,
        workspace_id: WorkspaceId,
        practice_id: PracticeId,
        display_name: impl Into<String>,
        panel_physicians: Vec<PhysicianId>,
    ) {
        self.workspaces.insert(
            workspace_id.clone(),
            StoredWorkspace {
                workspace_id,
                practice_id,
                display_name: display_name.into(),
                panel_physicians,
            },
        );
    }

    /// Stores a retained clinical source record owned strictly by `payload.workspace_id`.
    pub fn insert_source(&mut self, payload: CitedSourcePayload) {
        let key = (payload.workspace_id.clone(), payload.source_key.clone());
        self.citations.insert(
            key,
            StoredCitation {
                payload,
                withdrawn: false,
            },
        );
    }

    /// Withdraws a cited source server-side so subsequent opens redact clinical text (`Unavailable`).
    pub fn withdraw_source(&mut self, workspace_id: &WorkspaceId, source_key: &SourceKey) -> bool {
        if let Some(entry) = self
            .citations
            .get_mut(&(workspace_id.clone(), source_key.clone()))
        {
            entry.withdrawn = true;
            true
        } else {
            false
        }
    }

    /// Injects or clears a simulated backend persistence outage (`503`).
    pub fn set_store_outage(&mut self, message: Option<String>) {
        self.store_outage = message;
    }

    /// Single owner determining 3-tier membership for `(practice_id, physician_id, workspace_id)`.
    pub fn membership_for(
        &self,
        practice_id: &PracticeId,
        physician_id: &PhysicianId,
        workspace_id: &WorkspaceId,
    ) -> Result<MembershipTier, WorkspaceAccessError> {
        if let Some(message) = &self.store_outage {
            return Err(WorkspaceAccessError::StoreUnavailable {
                message: message.clone(),
            });
        }
        let Some(ws) = self.workspaces.get(workspace_id) else {
            return Ok(MembershipTier::Denied);
        };
        if &ws.practice_id != practice_id {
            return Ok(MembershipTier::Denied);
        }
        if ws.panel_physicians.contains(physician_id) {
            Ok(MembershipTier::Allowed)
        } else {
            Ok(MembershipTier::ClinicDemographic)
        }
    }

    /// Enforces full clinical workspace access (`Allowed` only; `Denied` -> `404` non-disclosure).
    pub fn require_clinical_access(
        &self,
        practice_id: &PracticeId,
        physician_id: &PhysicianId,
        workspace_id: &WorkspaceId,
    ) -> Result<(), WorkspaceAccessError> {
        match self.membership_for(practice_id, physician_id, workspace_id)? {
            MembershipTier::Allowed => Ok(()),
            MembershipTier::ClinicDemographic => {
                Err(WorkspaceAccessError::ClinicalDisclosureForbidden)
            }
            MembershipTier::Denied => Err(WorkspaceAccessError::NotFound),
        }
    }

    /// Lists authorized Practice patients, distinguishing honest `Empty` vs `Failed` vs `Incomplete`.
    #[must_use]
    pub fn query_patient_roster(
        &self,
        practice_id: &PracticeId,
        physician_id: &PhysicianId,
        query: Option<&str>,
        partial_source_outage: bool,
    ) -> PatientRosterState {
        if let Some(message) = &self.store_outage {
            return PatientRosterState::Failed {
                message: message.clone(),
            };
        }

        let needle = query.map(|q| q.trim().to_ascii_lowercase());
        let mut patients = Vec::new();
        for (idx, ws) in self.workspaces.values().enumerate() {
            if &ws.practice_id != practice_id {
                continue;
            }
            if let Some(ref q) = needle
                && !q.is_empty()
                && !ws.display_name.to_ascii_lowercase().contains(q)
            {
                continue;
            }
            let membership = if ws.panel_physicians.contains(physician_id) {
                MembershipTier::Allowed
            } else {
                MembershipTier::ClinicDemographic
            };
            patients.push(PatientWorkspaceSummary {
                patient_workspace_id: ws.workspace_id.clone(),
                practice_id: ws.practice_id.clone(),
                display_name: ws.display_name.clone(),
                cursor: format!("cursor-{idx}"),
                membership,
            });
        }

        if patients.is_empty() {
            return PatientRosterState::Empty {
                query: query.map(str::to_owned),
            };
        }

        let availability = if partial_source_outage {
            RosterAvailability::Incomplete {
                may_be_missing_someone: true,
            }
        } else {
            RosterAvailability::Complete
        };
        PatientRosterState::Loaded {
            patients,
            availability,
        }
    }

    /// Resolves a cited source enforcing practice isolation, panel access, patient ownership,
    /// and withdrawn-record redaction.
    pub fn fetch_cited_source(
        &self,
        practice_id: &PracticeId,
        physician_id: &PhysicianId,
        workspace_id: &WorkspaceId,
        source_key: &SourceKey,
    ) -> Result<CitedSourcePayload, WorkspaceAccessError> {
        self.require_clinical_access(practice_id, physician_id, workspace_id)?;

        // Enforces cross-patient isolation: lookup is strictly keyed by (workspace_id, source_key).
        let Some(entry) = self
            .citations
            .get(&(workspace_id.clone(), source_key.clone()))
        else {
            return Err(WorkspaceAccessError::NotFound);
        };

        if entry.withdrawn {
            return Err(WorkspaceAccessError::SourceWithdrawn);
        }

        Ok(entry.payload.clone())
    }
}

/// Generates a deterministic 100% synthetic 5,000-row clinical laboratory citation fixture (`[12]`).
pub fn synthetic_5000_row_lab_source(
    workspace_id: WorkspaceId,
    source_key: SourceKey,
) -> CitedSourcePayload {
    const ANALYTES: [(&str, &str, &str, &str); 5] = [
        ("Sodium", "139.2", "mEq/L", "136-145"),
        ("Potassium", "4.2", "mEq/L", "3.5-5.1"),
        ("Creatinine", "1.02", "mg/dL", "0.7-1.3"),
        ("Bicarbonate", "25.0", "mEq/L", "22-29"),
        ("Hemoglobin", "13.8", "g/dL", "12.0-16.0"),
    ];
    let mut fields = Vec::with_capacity(5_000);
    for idx in 0..5_000usize {
        let (name, value, unit, range) = ANALYTES[idx % ANALYTES.len()];
        fields.push(StructuredFieldRow {
            key: format!("lab-row-{idx:04}"),
            name: name.to_owned(),
            value: value.to_owned(),
            unit: unit.to_owned(),
            range: range.to_owned(),
            status: "final".to_owned(),
        });
    }

    CitedSourcePayload {
        source_key,
        workspace_id,
        number: 12,
        source_label: "Synthetic Longitudinal Chemistry Ledger [12]".to_owned(),
        title: "Synthetic 5,000-Row Clinical Laboratory Table".to_owned(),
        record_type: "laboratory_panel".to_owned(),
        recorded_at: "2026-01-15T14:30:00Z".to_owned(),
        record_status: "final".to_owned(),
        status_class: classify_record_status("final"),
        origin: CitationOrigin::SourceRecord,
        original_language: Locale::En,
        quote: "Sodium 139.2 mEq/L, Potassium 4.2 mEq/L, Creatinine 1.02 mg/dL".to_owned(),
        body: "Synthetic 5,000-row structured clinical laboratory table for M2 virtualization verification."
            .to_owned(),
        readable_parts: vec![ReadablePart {
            label: "Summary".to_owned(),
            text: "5,000 synthetic structured chemistry and hematology rows.".to_owned(),
            unreadable_notice: None,
        }],
        fields,
        raw_content: None,
        external_url: None,
    }
}
