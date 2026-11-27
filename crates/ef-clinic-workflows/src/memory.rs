//! Longitudinal patient memory & background rounds upkeep:
//! - Monotonic rounds watermark progression (aborted/failed runs never advance watermark)
//! - Quiet no-op runs (zero wording churn or duplicate entries when nothing changed)
//! - Dated provenance (`chart_record` vs `physician_instruction`)
//! - Disagreement preservation (conflicting chart record & physician instruction coexist)
//! - History-preserving retirement (superseded facts retired with resolution date/reason)
//! - Model-facing text hygiene (zero tracker IDs, zero unoffered tool references)

use ef_core::WorkspaceId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Distinct authority tag attached to every patient memory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryAuthority {
    ChartRecord,
    PhysicianInstruction,
}

/// Lifecycle state of a memory item (superseded facts are retired, never deleted).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MemoryEntryState {
    Active,
    Retired { resolved_at: String, reason: String },
}

/// One dated longitudinal memory item for a patient workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub entry_id: String,
    pub topic: String,
    pub statement: String,
    pub observed_at: String,
    pub authority: MemoryAuthority,
    pub state: MemoryEntryState,
}

/// Newly arrived clinical observation evaluated by background rounds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundsObservation {
    pub record_seq: u64,
    pub topic: String,
    pub statement: String,
    pub observed_at: String,
    pub authority: MemoryAuthority,
}

/// Outcome of executing a background rounds upkeep pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoundsRunOutcome {
    /// No material changes arrived since prior watermark -> zero edits.
    QuietNoOp { watermark: u64 },
    /// Completed cleanly and advanced watermark monotonically.
    Updated {
        new_watermark: u64,
        added_entries: usize,
        disagreements_preserved: usize,
    },
    /// Aborted/failed mid-execution -> watermark strictly unchanged.
    Aborted { unchanged_watermark: u64 },
}

/// Violation detected by model-facing text hygiene checks.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TextHygieneViolation {
    #[error("model-facing text contains forbidden tracker ID '{token}'")]
    TrackerIdLeak { token: String },
    #[error("model-facing text references unoffered tool '{tool_name}'")]
    UnofferedToolReference { tool_name: String },
}

/// Longitudinal memory store for one patient workspace.
#[derive(Debug, Clone)]
pub struct PatientMemoryStore {
    workspace_id: WorkspaceId,
    rounds_watermark: u64,
    entries: Vec<MemoryEntry>,
    next_entry_seq: u64,
    mutation_count: u64,
}

impl PatientMemoryStore {
    #[must_use]
    pub fn new(workspace_id: WorkspaceId) -> Self {
        Self {
            workspace_id,
            rounds_watermark: 0,
            entries: Vec::new(),
            next_entry_seq: 1,
            mutation_count: 0,
        }
    }

    /// Inserts an explicit dated memory entry (e.g., direct physician instruction).
    pub fn record_entry(
        &mut self,
        topic: impl Into<String>,
        statement: impl Into<String>,
        observed_at: impl Into<String>,
        authority: MemoryAuthority,
    ) -> String {
        let entry_id = format!("mem-{}", self.next_entry_seq);
        self.next_entry_seq = self.next_entry_seq.saturating_add(1);
        self.entries.push(MemoryEntry {
            entry_id: entry_id.clone(),
            topic: topic.into(),
            statement: statement.into(),
            observed_at: observed_at.into(),
            authority,
            state: MemoryEntryState::Active,
        });
        self.mutation_count = self.mutation_count.saturating_add(1);
        entry_id
    }

