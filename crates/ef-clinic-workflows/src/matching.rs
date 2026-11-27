//! Incoming patient message matching engine proving all 14 `MessageMatching.tla` invariants:
//! `OnlyWithAccess`, `SuggestionsWithAccess`, `JoinsItsPatient`, `LinksOnItsOwn`, `RightPatient`,
//! `NeverInvisible`, `UndoPutsBack`, `ActsAfterUndo`, `OnlyBetweenTheirPatients`, `TodaySaysSo`,
//! `ClearedOnDisconnect`, `ExactlyOnce`, `CantReadShows`, and `BacklogOnlyWhenAccepted`.

use std::collections::{BTreeMap, BTreeSet};

use ef_core::{PhysicianId, WorkspaceId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::today::{ChannelHealthStatus, TodayChannelBanner, derive_channel_banner};

/// 15-minute undecided fallback window (`900 s`) ensuring `NeverInvisible`.
pub const UNDECIDED_FALLBACK_SECS: u64 = 900;

/// Default Undo window duration (`15 s`) before deferred downstream record updates execute.
pub const DEFAULT_UNDO_WINDOW_SECS: u64 = 15;

/// Canonical conversation matching states from Chapter 1 §1.2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ConversationMatchState {
    None,
    Undecided {
        first_kept_at: u64,
    },
    ToMatch {
        suggested_patient: Option<WorkspaceId>,
    },
    Filtered,
    Linked {
        patient: WorkspaceId,
    },
    NotPatient,
}

/// Destination for a manual link or move operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MoveDestination {
    Patient(WorkspaceId),
    NotPatient,
}

/// Pre-move origin preserved across chained moves (`UndoPutsBack`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MatchOrigin {
    Linked(WorkspaceId),
    NotPatient,
    ToMatch {
        suggested_patient: Option<WorkspaceId>,
    },
    Filtered,
}

/// Active Undo window deferring downstream clinical record updates (`ActsAfterUndo`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoWindow {
    pub origin: MatchOrigin,
    pub target: MoveDestination,
    pub opened_at: u64,
    pub expires_at: u64,
}

/// Automated matching signal produced by identity evaluation (`LinksOnItsOwn`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutomatedMatchSignal {
    /// Exactly one confirmed patient with zero contradicting identity signals.
    UnambiguousSingle(WorkspaceId),
    /// Multiple candidates or conflicting identifiers -> routes to `ToMatch`.
    AmbiguousOrContradicted { suggested: Option<WorkspaceId> },
    /// Likely non-patient traffic -> routes to `Filtered` (still linkable).
    LikelyNonPatient,
}

/// A kept message belonging to a channel conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptChannelMessage {
    pub cursor_seq: u64,
    pub message_id: String,
    pub body: String,
    pub received_at: u64,
}

/// Incoming channel message payload before decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawChannelFrame {
    Readable {
        cursor_seq: u64,
        conversation_id: String,
        message_id: String,
        body: String,
    },
    Unreadable {
        cursor_seq: u64,
        reason: String,
    },
}

/// Surfaced unreadable message waiting for retry recovery, explicit Done, or disconnect (`CantReadShows`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnreadableQueueEntry {
    pub cursor_seq: u64,
    pub channel_id: String,
    pub reason: String,
    pub marked_done: bool,
}

/// Pre-connection backlog consent state (`BacklogOnlyWhenAccepted`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum BacklogConsentState {
    None,
    Offered { pending_count: usize },
    Accepted,
    Declined,
}

/// Deferred downstream clinical record update committed only after Undo expires (`ActsAfterUndo`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommittedRecordUpdate {
    pub conversation_id: String,
    pub destination: MoveDestination,
    pub message_count: usize,
    pub committed_at: u64,
}

/// One channel conversation managed by [`MessageMatchingEngine`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedConversation {
    pub conversation_id: String,
    pub channel_id: String,
    pub state: ConversationMatchState,
    pub messages: Vec<KeptChannelMessage>,
    pub undo: Option<UndoWindow>,
    /// Physicians currently holding an open view on this conversation.
    pub open_viewers: BTreeSet<PhysicianId>,
}

/// Scoped view of a `ToMatch` item with access-filtered suggestion (`SuggestionsWithAccess`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedToMatchItem {
    pub conversation_id: String,
    pub suggested_patient: Option<WorkspaceId>,
    pub message_count: usize,
}

