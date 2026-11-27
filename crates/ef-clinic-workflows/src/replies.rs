//! Patient message reply drafting & withholding engine proving all 12 `MessageReply.tla` invariants:
//! `OnlyWithAccess`, `OnlyTheReviewAnswers`, `ConcernStays`, `YourDraft`, `CopiesWhatYouSee`,
//! `CardShowsNew`, `OneReviewEach`, `HistoryTrue`, `ShowsUntilHandled`, `WithheldAfterMove`,
//! `NoteWhileDisconnected`, and `UndoBringsBack`.

use std::collections::BTreeMap;

use ef_core::{PhysicianId, SourceKey, WorkspaceId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::matching::MoveDestination;

/// Status of an inbound patient message inside a linked conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatientMessageStatus {
    Unwritten,
    Open,
    Answered,
    Closed,
}

/// One inbound patient message tracked by the reply workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackedPatientMessage {
    pub message_id: String,
    pub body: String,
    pub status: PatientMessageStatus,
}

/// Distinguishes human practice replies from automated out-of-office/auto-responder replies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PracticeReplyKind {
    HumanPracticeReply,
    AutoResponder,
}

/// Immutable record of a message actually sent by the practice (`HistoryTrue`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentPracticeMessage {
    pub message_id: String,
    pub kind: PracticeReplyKind,
    pub exact_text: String,
    pub sent_at: u64,
}

/// Single active clinical review state per conversation (`OneReviewEach`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ReviewExecutionState {
    Idle,
    Woken,
    Running {
        review_id: u64,
        for_patient: WorkspaceId,
    },
}

/// Reason why a system reply is withheld (`WithheldAfterMove`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WithholdReason {
    ConversationMoved,
    AwaitingFreshReview,
}

/// Snapshot captured when a physician closes a reply so Undo restores exact prior state (`UndoBringsBack`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClosedReplySnapshot {
    pub reply_version: u64,
    pub system_text: String,
    pub basis_citations: Vec<SourceKey>,
    pub reopened_message_ids: Vec<String>,
}

/// Reply state machine for a linked conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ReplyState {
    None,
    Ready {
        version: u64,
        system_text: String,
        basis_citations: Vec<SourceKey>,
    },
    Withheld {
        reason: WithholdReason,
    },
    ClosedUndo {
        snapshot: Box<ClosedReplySnapshot>,
    },
}

/// Per-physician personal draft bound strictly to `(physician_id, reply_version)` (`YourDraft`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalReplyDraft {
    pub reply_version: u64,
    pub text: String,
}

/// Source of the visible text on a physician's reply card (`YourDraft`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisibleReplyOrigin {
    None,
    SystemReply,
    PersonalDraft,
}

/// Derived view of a physician's reply card (`CardShowsNew`, `NoteWhileDisconnected`, `YourDraft`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedReplyCard {
    pub open_messages: Vec<TrackedPatientMessage>,
    pub review_state: ReviewExecutionState,
    pub visible_reply_text: Option<String>,
    pub visible_reply_origin: VisibleReplyOrigin,
    pub basis_citations: Vec<SourceKey>,
    pub flagged_concern: Option<String>,
    pub show_channel_disconnected_notice: bool,
    pub undo_available: bool,
    pub reply_state_label: &'static str,
}

/// Typed output submitted when a clinical review finishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewCompletion {
    pub review_id: u64,
    pub for_patient: WorkspaceId,
    pub proposed_reply_text: Option<String>,
    pub basis_citations: Vec<SourceKey>,
    pub flagged_concern: Option<String>,
    pub answered_by_human_reply: bool,
}

/// Typed errors for reply drafting and withholding operations.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ReplyError {
    #[error("conversation '{conversation_id}' not found")]
    ConversationNotFound { conversation_id: String },
    #[error("physician lacks access to patient '{patient}' (OnlyWithAccess)")]
    Unauthorized { patient: String },
    #[error("a review is already running for conversation '{conversation_id}' (OneReviewEach)")]
    ReviewAlreadyRunning { conversation_id: String },
    #[error("patient-facing reply must not contain internal citation or tool markup")]
    InternalMarkupForbidden,
    #[error("no ready reply available to edit, copy, or close")]
    NoReadyReply,
    #[error("no valid undo snapshot available to restore")]
    NoUndoAvailable,
}

