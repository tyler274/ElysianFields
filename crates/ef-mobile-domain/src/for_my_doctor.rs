//! "For my doctor" curated sharing state machine proving all **13 formal `Shared.tla` invariants**
//! plus the **42 CFR Part 2 written consent gate** from Chapter 2 §2.5.

use std::collections::{BTreeMap, BTreeSet};

use ef_core::PracticeId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::account::MobileAccountId;
use crate::records::{ConfidentialityTag, PART2_REDISCLOSURE_NOTICE, PatientRecordStore};

/// Mandatory pre-confirmation disclosure text (`DisclosureBeforeSend`).
pub const SEND_IRREVERSIBLE_DISCLOSURE: &str = "Once confirmed, this send cannot be taken back. Do not use this for urgent medical needs or questions requiring an answer.";

/// Connected-care link standing for one practice as observed by Curie Mobile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PracticeLinkStatus {
    NotInvited,
    Invited,
    Connected,
    Ended,
}

/// Explicit 42 CFR Part 2 written consent confirmation provided at send confirmation time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum Part2ConsentConfirmation {
    /// No Part 2 written consent confirmed.
    None,
    /// Explicit written consent covering `Part2Sud` records (excludes SUD counseling notes).
    SudRecordsOnly { consented_at: String },
    /// Explicit written consent covering `Part2Sud` records AND separate consent for `Part2SudCounselingNotes`.
    SudAndCounselingNotes { consented_at: String },
}

/// A patient-authored personal note in `To send`, optionally linked to specific records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorNote {
    pub note_id: String,
    pub text: String,
    pub linked_record_ids: Vec<String>,
    pub revision: u64,
    pub sync_state: NoteSaveState,
}

/// Honest persistence status for notes and marks (`SaveHonesty`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteSaveState {
    UnsavedLocal,
    Saved,
}

/// Immutable frozen record snapshot inside a confirmed send (`SendHoldsExactImmutableContent`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrozenSharedRecord {
    pub record_id: String,
    pub version: u64,
    pub title: String,
    pub summary: String,
    pub confidentiality: ConfidentialityTag,
    pub part2_notice: Option<String>,
}

/// Immutable confirmed send payload frozen at confirmation time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrozenSendSnapshot {
    pub send_id: String,
    pub idempotency_key: String,
    pub account_id: MobileAccountId,
    pub practice_id: PracticeId,
    pub records: Vec<FrozenSharedRecord>,
    pub notes: Vec<DoctorNote>,
    pub part2_notice_attached: Option<String>,
    pub part2_consent: Part2ConsentConfirmation,
}

/// Honest delivery status (`SentAndReceivedTruth`, `ConnectionEndStopsUncertain`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendDeliveryStatus {
    /// Client initiated send; awaiting server persistence ACK.
    Sending,
    /// Network timeout after initiation; eligible for idempotent `Try again` ONLY while Connected.
    Uncertain,
    /// Persisted by server; practice has not yet picked it up.
    Sent,
    /// Practice explicitly confirmed receipt.
    ReceivedByCareTeam,
    /// Halted because connection ended while `Sending`/`Uncertain` (`Try again` removed).
    StoppedConnectionEnded,
}

/// Record of a send attempt or confirmed send in the patient's history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorSendRecord {
    pub snapshot: FrozenSendSnapshot,
    pub status: SendDeliveryStatus,
    pub try_again_available: bool,
}

/// Simulated transport result when confirming a send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendTransportOutcome {
    /// Server persisted the send cleanly (`Sent`).
    PersistedSent,
    /// Network dropped after request left device (`Uncertain`; retryable with same key).
    NetworkUncertain,
}