/// Typed errors for message matching operations.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MatchingError {
    #[error("channel cursor mismatch: expected seq {expected}, got {actual} (ExactlyOnce)")]
    CursorSequenceMismatch { expected: u64, actual: u64 },
    #[error("conversation '{conversation_id}' not found")]
    ConversationNotFound { conversation_id: String },
    #[error("physician lacks authorization for patient '{patient}' (OnlyBetweenTheirPatients)")]
    UnauthorizedPatient { patient: String },
    #[error("no active undo window on conversation '{conversation_id}'")]
    NoActiveUndo { conversation_id: String },
    #[error("channel '{channel_id}' has not accepted backlog consent (BacklogOnlyWhenAccepted)")]
    BacklogConsentRequired { channel_id: String },
    #[error("channel '{channel_id}' is disconnected")]
    ChannelDisconnected { channel_id: String },
}

#[derive(Debug, Clone)]
struct ChannelState {
    health: ChannelHealthStatus,
    next_expected_cursor: u64,
    backlog_consent: BacklogConsentState,
    held_backlog: Vec<RawChannelFrame>,
}

/// Single-owner incoming message matching engine enforcing all 14 `MessageMatching.tla` invariants.
#[derive(Debug, Clone, Default)]
pub struct MessageMatchingEngine {
    channels: BTreeMap<String, ChannelState>,
    conversations: BTreeMap<String, ManagedConversation>,
    unreadable_queue: Vec<UnreadableQueueEntry>,
    committed_record_updates: Vec<CommittedRecordUpdate>,
}

impl MessageMatchingEngine {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers or updates a connected communication channel.
    pub fn register_channel(&mut self, channel_id: impl Into<String>, health: ChannelHealthStatus) {
        self.channels
            .entry(channel_id.into())
            .and_modify(|ch| ch.health = health.clone())
            .or_insert(ChannelState {
                health,
                next_expected_cursor: 1,
                backlog_consent: BacklogConsentState::None,
                held_backlog: Vec::new(),
            });
    }

    /// Derives the Today health banner for a channel (`TodaySaysSo`).
    #[must_use]
    pub fn channel_today_banner(&self, channel_id: &str) -> TodayChannelBanner {
        match self.channels.get(channel_id) {
            Some(ch) => derive_channel_banner(channel_id, &ch.health),
            None => TodayChannelBanner::None,
        }
    }

    /// Ingests strictly the next channel message (`ExactlyOnce`), advancing cursor by 1 only
    /// after keeping the message or recording it in the unreadable queue (`CantReadShows`).
    pub fn ingest_channel_frame(
        &mut self,
        channel_id: &str,
        frame: RawChannelFrame,
        now_secs: u64,
    ) -> Result<(), MatchingError> {
        let Some(ch) = self.channels.get_mut(channel_id) else {
            return Err(MatchingError::ChannelDisconnected {
                channel_id: channel_id.to_owned(),
            });
        };
        if ch.health == ChannelHealthStatus::Disconnected {
            return Err(MatchingError::ChannelDisconnected {
                channel_id: channel_id.to_owned(),
            });
        }

        let seq = match &frame {
            RawChannelFrame::Readable { cursor_seq, .. }
            | RawChannelFrame::Unreadable { cursor_seq, .. } => *cursor_seq,
        };
        if seq != ch.next_expected_cursor {
            return Err(MatchingError::CursorSequenceMismatch {
                expected: ch.next_expected_cursor,
                actual: seq,
            });
        }

        match frame {
            RawChannelFrame::Readable {
                cursor_seq,
                conversation_id,
                message_id,
                body,
            } => {
                let entry = self
                    .conversations
                    .entry(conversation_id.clone())
                    .or_insert_with(|| ManagedConversation {
                        conversation_id,
                        channel_id: channel_id.to_owned(),
                        state: ConversationMatchState::None,
                        messages: Vec::new(),
                        undo: None,
                        open_viewers: BTreeSet::new(),
                    });
                entry.messages.push(KeptChannelMessage {
                    cursor_seq,
                    message_id,
                    body,
                    received_at: now_secs,
                });
                // JoinsItsPatient: linked conversations stay bound; only `None` enters `Undecided`.
                if entry.state == ConversationMatchState::None {
                    entry.state = ConversationMatchState::Undecided {
                        first_kept_at: now_secs,
                    };
                }
            }
            RawChannelFrame::Unreadable { cursor_seq, reason } => {
                self.unreadable_queue.push(UnreadableQueueEntry {
                    cursor_seq,
                    channel_id: channel_id.to_owned(),
                    reason,
                    marked_done: false,
                });
            }
        }

        ch.next_expected_cursor = ch.next_expected_cursor.saturating_add(1);
        Ok(())
    }