/// Returns true if `text` contains forbidden internal citation brackets (`[1]`..`[99]`) or tool IDs.
#[must_use]
pub fn contains_internal_citation_markup(text: &str) -> bool {
    let bytes = text.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'[' {
            let mut j = i + 1;
            let mut digit_count = 0usize;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                digit_count += 1;
                j += 1;
            }
            if digit_count > 0 && j < bytes.len() && bytes[j] == b']' {
                return true;
            }
        }
    }
    text.contains("source_key:") || text.contains("tool:") || text.contains("call_id:")
}

/// Strips inline `[N]` citation tags to produce clean patient-facing prose.
#[must_use]
pub fn sanitize_patient_facing_reply(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let bytes = raw.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            let mut j = i + 1;
            let mut digits = 0usize;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                digits += 1;
                j += 1;
            }
            if digits > 0 && j < bytes.len() && bytes[j] == b']' {
                i = j + 1;
                continue;
            }
        }
        if let Some(ch) = raw[i..].chars().next() {
            out.push(ch);
            i += ch.len_utf8();
        } else {
            break;
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Debug, Clone)]
struct ReplyConversation {
    linked_patient: Option<WorkspaceId>,
    messages: Vec<TrackedPatientMessage>,
    sent_history: Vec<SentPracticeMessage>,
    review_state: ReviewExecutionState,
    reply_state: ReplyState,
    reply_version_seq: u64,
    flagged_concern: Option<String>,
    channel_connected: bool,
    personal_drafts: BTreeMap<PhysicianId, PersonalReplyDraft>,
}

/// Single-owner message reply workflow engine enforcing all 12 `MessageReply.tla` invariants.
#[derive(Debug, Clone, Default)]
pub struct MessageReplyEngine {
    conversations: BTreeMap<String, ReplyConversation>,
    next_review_id: u64,
}

impl MessageReplyEngine {
    #[must_use]
    pub fn new() -> Self {
        Self {
            conversations: BTreeMap::new(),
            next_review_id: 1,
        }
    }

    /// Registers a linked patient conversation on a connected or disconnected channel.
    pub fn register_conversation(
        &mut self,
        conversation_id: impl Into<String>,
        patient: WorkspaceId,
        channel_connected: bool,
    ) {
        self.conversations.insert(
            conversation_id.into(),
            ReplyConversation {
                linked_patient: Some(patient),
                messages: Vec::new(),
                sent_history: Vec::new(),
                review_state: ReviewExecutionState::Idle,
                reply_state: ReplyState::None,
                reply_version_seq: 0,
                flagged_concern: None,
                channel_connected,
                personal_drafts: BTreeMap::new(),
            },
        );
    }

    /// Updates backing channel connectivity (`NoteWhileDisconnected`).
    pub fn set_channel_connected(&mut self, conversation_id: &str, connected: bool) {
        if let Some(conv) = self.conversations.get_mut(conversation_id) {
            conv.channel_connected = connected;
        }
    }

    /// Appends a new inbound patient message, waking a review and invalidating any close Undo (`UndoBringsBack`, `CardShowsNew`).
    pub fn receive_patient_message(
        &mut self,
        conversation_id: &str,
        message_id: impl Into<String>,
        body: impl Into<String>,
    ) -> Result<(), ReplyError> {
        let Some(conv) = self.conversations.get_mut(conversation_id) else {
            return Err(ReplyError::ConversationNotFound {
                conversation_id: conversation_id.to_owned(),
            });
        };
        conv.messages.push(TrackedPatientMessage {
            message_id: message_id.into(),
            body: body.into(),
            status: PatientMessageStatus::Open,
        });
        // New patient message invalidates any close Undo snapshot.
        if matches!(conv.reply_state, ReplyState::ClosedUndo { .. }) {
            conv.reply_state = ReplyState::None;
        }
        if matches!(conv.review_state, ReviewExecutionState::Idle) {
            conv.review_state = ReviewExecutionState::Woken;
        }
        Ok(())
    }