/// Errors returned by `For my doctor` curation and confirmation operations.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ForMyDoctorError {
    #[error("disclosure must be presented and acknowledged before confirming a send")]
    DisclosureNotAcknowledged,
    #[error("cannot send: practice '{practice_id}' is not in Connected state (status: {status:?})")]
    PracticeNotConnected {
        practice_id: String,
        status: PracticeLinkStatus,
    },
    #[error("cannot confirm an empty send with zero records and zero notes")]
    NothingSelectedToSend,
    #[error("record '{record_id}' is not marked or no longer available")]
    RecordNotAvailable { record_id: String },
    #[error("note '{note_id}' not found in To send")]
    NoteNotFound { note_id: String },
    #[error(
        "optimistic concurrency conflict on note '{note_id}': expected rev {expected}, current rev {current}"
    )]
    NoteSaveConflict {
        note_id: String,
        expected: u64,
        current: u64,
    },
    #[error(
        "42 CFR Part 2 written consent is required before sending Part 2 SUD record '{record_id}'"
    )]
    Part2WrittenConsentRequired { record_id: String },
    #[error(
        "separate 42 CFR § 2.31(b) written consent is required before sending SUD counseling notes '{record_id}'"
    )]
    Part2CounselingNotesConsentRequired { record_id: String },
    #[error("send '{send_id}' is not eligible for Try again retry")]
    RetryNotAvailable { send_id: String },
    #[error("send '{send_id}' not found")]
    SendNotFound { send_id: String },
}

/// Violation of one of the 13 formal `Shared.tla` invariants.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SharedInvariantViolation {
    #[error("NothingReachesWithoutConfirm violated: unconfirmed transfer detected")]
    NothingReachesWithoutConfirm,
    #[error(
        "SendToOneConnectedPractice violated: send '{send_id}' targeted non-connected practice"
    )]
    SendToOneConnectedPractice { send_id: String },
    #[error("SendReachesAtMostOnce violated: duplicate delivery for key '{idempotency_key}'")]
    SendReachesAtMostOnce { idempotency_key: String },
    #[error("ToSendExact violated: marked record '{record_id}' does not exist in active records")]
    ToSendExact { record_id: String },
    #[error(
        "ConnectionEndStopsUncertain violated: send '{send_id}' still offers retry after disconnect"
    )]
    ConnectionEndStopsUncertain { send_id: String },
}

/// Idempotent practice-side staging receiver ensuring `SendReachesAtMostOnce`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PracticeSendReceiver {
    received_by_key: BTreeMap<String, FrozenSendSnapshot>,
    picked_up_keys: BTreeSet<String>,
    delivery_counts: BTreeMap<String, u32>,
}

impl PracticeSendReceiver {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Accepts a confirmed send idempotently; duplicate retries never store a second copy.
    pub fn accept_idempotent(&mut self, snapshot: FrozenSendSnapshot) -> bool {
        let key = snapshot.idempotency_key.clone();
        if self.received_by_key.contains_key(&key) {
            return false;
        }
        self.received_by_key.insert(key.clone(), snapshot);
        self.delivery_counts.insert(key, 1);
        true
    }

    /// Confirms practice care-team pickup for `idempotency_key`.
    pub fn confirm_care_team_pickup(&mut self, idempotency_key: &str) -> bool {
        if self.received_by_key.contains_key(idempotency_key) {
            self.picked_up_keys.insert(idempotency_key.to_owned());
            true
        } else {
            false
        }
    }

    /// Scrubs all staging transfer records belonging to `account_id` (mandatory on account deletion/replay).
    pub fn scrub_account_staging(&mut self, account_id: &MobileAccountId) -> usize {
        let keys_to_remove: Vec<String> = self
            .received_by_key
            .iter()
            .filter(|(_, snap)| &snap.account_id == account_id)
            .map(|(k, _)| k.clone())
            .collect();
        let removed = keys_to_remove.len();
        for key in keys_to_remove {
            self.received_by_key.remove(&key);
            self.picked_up_keys.remove(&key);
            self.delivery_counts.remove(&key);
        }
        removed
    }

    #[must_use]
    pub fn delivery_count(&self, idempotency_key: &str) -> u32 {
        self.delivery_counts
            .get(idempotency_key)
            .copied()
            .unwrap_or(0)
    }