    /// Applies an automated matching signal to an `Undecided` conversation (`LinksOnItsOwn`).
    pub fn apply_automated_match(
        &mut self,
        conversation_id: &str,
        signal: AutomatedMatchSignal,
        now_secs: u64,
    ) -> Result<(), MatchingError> {
        let Some(conv) = self.conversations.get_mut(conversation_id) else {
            return Err(MatchingError::ConversationNotFound {
                conversation_id: conversation_id.to_owned(),
            });
        };
        if !matches!(conv.state, ConversationMatchState::Undecided { .. }) {
            return Ok(());
        }
        match signal {
            AutomatedMatchSignal::UnambiguousSingle(patient) => {
                conv.state = ConversationMatchState::Linked {
                    patient: patient.clone(),
                };
                self.committed_record_updates.push(CommittedRecordUpdate {
                    conversation_id: conv.conversation_id.clone(),
                    destination: MoveDestination::Patient(patient),
                    message_count: conv.messages.len(),
                    committed_at: now_secs,
                });
            }
            AutomatedMatchSignal::AmbiguousOrContradicted { suggested } => {
                conv.state = ConversationMatchState::ToMatch {
                    suggested_patient: suggested,
                };
            }
            AutomatedMatchSignal::LikelyNonPatient => {
                conv.state = ConversationMatchState::Filtered;
            }
        }
        Ok(())
    }

    /// Runs the 15-minute undecided fallback (`NeverInvisible`), moving stalled `Undecided`
    /// conversations to `ToMatch` so no kept message ever becomes invisible.
    pub fn run_undecided_fallback(&mut self, now_secs: u64) -> usize {
        let mut transitioned = 0usize;
        for conv in self.conversations.values_mut() {
            if let ConversationMatchState::Undecided { first_kept_at } = conv.state
                && now_secs >= first_kept_at.saturating_add(UNDECIDED_FALLBACK_SECS)
            {
                conv.state = ConversationMatchState::ToMatch {
                    suggested_patient: None,
                };
                transitioned = transitioned.saturating_add(1);
            }
        }
        transitioned
    }

    /// Opens a physician's view on a linked conversation only if authorized (`OnlyWithAccess`).
    pub fn open_conversation_view<F>(
        &mut self,
        physician_id: &PhysicianId,
        conversation_id: &str,
        has_access: F,
    ) -> Result<(), MatchingError>
    where
        F: Fn(&PhysicianId, &WorkspaceId) -> bool,
    {
        let Some(conv) = self.conversations.get_mut(conversation_id) else {
            return Err(MatchingError::ConversationNotFound {
                conversation_id: conversation_id.to_owned(),
            });
        };
        if let ConversationMatchState::Linked { patient } = &conv.state
            && !has_access(physician_id, patient)
        {
            return Err(MatchingError::UnauthorizedPatient {
                patient: patient.as_str().to_owned(),
            });
        }
        conv.open_viewers.insert(physician_id.clone());
        Ok(())
    }

    /// Returns linked conversations for `patient` visible only to an authorized physician (`OnlyWithAccess`, `RightPatient`).
    #[must_use]
    pub fn linked_conversations_for_patient<F>(
        &self,
        physician_id: &PhysicianId,
        patient: &WorkspaceId,
        has_access: F,
    ) -> Vec<&ManagedConversation>
    where
        F: Fn(&PhysicianId, &WorkspaceId) -> bool,
    {
        if !has_access(physician_id, patient) {
            return Vec::new();
        }
        self.conversations
            .values()
            .filter(|conv| {
                matches!(&conv.state, ConversationMatchState::Linked { patient: p } if p == patient)
            })
            .collect()
    }

    /// Returns **Messages to match** items with `suggested_patient` stripped whenever `physician_id`
    /// lacks access to that patient (`SuggestionsWithAccess`).
    #[must_use]
    pub fn visible_to_match_for<F>(
        &self,
        physician_id: &PhysicianId,
        has_access: F,
    ) -> Vec<ScopedToMatchItem>
    where
        F: Fn(&PhysicianId, &WorkspaceId) -> bool,
    {
        self.conversations
            .values()
            .filter_map(|conv| match &conv.state {
                ConversationMatchState::ToMatch { suggested_patient } => {
                    let visible_suggestion = suggested_patient
                        .as_ref()
                        .filter(|p| has_access(physician_id, p))
                        .cloned();
                    Some(ScopedToMatchItem {
                        conversation_id: conv.conversation_id.clone(),
                        suggested_patient: visible_suggestion,
                        message_count: conv.messages.len(),
                    })
                }
                _ => None,
            })
            .collect()
    }