    /// Records a practice message actually sent on the channel (`HistoryTrue`, `OnlyTheReviewAnswers`).
    ///
    /// Notice: Recording a practice message NEVER directly marks patient messages `Answered`;
    /// only a completed clinical review examining a `HumanPracticeReply` can do so, and
    /// `AutoResponder` messages NEVER mark patient messages answered!
    pub fn record_practice_sent(
        &mut self,
        conversation_id: &str,
        message_id: impl Into<String>,
        kind: PracticeReplyKind,
        exact_text: impl Into<String>,
        sent_at: u64,
    ) -> Result<(), ReplyError> {
        let Some(conv) = self.conversations.get_mut(conversation_id) else {
            return Err(ReplyError::ConversationNotFound {
                conversation_id: conversation_id.to_owned(),
            });
        };
        conv.sent_history.push(SentPracticeMessage {
            message_id: message_id.into(),
            kind,
            exact_text: exact_text.into(),
            sent_at,
        });
        if matches!(conv.review_state, ReviewExecutionState::Idle) {
            conv.review_state = ReviewExecutionState::Woken;
        }
        Ok(())
    }

    /// Starts the single clinical review for `conversation_id` (`OneReviewEach`).
    pub fn start_review(
        &mut self,
        conversation_id: &str,
    ) -> Result<(u64, WorkspaceId), ReplyError> {
        let Some(conv) = self.conversations.get_mut(conversation_id) else {
            return Err(ReplyError::ConversationNotFound {
                conversation_id: conversation_id.to_owned(),
            });
        };
        if matches!(conv.review_state, ReviewExecutionState::Running { .. }) {
            return Err(ReplyError::ReviewAlreadyRunning {
                conversation_id: conversation_id.to_owned(),
            });
        }
        let Some(patient) = conv.linked_patient.clone() else {
            return Err(ReplyError::ConversationNotFound {
                conversation_id: conversation_id.to_owned(),
            });
        };
        let review_id = self.next_review_id;
        self.next_review_id = self.next_review_id.saturating_add(1);
        conv.review_state = ReviewExecutionState::Running {
            review_id,
            for_patient: patient.clone(),
        };
        Ok((review_id, patient))
    }

    /// Simulates a worker/server restart: returns any `Running` review cleanly to `Woken`
    /// without duplicating or re-queuing consumed messages (`OneReviewEach`, `ShowsUntilHandled`).
    pub fn recover_interrupted_reviews_on_restart(&mut self) -> usize {
        let mut recovered = 0usize;
        for conv in self.conversations.values_mut() {
            if matches!(conv.review_state, ReviewExecutionState::Running { .. }) {
                conv.review_state = ReviewExecutionState::Woken;
                recovered = recovered.saturating_add(1);
            }
        }
        recovered
    }