    #[must_use]
    pub fn snapshot(&self, idempotency_key: &str) -> Option<&FrozenSendSnapshot> {
        self.received_by_key.get(idempotency_key)
    }

    #[must_use]
    pub fn total_received_count(&self) -> usize {
        self.received_by_key.len()
    }

    #[must_use]
    pub fn is_picked_up(&self, idempotency_key: &str) -> bool {
        self.picked_up_keys.contains(idempotency_key)
    }
}

/// Patient's `For my doctor` workspace (`To send` curation + immutable confirmed send history).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForMyDoctorWorkspace {
    account_id: MobileAccountId,
    connections: BTreeMap<PracticeId, PracticeLinkStatus>,
    marked_record_ids: BTreeSet<String>,
    notes: BTreeMap<String, DoctorNote>,
    sends: Vec<DoctorSendRecord>,
    notices: Vec<String>,
    next_send_seq: u64,
}

impl ForMyDoctorWorkspace {
    #[must_use]
    pub fn new(account_id: MobileAccountId) -> Self {
        Self {
            account_id,
            connections: BTreeMap::new(),
            marked_record_ids: BTreeSet::new(),
            notes: BTreeMap::new(),
            sends: Vec::new(),
            notices: Vec::new(),
            next_send_seq: 1,
        }
    }

    /// Connects a practice (`JoinAndLeaveSafe`: never auto-sends historical `To send` items).
    pub fn connect_practice(&mut self, practice_id: PracticeId) {
        self.connections
            .insert(practice_id, PracticeLinkStatus::Connected);
    }

    /// Ends a practice connection (`ConnectionEndStopsUncertain`, `JoinAndLeaveSafe`).
    ///
    /// - Stops retrying any `Sending`/`Uncertain` send to `practice_id` and removes `Try again`.
    /// - Preserves unsent marks/notes in `To send` AND preserves all earlier `Sent`/`Received` history.
    pub fn end_practice_connection(&mut self, practice_id: &PracticeId) {
        self.connections
            .insert(practice_id.clone(), PracticeLinkStatus::Ended);
        for send in &mut self.sends {
            if &send.snapshot.practice_id == practice_id
                && matches!(
                    send.status,
                    SendDeliveryStatus::Sending | SendDeliveryStatus::Uncertain
                )
            {
                send.status = SendDeliveryStatus::StoppedConnectionEnded;
                send.try_again_available = false;
            }
        }
    }

    #[must_use]
    pub fn practice_status(&self, practice_id: &PracticeId) -> PracticeLinkStatus {
        self.connections
            .get(practice_id)
            .copied()
            .unwrap_or(PracticeLinkStatus::NotInvited)
    }

    /// Marks an active patient record for `To send`.
    pub fn mark_record(
        &mut self,
        record_id: &str,
        records: &PatientRecordStore,
    ) -> Result<(), ForMyDoctorError> {
        let Some(rec) = records.get_record(&self.account_id, record_id) else {
            return Err(ForMyDoctorError::RecordNotAvailable {
                record_id: record_id.to_owned(),
            });
        };
        if rec.removed_upstream {
            return Err(ForMyDoctorError::RecordNotAvailable {
                record_id: record_id.to_owned(),
            });
        }
        self.marked_record_ids.insert(record_id.to_owned());
        Ok(())
    }

    /// Removes a record mark from `To send`.
    pub fn unmark_record(&mut self, record_id: &str) {
        self.marked_record_ids.remove(record_id);
    }

    /// Creates or edits a local unsaved note (`SaveHonesty`).
    pub fn stage_unsaved_note(
        &mut self,
        note_id: impl Into<String>,
        text: impl Into<String>,
        linked_record_ids: Vec<String>,
    ) {
        let note_id = note_id.into();
        let current_rev = self.notes.get(&note_id).map_or(0, |n| n.revision);
        self.notes.insert(
            note_id.clone(),
            DoctorNote {
                note_id,
                text: text.into(),
                linked_record_ids,
                revision: current_rev,
                sync_state: NoteSaveState::UnsavedLocal,
            },
        );
    }