    /// Links or moves a conversation (`OnlyBetweenTheirPatients`, `UndoPutsBack`, `ActsAfterUndo`).
    ///
    /// - Enforces physician authorization on both current linked patient and target patient.
    /// - Preserves original pre-move `origin` even across chained moves while Undo is active.
    /// - Immediately closes `open_viewers` for peer physicians lacking access to the destination.
    /// - Defers downstream clinical record update until Undo expires.
    pub fn move_conversation<F>(
        &mut self,
        physician_id: &PhysicianId,
        conversation_id: &str,
        destination: MoveDestination,
        has_access: F,
        now_secs: u64,
    ) -> Result<(), MatchingError>
    where
        F: Fn(&PhysicianId, &WorkspaceId) -> bool,
    {
        let Some(conv) = self.conversations.get_mut(conversation_id) else {
            return Err(MatchingError::ConversationNotFound {
                conversation_id: conversation_id.to_owned(),
            });
        };

        if let ConversationMatchState::Linked { patient } = &conv.state
            && !has_access(physician_id, patient)
        {
            return Err(MatchingError::UnauthorizedPatient {
                patient: patient.as_str().to_owned(),
            });
        }
        if let MoveDestination::Patient(dest_patient) = &destination
            && !has_access(physician_id, dest_patient)
        {
            return Err(MatchingError::UnauthorizedPatient {
                patient: dest_patient.as_str().to_owned(),
            });
        }

        // Preserve original pre-move origin across chained moves!
        let origin = match &conv.undo {
            Some(existing) => existing.origin.clone(),
            None => match &conv.state {
                ConversationMatchState::Linked { patient } => MatchOrigin::Linked(patient.clone()),
                ConversationMatchState::NotPatient => MatchOrigin::NotPatient,
                ConversationMatchState::ToMatch { suggested_patient } => MatchOrigin::ToMatch {
                    suggested_patient: suggested_patient.clone(),
                },
                ConversationMatchState::Filtered => MatchOrigin::Filtered,
                ConversationMatchState::None | ConversationMatchState::Undecided { .. } => {
                    MatchOrigin::ToMatch {
                        suggested_patient: None,
                    }
                }
            },
        };

        conv.state = match &destination {
            MoveDestination::Patient(p) => ConversationMatchState::Linked { patient: p.clone() },
            MoveDestination::NotPatient => ConversationMatchState::NotPatient,
        };

        // Immediately close active views for peer physicians lacking access to destination.
        match &destination {
            MoveDestination::Patient(dest_patient) => {
                conv.open_viewers
                    .retain(|viewer| has_access(viewer, dest_patient));
            }
            MoveDestination::NotPatient => {
                conv.open_viewers.clear();
            }
        }

        conv.undo = Some(UndoWindow {
            origin,
            target: destination,
            opened_at: now_secs,
            expires_at: now_secs.saturating_add(DEFAULT_UNDO_WINDOW_SECS),
        });
        Ok(())
    }

    /// Pressing **Undo** restores the conversation AND every kept message (including newly arrived
    /// messages) back to its original pre-move origin and cancels deferred updates (`UndoPutsBack`, `ActsAfterUndo`).
    pub fn undo_move(&mut self, conversation_id: &str) -> Result<(), MatchingError> {
        let Some(conv) = self.conversations.get_mut(conversation_id) else {
            return Err(MatchingError::ConversationNotFound {
                conversation_id: conversation_id.to_owned(),
            });
        };
        let Some(undo) = conv.undo.take() else {
            return Err(MatchingError::NoActiveUndo {
                conversation_id: conversation_id.to_owned(),
            });
        };
        conv.state = match undo.origin {
            MatchOrigin::Linked(patient) => ConversationMatchState::Linked { patient },
            MatchOrigin::NotPatient => ConversationMatchState::NotPatient,
            MatchOrigin::ToMatch { suggested_patient } => {
                ConversationMatchState::ToMatch { suggested_patient }
            }
            MatchOrigin::Filtered => ConversationMatchState::Filtered,
        };
        Ok(())
    }