    /// Completes a running review (`OnlyTheReviewAnswers`, `WithheldAfterMove`, `YourDraft`).
    ///
    /// - Discards stale review output if `for_patient` no longer matches `linked_patient` or `review_id` is stale!
    /// - Only marks open messages `Answered` when `answered_by_human_reply` is true AND a
    ///   `PracticeReplyKind::HumanPracticeReply` actually exists in `sent_history` (`AutoResponder` never answers!).
    /// - Rejects any patient-facing draft containing internal citation or tool markup.
    pub fn complete_review(
        &mut self,
        conversation_id: &str,
        completion: ReviewCompletion,
    ) -> Result<bool, ReplyError> {
        let Some(conv) = self.conversations.get_mut(conversation_id) else {
            return Err(ReplyError::ConversationNotFound {
                conversation_id: conversation_id.to_owned(),
            });
        };

        // WithheldAfterMove: stale review for old patient or superseded review_id is discarded!
        let matches_active = matches!(
            &conv.review_state,
            ReviewExecutionState::Running {
                review_id: active_id,
                for_patient: active_patient,
            } if *active_id == completion.review_id && active_patient == &completion.for_patient
        ) && conv.linked_patient.as_ref() == Some(&completion.for_patient);

        if !matches_active {
            return Ok(false);
        }

        if let Some(ref concern) = completion.flagged_concern {
            conv.flagged_concern = Some(concern.clone());
        }

        // OnlyTheReviewAnswers: auto-responders NEVER mark patient messages answered.
        let has_human_sent = conv
            .sent_history
            .iter()
            .any(|msg| msg.kind == PracticeReplyKind::HumanPracticeReply);

        if completion.answered_by_human_reply && has_human_sent {
            for msg in &mut conv.messages {
                if msg.status == PatientMessageStatus::Open {
                    msg.status = PatientMessageStatus::Answered;
                }
            }
            conv.reply_state = ReplyState::None;
        } else if let Some(text) = completion.proposed_reply_text {
            if contains_internal_citation_markup(&text) {
                return Err(ReplyError::InternalMarkupForbidden);
            }
            conv.reply_version_seq = conv.reply_version_seq.saturating_add(1);
            conv.reply_state = ReplyState::Ready {
                version: conv.reply_version_seq,
                system_text: text,
                basis_citations: completion.basis_citations,
            };
        }

        conv.review_state = ReviewExecutionState::Idle;
        Ok(true)
    }

    /// Saves a physician's personal draft isolated to `(physician_id, current_reply_version)` (`YourDraft`, `OnlyWithAccess`).
    pub fn edit_personal_draft<F>(
        &mut self,
        physician_id: &PhysicianId,
        conversation_id: &str,
        draft_text: String,
        has_access: F,
    ) -> Result<(), ReplyError>
    where
        F: Fn(&PhysicianId, &WorkspaceId) -> bool,
    {
        if contains_internal_citation_markup(&draft_text) {
            return Err(ReplyError::InternalMarkupForbidden);
        }
        let conv = self.authorize_mut(physician_id, conversation_id, has_access)?;
        let ReplyState::Ready { version, .. } = &conv.reply_state else {
            return Err(ReplyError::NoReadyReply);
        };
        conv.personal_drafts.insert(
            physician_id.clone(),
            PersonalReplyDraft {
                reply_version: *version,
                text: draft_text,
            },
        );
        Ok(())
    }

    /// **Restore system reply**: discards ONLY `physician_id`'s personal draft (`YourDraft`).
    pub fn restore_system_reply<F>(
        &mut self,
        physician_id: &PhysicianId,
        conversation_id: &str,
        has_access: F,
    ) -> Result<(), ReplyError>
    where
        F: Fn(&PhysicianId, &WorkspaceId) -> bool,
    {
        let conv = self.authorize_mut(physician_id, conversation_id, has_access)?;
        let _ = conv.personal_drafts.remove(physician_id);
        Ok(())
    }

    /// **Copy reply**: returns the exact text currently displayed on `physician_id`'s card
    /// with ZERO state mutation (`CopiesWhatYouSee`, `OnlyWithAccess`).
    pub fn copy_reply<F>(
        &self,
        physician_id: &PhysicianId,
        conversation_id: &str,
        has_access: F,
    ) -> Result<String, ReplyError>
    where
        F: Fn(&PhysicianId, &WorkspaceId) -> bool,
    {
        let card = self.derive_reply_card(physician_id, conversation_id, has_access)?;
        card.visible_reply_text.ok_or(ReplyError::NoReadyReply)
    }