    /// Runs background rounds over `observations` arriving after `rounds_watermark`.
    ///
    /// - If `abort_mid_run` is true, simulates a mid-run failure: makes zero committed edits
    ///   and NEVER advances `rounds_watermark`.
    /// - If no observations have `record_seq > rounds_watermark` (or all duplicate existing active
    ///   facts), returns `QuietNoOp` with zero mutations.
    /// - When a `ChartRecord` contradicts an active `PhysicianInstruction` on the same topic
    ///   (or vice versa), BOTH entries remain `Active` with distinct dates and authority tags!
    pub fn run_rounds(
        &mut self,
        observations: &[RoundsObservation],
        abort_mid_run: bool,
    ) -> RoundsRunOutcome {
        let pending: Vec<&RoundsObservation> = observations
            .iter()
            .filter(|obs| obs.record_seq > self.rounds_watermark)
            .collect();

        if pending.is_empty() {
            return RoundsRunOutcome::QuietNoOp {
                watermark: self.rounds_watermark,
            };
        }

        if abort_mid_run {
            return RoundsRunOutcome::Aborted {
                unchanged_watermark: self.rounds_watermark,
            };
        }

        let mut added = 0usize;
        let mut disagreements = 0usize;
        let mut max_seq = self.rounds_watermark;

        for obs in pending {
            max_seq = max_seq.max(obs.record_seq);

            // Quiet no-op deduplication: exact duplicate of an already-active entry causes zero churn.
            let already_active = self.entries.iter().any(|e| {
                e.state == MemoryEntryState::Active
                    && e.topic == obs.topic
                    && e.statement == obs.statement
                    && e.authority == obs.authority
            });
            if already_active {
                continue;
            }

            // Disagreement preservation: opposite authority on same topic is kept alongside, never overwritten.
            let has_opposite_authority = self.entries.iter().any(|e| {
                e.state == MemoryEntryState::Active
                    && e.topic == obs.topic
                    && e.authority != obs.authority
            });
            if has_opposite_authority {
                disagreements = disagreements.saturating_add(1);
            }

            let entry_id = format!("mem-{}", self.next_entry_seq);
            self.next_entry_seq = self.next_entry_seq.saturating_add(1);
            self.entries.push(MemoryEntry {
                entry_id,
                topic: obs.topic.clone(),
                statement: obs.statement.clone(),
                observed_at: obs.observed_at.clone(),
                authority: obs.authority,
                state: MemoryEntryState::Active,
            });
            added = added.saturating_add(1);
            self.mutation_count = self.mutation_count.saturating_add(1);
        }

        self.rounds_watermark = max_seq;
        if added == 0 {
            RoundsRunOutcome::QuietNoOp {
                watermark: self.rounds_watermark,
            }
        } else {
            RoundsRunOutcome::Updated {
                new_watermark: self.rounds_watermark,
                added_entries: added,
                disagreements_preserved: disagreements,
            }
        }
    }

    /// History-preserving retirement: retires ONE entry with resolution date/reason without
    /// deleting it or mutating any unrelated active entries.
    pub fn retire_entry(
        &mut self,
        entry_id: &str,
        resolved_at: impl Into<String>,
        reason: impl Into<String>,
    ) -> bool {
        for entry in &mut self.entries {
            if entry.entry_id == entry_id && entry.state == MemoryEntryState::Active {
                entry.state = MemoryEntryState::Retired {
                    resolved_at: resolved_at.into(),
                    reason: reason.into(),
                };
                self.mutation_count = self.mutation_count.saturating_add(1);
                return true;
            }
        }
        false
    }

    /// Returns all active entries matching `topic` (both sides of a disagreement stay visible).
    #[must_use]
    pub fn active_entries_for_topic(&self, topic: &str) -> Vec<&MemoryEntry> {
        self.entries
            .iter()
            .filter(|e| e.state == MemoryEntryState::Active && e.topic == topic)
            .collect()
    }

    /// All memory entries including retired history.
    #[must_use]
    pub fn all_entries(&self) -> &[MemoryEntry] {
        &self.entries
    }

    /// Current monotonic rounds watermark.
    #[must_use]
    pub const fn rounds_watermark(&self) -> u64 {
        self.rounds_watermark
    }

    /// Total number of mutations applied to memory entries (used to prove QuietNoOp = 0 edits).
    #[must_use]
    pub const fn mutation_count(&self) -> u64 {
        self.mutation_count
    }

    /// Owning patient workspace ID.
    #[must_use]
    pub const fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }
}

/// Checks rendered model-facing prompt/skill text for zero issue-tracker IDs and zero
/// references to tools (`tool:<name>`) absent from `offered_tools`.
pub fn verify_model_facing_text_hygiene(
    rendered_text: &str,
    offered_tools: &[&str],
) -> Result<(), Vec<TextHygieneViolation>> {
    let mut violations = Vec::new();

    for raw_token in rendered_text.split_whitespace() {
        let token = raw_token
            .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != ':' && c != '_');
        if looks_like_tracker_id(token) {
            violations.push(TextHygieneViolation::TrackerIdLeak {
                token: token.to_owned(),
            });
        }
        if let Some(tool_name) = token.strip_prefix("tool:")
            && !tool_name.is_empty()
            && !offered_tools.contains(&tool_name)
        {
            violations.push(TextHygieneViolation::UnofferedToolReference {
                tool_name: tool_name.to_owned(),
            });
        }
    }

    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

fn looks_like_tracker_id(token: &str) -> bool {
    let Some((prefix, suffix)) = token.split_once('-') else {
        return false;
    };
    let valid_prefix = (2..=5).contains(&prefix.len())
        && prefix.chars().all(|c| c.is_ascii_uppercase())
        && !matches!(
            prefix,
            "UTF"
                | "ISO"
                | "SHA"
                | "FHIR"
                | "HL"
                | "CCDA"
                | "ICD"
                | "CPT"
                | "LOINC"
                | "CMP"
                | "BMP"
                | "CBC"
                | "SUD"
                | "CFR"
        );
    let valid_suffix = !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit());
    valid_prefix && valid_suffix
}