    /// Persists a note using optimistic revision fencing (`SaveHonesty`: remote edits never silently overwrite).
    pub fn save_note(
        &mut self,
        note_id: &str,
        text: impl Into<String>,
        linked_record_ids: Vec<String>,
        expected_revision: u64,
    ) -> Result<DoctorNote, ForMyDoctorError> {
        let current_rev = self.notes.get(note_id).map_or(0, |n| n.revision);
        if current_rev != expected_revision {
            return Err(ForMyDoctorError::NoteSaveConflict {
                note_id: note_id.to_owned(),
                expected: expected_revision,
                current: current_rev,
            });
        }
        let saved = DoctorNote {
            note_id: note_id.to_owned(),
            text: text.into(),
            linked_record_ids,
            revision: current_rev.saturating_add(1),
            sync_state: NoteSaveState::Saved,
        };
        self.notes.insert(note_id.to_owned(), saved.clone());
        Ok(saved)
    }

    /// Deletes a note from `To send`.
    pub fn delete_note(&mut self, note_id: &str) -> bool {
        self.notes.remove(note_id).is_some()
    }

    /// Handles upstream removal of a record (`SourceRemovalClearsMark`):
    /// clears its mark from `To send` with a plain notice while preserving earlier confirmed sends.
    pub fn on_source_removed(&mut self, record_id: &str) {
        if self.marked_record_ids.remove(record_id) {
            self.notices.push(format!(
                "Record '{record_id}' was removed by its source and cleared from To send."
            ));
        }
    }