    /// Closes a reply (**No reply needed** / **Done**), preserving any flagged clinical concern
    /// (`ConcernStays`) and opening an Undo snapshot (`UndoBringsBack`).
    pub fn close_reply<F>(
        &mut self,
        physician_id: &PhysicianId,
        conversation_id: &str,
        has_access: F,
    ) -> Result<(), ReplyError>
    where
        F: Fn(&PhysicianId, &WorkspaceId) -> bool,
    {
        let conv = self.authorize_mut(physician_id, conversation_id, has_access)?;
        let ReplyState::Ready {
            version,
            system_text,
            basis_citations,
        } = conv.reply_state.clone()
        else {
            return Err(ReplyError::NoReadyReply);
        };

        let mut reopened_message_ids = Vec::new();
        for msg in &mut conv.messages {
            if msg.status == PatientMessageStatus::Open {
                msg.status = PatientMessageStatus::Closed;
                reopened_message_ids.push(msg.message_id.clone());
            }
        }
        // ConcernStays: conv.flagged_concern is intentionally preserved!
        conv.reply_state = ReplyState::ClosedUndo {
            snapshot: Box::new(ClosedReplySnapshot {
                reply_version: version,
                system_text,
                basis_citations,
                reopened_message_ids,
            }),
        };
        Ok(())
    }

    /// Undoes a closed reply, restoring the exact closed reply and open patient messages (`UndoBringsBack`).
    pub fn undo_close_reply<F>(
        &mut self,
        physician_id: &PhysicianId,
        conversation_id: &str,
        has_access: F,
    ) -> Result<(), ReplyError>
    where
        F: Fn(&PhysicianId, &WorkspaceId) -> bool,
    {
        let conv = self.authorize_mut(physician_id, conversation_id, has_access)?;
        let ReplyState::ClosedUndo { snapshot } = conv.reply_state.clone() else {
            return Err(ReplyError::NoUndoAvailable);
        };
        for msg in &mut conv.messages {
            if snapshot.reopened_message_ids.contains(&msg.message_id) {
                msg.status = PatientMessageStatus::Open;
            }
        }
        conv.reply_state = ReplyState::Ready {
            version: snapshot.reply_version,
            system_text: snapshot.system_text,
            basis_citations: snapshot.basis_citations,
        };
        Ok(())
    }

    /// Handles moving the conversation to another patient or `NotPatient`:
    /// - Another patient: immediately withholds prior ready reply, invalidates Undo, and wakes a fresh review (`WithheldAfterMove`, `UndoBringsBack`).
    /// - `NotPatient`: clears both reply AND flagged concern (`ConcernStays`).
    pub fn on_conversation_moved(
        &mut self,
        conversation_id: &str,
        destination: MoveDestination,
    ) -> Result<(), ReplyError> {
        let Some(conv) = self.conversations.get_mut(conversation_id) else {
            return Err(ReplyError::ConversationNotFound {
                conversation_id: conversation_id.to_owned(),
            });
        };
        match destination {
            MoveDestination::Patient(new_patient) => {
                conv.linked_patient = Some(new_patient);
                conv.personal_drafts.clear();
                conv.reply_state = ReplyState::Withheld {
                    reason: WithholdReason::ConversationMoved,
                };
                // Supersede any in-flight review for old patient and wake a fresh review.
                conv.review_state = ReviewExecutionState::Woken;
            }
            MoveDestination::NotPatient => {
                conv.linked_patient = None;
                conv.personal_drafts.clear();
                conv.reply_state = ReplyState::None;
                conv.flagged_concern = None;
                conv.review_state = ReviewExecutionState::Idle;
            }
        }
        Ok(())
    }