    /// Expires standing Undo windows at `now_secs` and commits deferred clinical record updates
    /// ONLY when the standing destination differs from the original pre-move origin (`ActsAfterUndo`).
    pub fn expire_undo_windows(&mut self, now_secs: u64) -> usize {
        let mut committed = 0usize;
        for conv in self.conversations.values_mut() {
            let expired = conv.undo.as_ref().is_some_and(|u| now_secs >= u.expires_at);
            if !expired {
                continue;
            }
            if let Some(undo) = conv.undo.take() {
                let is_noop = matches!(
                    (&undo.origin, &undo.target),
                    (MatchOrigin::Linked(orig), MoveDestination::Patient(dest)) if orig == dest
                ) || matches!(
                    (&undo.origin, &undo.target),
                    (MatchOrigin::NotPatient, MoveDestination::NotPatient)
                );
                if !is_noop {
                    self.committed_record_updates.push(CommittedRecordUpdate {
                        conversation_id: conv.conversation_id.clone(),
                        destination: undo.target,
                        message_count: conv.messages.len(),
                        committed_at: now_secs,
                    });
                    committed = committed.saturating_add(1);
                }
            }
        }
        committed
    }

    /// Marks an unreadable queue entry explicitly **Done** (`CantReadShows`).
    pub fn mark_unreadable_done(&mut self, channel_id: &str, cursor_seq: u64) -> bool {
        for entry in &mut self.unreadable_queue {
            if entry.channel_id == channel_id && entry.cursor_seq == cursor_seq {
                entry.marked_done = true;
                return true;
            }
        }
        false
    }

    /// Disconnects a channel: clears ALL unlinked conversations and unreadable entries while
    /// preserving all already-`Linked` patient conversations (`ClearedOnDisconnect`).
    pub fn disconnect_channel(&mut self, channel_id: &str) {
        if let Some(ch) = self.channels.get_mut(channel_id) {
            ch.health = ChannelHealthStatus::Disconnected;
            ch.held_backlog.clear();
            ch.backlog_consent = BacklogConsentState::None;
        }
        self.conversations.retain(|_, conv| {
            conv.channel_id != channel_id
                || matches!(conv.state, ConversationMatchState::Linked { .. })
        });
        self.unreadable_queue
            .retain(|entry| entry.channel_id != channel_id);
    }

    /// Reconnects a channel with pre-connection backlog: offers backlog (`Offered`) and NEVER
    /// ingests until explicit physician consent is accepted (`BacklogOnlyWhenAccepted`).
    pub fn reconnect_channel_offering_backlog(
        &mut self,
        channel_id: &str,
        backlog: Vec<RawChannelFrame>,
    ) {
        let count = backlog.len();
        let entry = self
            .channels
            .entry(channel_id.to_owned())
            .or_insert(ChannelState {
                health: ChannelHealthStatus::Connected,
                next_expected_cursor: 1,
                backlog_consent: BacklogConsentState::None,
                held_backlog: Vec::new(),
            });
        entry.health = ChannelHealthStatus::Connected;
        entry.held_backlog = backlog;
        entry.backlog_consent = if count > 0 {
            BacklogConsentState::Offered {
                pending_count: count,
            }
        } else {
            BacklogConsentState::None
        };
    }

    /// Records explicit physician consent (`Accepted`) and ingests the offered backlog (`BacklogOnlyWhenAccepted`).
    pub fn accept_backlog_consent(
        &mut self,
        channel_id: &str,
        now_secs: u64,
    ) -> Result<usize, MatchingError> {
        let frames = {
            let Some(ch) = self.channels.get_mut(channel_id) else {
                return Err(MatchingError::ChannelDisconnected {
                    channel_id: channel_id.to_owned(),
                });
            };
            ch.backlog_consent = BacklogConsentState::Accepted;
            std::mem::take(&mut ch.held_backlog)
        };
        let count = frames.len();
        for frame in frames {
            self.ingest_channel_frame(channel_id, frame, now_secs)?;
        }
        Ok(count)
    }

    /// Returns active unreadable queue items not yet marked Done (`CantReadShows`).
    #[must_use]
    pub fn visible_unreadable_entries(&self) -> Vec<&UnreadableQueueEntry> {
        self.unreadable_queue
            .iter()
            .filter(|e| !e.marked_done)
            .collect()
    }

    /// Returns the backlog consent status for `channel_id`.
    #[must_use]
    pub fn backlog_consent_state(&self, channel_id: &str) -> Option<&BacklogConsentState> {
        self.channels.get(channel_id).map(|c| &c.backlog_consent)
    }

    /// Inspects a managed conversation.
    #[must_use]
    pub fn conversation(&self, id: &str) -> Option<&ManagedConversation> {
        self.conversations.get(id)
    }

    /// All downstream clinical record updates committed after Undo expiry or auto-linking.
    #[must_use]
    pub fn committed_record_updates(&self) -> &[CommittedRecordUpdate] {
        &self.committed_record_updates
    }
}