    /// Explicitly confirms and executes an immutable send to one connected practice.
    ///
    /// Enforces all send-time invariants:
    /// - `DisclosureBeforeSend`
    /// - `SendToOneConnectedPractice`
    /// - 42 CFR Part 2 written consent gate (separate consent for SUD counseling notes) + verbatim § 2.32 notice
    /// - `SendHoldsExactImmutableContent` (freezes selected records AND every record linked to selected notes)
    /// - `SendReachesAtMostOnce` (idempotent send key)
    /// - `ConfirmedSendClearsCarriedOnly` (clears only carried items once `Sent`, clears nothing on `Uncertain`)
    #[allow(clippy::too_many_arguments)]
    pub fn confirm_send(
        &mut self,
        practice_id: &PracticeId,
        selected_record_ids: &[String],
        selected_note_ids: &[String],
        idempotency_key: impl Into<String>,
        disclosure_acknowledged: bool,
        part2_consent: Part2ConsentConfirmation,
        records: &PatientRecordStore,
        receiver: &mut PracticeSendReceiver,
        outcome: SendTransportOutcome,
    ) -> Result<DoctorSendRecord, ForMyDoctorError> {
        if !disclosure_acknowledged {
            return Err(ForMyDoctorError::DisclosureNotAcknowledged);
        }

        let status = self.practice_status(practice_id);
        if status != PracticeLinkStatus::Connected {
            return Err(ForMyDoctorError::PracticeNotConnected {
                practice_id: practice_id.as_str().to_owned(),
                status,
            });
        }

        if selected_record_ids.is_empty() && selected_note_ids.is_empty() {
            return Err(ForMyDoctorError::NothingSelectedToSend);
        }

        let mut frozen_notes = Vec::with_capacity(selected_note_ids.len());
        let mut record_ids_to_freeze: BTreeSet<String> =
            selected_record_ids.iter().cloned().collect();

        for note_id in selected_note_ids {
            let Some(note) = self.notes.get(note_id) else {
                return Err(ForMyDoctorError::NoteNotFound {
                    note_id: note_id.clone(),
                });
            };
            for linked in &note.linked_record_ids {
                record_ids_to_freeze.insert(linked.clone());
            }
            frozen_notes.push(note.clone());
        }

        let mut frozen_records = Vec::with_capacity(record_ids_to_freeze.len());
        let mut requires_part2_notice = false;

        for rec_id in &record_ids_to_freeze {
            let Some(rec) = records.get_record(&self.account_id, rec_id) else {
                return Err(ForMyDoctorError::RecordNotAvailable {
                    record_id: rec_id.clone(),
                });
            };
            if rec.removed_upstream {
                return Err(ForMyDoctorError::RecordNotAvailable {
                    record_id: rec_id.clone(),
                });
            }

            // 42 CFR Part 2 written consent gate.
            match rec.confidentiality {
                ConfidentialityTag::Standard => {}
                ConfidentialityTag::Part2Sud => {
                    if matches!(part2_consent, Part2ConsentConfirmation::None) {
                        return Err(ForMyDoctorError::Part2WrittenConsentRequired {
                            record_id: rec_id.clone(),
                        });
                    }
                    requires_part2_notice = true;
                }
                ConfidentialityTag::Part2SudCounselingNotes => {
                    if !matches!(
                        part2_consent,
                        Part2ConsentConfirmation::SudAndCounselingNotes { .. }
                    ) {
                        return Err(ForMyDoctorError::Part2CounselingNotesConsentRequired {
                            record_id: rec_id.clone(),
                        });
                    }
                    requires_part2_notice = true;
                }
            }

            frozen_records.push(FrozenSharedRecord {
                record_id: rec.record_id.clone(),
                version: rec.version,
                title: rec.title.clone(),
                summary: rec.summary.clone(),
                confidentiality: rec.confidentiality,
                part2_notice: if rec.is_part2() {
                    Some(
                        rec.part2_notice
                            .clone()
                            .unwrap_or_else(|| PART2_REDISCLOSURE_NOTICE.to_owned()),
                    )
                } else {
                    None
                },
            });
        }

        let send_id = format!("send-{}", self.next_send_seq);
        self.next_send_seq = self.next_send_seq.saturating_add(1);
        let snapshot = FrozenSendSnapshot {
            send_id,
            idempotency_key: idempotency_key.into(),
            account_id: self.account_id.clone(),
            practice_id: practice_id.clone(),
            records: frozen_records,
            notes: frozen_notes,
            part2_notice_attached: if requires_part2_notice {
                Some(PART2_REDISCLOSURE_NOTICE.to_owned())
            } else {
                None
            },
            part2_consent,
        };

        // Even under `NetworkUncertain`, the server may have received the payload before the
        // response dropped; idempotency guarantees retry delivers at most once.
        let _ = receiver.accept_idempotent(snapshot.clone());

        let record = match outcome {
            SendTransportOutcome::PersistedSent => {
                // `ConfirmedSendClearsCarriedOnly`: clear ONLY carried marks and notes.
                for carried_rec in selected_record_ids {
                    self.marked_record_ids.remove(carried_rec);
                }
                for carried_note in selected_note_ids {
                    self.notes.remove(carried_note);
                }
                DoctorSendRecord {
                    snapshot,
                    status: SendDeliveryStatus::Sent,
                    try_again_available: false,
                }
            }
            SendTransportOutcome::NetworkUncertain => {
                // Unconfirmed send clears NOTHING from `To send`.
                DoctorSendRecord {
                    snapshot,
                    status: SendDeliveryStatus::Uncertain,
                    try_again_available: true,
                }
            }
        };

        self.sends.push(record.clone());
        Ok(record)
    }