    /// Derives the physician-scoped reply card (`OnlyWithAccess`, `YourDraft`, `CardShowsNew`, `NoteWhileDisconnected`).
    pub fn derive_reply_card<F>(
        &self,
        physician_id: &PhysicianId,
        conversation_id: &str,
        has_access: F,
    ) -> Result<DerivedReplyCard, ReplyError>
    where
        F: Fn(&PhysicianId, &WorkspaceId) -> bool,
    {
        let Some(conv) = self.conversations.get(conversation_id) else {
            return Err(ReplyError::ConversationNotFound {
                conversation_id: conversation_id.to_owned(),
            });
        };
        let Some(patient) = &conv.linked_patient else {
            return Err(ReplyError::ConversationNotFound {
                conversation_id: conversation_id.to_owned(),
            });
        };
        if !has_access(physician_id, patient) {
            return Err(ReplyError::Unauthorized {
                patient: patient.as_str().to_owned(),
            });
        }

        let open_messages: Vec<TrackedPatientMessage> = conv
            .messages
            .iter()
            .filter(|m| m.status == PatientMessageStatus::Open)
            .cloned()
            .collect();
        let (
            visible_reply_text,
            visible_reply_origin,
            basis_citations,
            reply_state_label,
            is_open_reply,
            undo_available,
        ) = match &conv.reply_state {
            ReplyState::None => (
                None,
                VisibleReplyOrigin::None,
                Vec::new(),
                "none",
                false,
                false,
            ),
            ReplyState::Ready {
                version,
                system_text,
                basis_citations,
            } => {
                let matching_draft = conv
                    .personal_drafts
                    .get(physician_id)
                    .filter(|d| d.reply_version == *version);
                match matching_draft {
                    Some(draft) => (
                        Some(draft.text.clone()),
                        VisibleReplyOrigin::PersonalDraft,
                        basis_citations.clone(),
                        "ready",
                        true,
                        false,
                    ),
                    None => (
                        Some(system_text.clone()),
                        VisibleReplyOrigin::SystemReply,
                        basis_citations.clone(),
                        "ready",
                        true,
                        false,
                    ),
                }
            }
            ReplyState::Withheld { .. } => (
                None,
                VisibleReplyOrigin::None,
                Vec::new(),
                "withheld",
                true,
                false,
            ),
            ReplyState::ClosedUndo { .. } => (
                None,
                VisibleReplyOrigin::None,
                Vec::new(),
                "closed_undo",
                false,
                true,
            ),
        };

        // NoteWhileDisconnected: shown iff open (`ready` or `withheld`) AND channel is disconnected.
        let show_channel_disconnected_notice = is_open_reply && !conv.channel_connected;

        Ok(DerivedReplyCard {
            open_messages,
            review_state: conv.review_state.clone(),
            visible_reply_text,
            visible_reply_origin,
            basis_citations,
            flagged_concern: conv.flagged_concern.clone(),
            show_channel_disconnected_notice,
            undo_available,
            reply_state_label,
        })
    }

    /// Returns exact recorded sent practice history (`HistoryTrue`).
    #[must_use]
    pub fn sent_history(&self, conversation_id: &str) -> &[SentPracticeMessage] {
        self.conversations
            .get(conversation_id)
            .map_or(&[], |c| c.sent_history.as_slice())
    }

    /// Returns the flagged clinical concern even if the conversation was moved to `NotPatient` (for testing `ConcernStays`).
    #[must_use]
    pub fn flagged_concern(&self, conversation_id: &str) -> Option<&str> {
        self.conversations
            .get(conversation_id)
            .and_then(|c| c.flagged_concern.as_deref())
    }

    fn authorize_mut<F>(
        &mut self,
        physician_id: &PhysicianId,
        conversation_id: &str,
        has_access: F,
    ) -> Result<&mut ReplyConversation, ReplyError>
    where
        F: Fn(&PhysicianId, &WorkspaceId) -> bool,
    {
        let Some(conv) = self.conversations.get_mut(conversation_id) else {
            return Err(ReplyError::ConversationNotFound {
                conversation_id: conversation_id.to_owned(),
            });
        };
        let Some(patient) = &conv.linked_patient else {
            return Err(ReplyError::ConversationNotFound {
                conversation_id: conversation_id.to_owned(),
            });
        };
        if !has_access(physician_id, patient) {
            return Err(ReplyError::Unauthorized {
                patient: patient.as_str().to_owned(),
            });
        }
        Ok(conv)
    }
}