    /// Retries an `Uncertain` send using the exact same `idempotency_key` (`SendReachesAtMostOnce`).
    pub fn retry_uncertain_send(
        &mut self,
        send_id: &str,
        receiver: &mut PracticeSendReceiver,
    ) -> Result<DoctorSendRecord, ForMyDoctorError> {
        let Some(idx) = self
            .sends
            .iter()
            .position(|s| s.snapshot.send_id == send_id)
        else {
            return Err(ForMyDoctorError::SendNotFound {
                send_id: send_id.to_owned(),
            });
        };
        let practice_id = self.sends[idx].snapshot.practice_id.clone();
        if self.practice_status(&practice_id) != PracticeLinkStatus::Connected
            || !self.sends[idx].try_again_available
            || self.sends[idx].status != SendDeliveryStatus::Uncertain
        {
            return Err(ForMyDoctorError::RetryNotAvailable {
                send_id: send_id.to_owned(),
            });
        }

        let snapshot = self.sends[idx].snapshot.clone();
        let _ = receiver.accept_idempotent(snapshot.clone());
        self.sends[idx].status = SendDeliveryStatus::Sent;
        self.sends[idx].try_again_available = false;

        // Clear carried marks and notes now that server persistence is confirmed.
        for rec in &snapshot.records {
            self.marked_record_ids.remove(&rec.record_id);
        }
        for note in &snapshot.notes {
            self.notes.remove(&note.note_id);
        }
        Ok(self.sends[idx].clone())
    }

    /// Marks a `Sent` record as `ReceivedByCareTeam` strictly after practice pickup confirmation (`SentAndReceivedTruth`).
    pub fn sync_care_team_receipt(&mut self, receiver: &PracticeSendReceiver) {
        for send in &mut self.sends {
            if send.status == SendDeliveryStatus::Sent
                && receiver.is_picked_up(&send.snapshot.idempotency_key)
            {
                send.status = SendDeliveryStatus::ReceivedByCareTeam;
            }
        }
    }

    /// Purges `To send` marks/notes for privacy restriction operations (`doctor_page_delete`, `doctor_note_delete`, `doctor_item_delete`, `account_delete`).
    pub fn clear_all_to_send(&mut self) {
        self.marked_record_ids.clear();
        self.notes.clear();
    }

    /// Terminates all connected-care links (called during account deletion & restore replay).
    pub fn terminate_all_connections(&mut self) {
        let practice_ids: Vec<PracticeId> = self.connections.keys().cloned().collect();
        for practice_id in practice_ids {
            self.end_practice_connection(&practice_id);
        }
    }

    #[must_use]
    pub fn marked_record_ids(&self) -> &BTreeSet<String> {
        &self.marked_record_ids
    }

    #[must_use]
    pub fn notes(&self) -> &BTreeMap<String, DoctorNote> {
        &self.notes
    }

    #[must_use]
    pub fn sends(&self) -> &[DoctorSendRecord] {
        &self.sends
    }

    #[must_use]
    pub fn notices(&self) -> &[String] {
        &self.notices
    }

    /// Verifies core structural `Shared.tla` invariants over workspace + receiver state.
    pub fn verify_invariants(
        &self,
        records: &PatientRecordStore,
        receiver: &PracticeSendReceiver,
    ) -> Result<(), SharedInvariantViolation> {
        // ToSendExact: every marked record must exist and not be removed upstream.
        for rec_id in &self.marked_record_ids {
            let active = records
                .get_record(&self.account_id, rec_id)
                .is_some_and(|r| !r.removed_upstream);
            if !active {
                return Err(SharedInvariantViolation::ToSendExact {
                    record_id: rec_id.clone(),
                });
            }
        }

        // SendReachesAtMostOnce & ConnectionEndStopsUncertain.
        for send in &self.sends {
            let count = receiver.delivery_count(&send.snapshot.idempotency_key);
            if count > 1 {
                return Err(SharedInvariantViolation::SendReachesAtMostOnce {
                    idempotency_key: send.snapshot.idempotency_key.clone(),
                });
            }
            if self.practice_status(&send.snapshot.practice_id) == PracticeLinkStatus::Ended
                && send.try_again_available
            {
                return Err(SharedInvariantViolation::ConnectionEndStopsUncertain {
                    send_id: send.snapshot.send_id.clone(),
                });
            }
        }

        Ok(())
    }
}
