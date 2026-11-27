//! HTTP route handlers for M3 Clinic Workflows (`ef-clinic-workflows`) and
//! Connected Care Clinic invitations (`ef-connected-care`).

use ef_clinic_workflows::matching::{
    AutomatedMatchSignal, ConversationMatchState, MatchingError, MoveDestination, RawChannelFrame,
};
use ef_clinic_workflows::memory::{
    MemoryAuthority, MemoryEntry, PatientMemoryStore, RoundsObservation, RoundsRunOutcome,
};
use ef_clinic_workflows::replies::{
    DerivedReplyCard, ReplyError, ReviewCompletion, ReviewExecutionState, TrackedPatientMessage,
    VisibleReplyOrigin, contains_internal_citation_markup,
};
use ef_clinic_workflows::today::{
    ChannelHealthStatus, TodayItemKind, TodayQueueItem, TodayUrgency,
};
use ef_connected_care::{
    BridgeError, ConnectionStanding, CreateInvitationRequest, DateOfBirth, InvitationId, PairKey,
};
use ef_core::{PatientId, PhysicianId, PracticeId, SourceKey, WorkspaceId};
use serde::{Deserialize, Serialize};

use crate::http::{HttpMethod, HttpRequest, HttpResponse};
use crate::persistence::{WalAutoMatchSignal, WalEntry, WalMoveDestination};
use crate::service::{ClinicService, wal_failure_response, workspace_error_response};

#[derive(Debug, Deserialize)]
struct TodayStandingBody {
    item_id: String,
    action: TodayStandingAction,
    #[serde(default)]
    version_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TodayStandingAction {
    Seen,
    Done,
}

#[derive(Debug, Deserialize)]
struct IngestMessageBody {
    channel_id: String,
    cursor_seq: u64,
    #[serde(default)]
    conversation_id: Option<String>,
    #[serde(default)]
    message_id: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    unreadable_reason: Option<String>,
    #[serde(default)]
    auto_signal: Option<WalAutoMatchSignal>,
}

#[derive(Debug, Deserialize)]
struct DecideMessageBody {
    conversation_id: String,
    destination: WalMoveDestination,
}

#[derive(Debug, Serialize)]
struct UndoWindowDto {
    target: WalMoveDestination,
    opened_at: u64,
    expires_at: u64,
}

#[derive(Debug, Deserialize)]
struct UndoMessageBody {
    conversation_id: String,
}

#[derive(Debug, Serialize)]
struct ToMatchItemDto {
    conversation_id: String,
    suggested_patient: Option<WorkspaceId>,
    message_count: usize,
}

#[derive(Debug, Deserialize)]
struct CompleteReplyReviewBody {
    #[serde(default)]
    proposed_reply_text: Option<String>,
    #[serde(default)]
    basis_citations: Vec<SourceKey>,
    #[serde(default)]
    flagged_concern: Option<String>,
    #[serde(default)]
    answered_by_human_reply: bool,
}

#[derive(Debug, Deserialize)]
struct EditReplyDraftBody {
    text: String,
}

#[derive(Debug, Serialize)]
struct ReplyCardResponse {
    conversation_id: String,
    open_messages: Vec<TrackedPatientMessage>,
    review_state: ReviewExecutionState,
    visible_reply_text: Option<String>,
    visible_reply_origin: &'static str,
    basis_citations: Vec<SourceKey>,
    flagged_concern: Option<String>,
    show_channel_disconnected_notice: bool,
    undo_available: bool,
    reply_state_label: &'static str,
}

impl ReplyCardResponse {
    fn from_card(conversation_id: String, card: DerivedReplyCard) -> Self {
        let origin = match card.visible_reply_origin {
            VisibleReplyOrigin::None => "none",
            VisibleReplyOrigin::SystemReply => "system_reply",
            VisibleReplyOrigin::PersonalDraft => "personal_draft",
        };
        Self {
            conversation_id,
            open_messages: card.open_messages,
            review_state: card.review_state,
            visible_reply_text: card.visible_reply_text,
            visible_reply_origin: origin,
            basis_citations: card.basis_citations,
            flagged_concern: card.flagged_concern,
            show_channel_disconnected_notice: card.show_channel_disconnected_notice,
            undo_available: card.undo_available,
            reply_state_label: card.reply_state_label,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum MemoryMutationRequest {
    Record {
        topic: String,
        statement: String,
        observed_at: String,
        authority: MemoryAuthority,
    },
    RunRounds {
        observations: Vec<RoundsObservation>,
        #[serde(default)]
        abort_mid_run: bool,
    },
    Retire {
        entry_id: String,
        resolved_at: String,
        reason: String,
    },
}

#[derive(Debug, Serialize)]
struct PatientMemorySnapshotResponse {
    workspace_id: WorkspaceId,
    rounds_watermark: u64,
    mutation_count: u64,
    entries: Vec<MemoryEntry>,
}

#[derive(Debug, Deserialize)]
struct DobInput {
    year: u16,
    month: u8,
    day: u8,
}

#[derive(Debug, Deserialize)]
struct CreateInvitationBody {
    patient_id: PatientId,
    #[serde(default)]
    workspace_id: Option<WorkspaceId>,
    practice_name: String,
    physician_name: String,
    expected_dob: DobInput,
    #[serde(default)]
    created_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InvitationActionTimestampBody {
    #[serde(default)]
    occurred_at: Option<String>,
}

#[derive(Debug, Serialize)]
struct IssuedInvitationResponse {
    invitation_id: InvitationId,
    link_token: ef_connected_care::InviteToken,
    short_code: ef_connected_care::InviteShortCode,
    standing: ConnectionStanding,
}

impl ClinicService {
    /// Routes `GET /elysianfields/api/today` enforcing per-physician clinical access and `TodaySaysSo`.
    pub(crate) fn route_today_get(&mut self, req: &HttpRequest) -> HttpResponse {
        if req.method != HttpMethod::Get {
            return HttpResponse::method_not_allowed();
        }
        let whoami = match self.authenticate_request(req) {
            Ok(w) => w,
            Err(resp) => return resp,
        };

        let mut candidates: Vec<TodayQueueItem> = self
            .today_candidates
            .iter()
            .filter(|item| match &item.workspace_id {
                None => true,
                Some(ws) => self
                    .evidence
                    .require_clinical_access(&whoami.practice_id, &whoami.physician_id, ws)
                    .is_ok(),
            })
            .cloned()
            .collect();

        // Project open `ToMatch` items from MessageMatchingEngine onto Today.
        let to_match =
            self.matching_engine
                .visible_to_match_for(&whoami.physician_id, |phys, ws| {
                    self.evidence
                        .require_clinical_access(&whoami.practice_id, phys, ws)
                        .is_ok()
                });
        for item in to_match {
            let item_id = format!("match:{}", item.conversation_id);
            if !candidates.iter().any(|c| c.item_id == item_id) {
                candidates.push(TodayQueueItem {
                    item_id,
                    workspace_id: item.suggested_patient,
                    urgency: TodayUrgency::Today,
                    waiting_since_epoch_secs: self.clock_secs,
                    kind: TodayItemKind::UnlinkedPatientMessage,
                    version_id: item.message_count as u64,
                    summary: format!(
                        "Unlinked patient conversation ({} messages)",
                        item.message_count
                    ),
                    can_mark_done: false,
                });
            }
        }

        let scheduled_visits = self
            .scheduled_visits
            .iter()
            .filter(|visit| {
                self.evidence
                    .require_clinical_access(
                        &whoami.practice_id,
                        &whoami.physician_id,
                        &visit.workspace_id,
                    )
                    .is_ok()
            })
            .cloned()
            .collect();

        let view = self.today_engine.build_view(
            &whoami.physician_id,
            &candidates,
            scheduled_visits,
            self.today_also_reviewed,
            Vec::new(),
        );
        HttpResponse::json_clinical(200, &view)
    }

    /// Routes `POST /elysianfields/api/today/standing` (`seen` or `done`).
    pub(crate) fn route_today_standing(&mut self, req: &HttpRequest) -> HttpResponse {
        if req.method != HttpMethod::Post {
            return HttpResponse::method_not_allowed();
        }
        let whoami = match self.authenticate_request(req) {
            Ok(w) => w,
            Err(resp) => return resp,
        };
        let body: TodayStandingBody = match serde_json::from_slice(&req.body) {
            Ok(parsed) => parsed,
            Err(err) => return HttpResponse::bad_request(err.to_string()),
        };

        let Some(item) = self
            .today_candidates
            .iter()
            .find(|c| c.item_id == body.item_id)
            .cloned()
        else {
            return HttpResponse::not_found();
        };
        if let Some(ws) = &item.workspace_id
            && let Err(err) =
                self.evidence
                    .require_clinical_access(&whoami.practice_id, &whoami.physician_id, ws)
        {
            return workspace_error_response(err);
        }

        match body.action {
            TodayStandingAction::Seen => {
                let version_id = body.version_id.unwrap_or(item.version_id);
                self.today_engine
                    .mark_seen(&whoami.physician_id, &item.item_id, version_id);
                if let Err(err) = self.wal.append(&WalEntry::MarkTodaySeen {
                    physician_id: whoami.physician_id,
                    item_id: item.item_id,
                    version_id,
                }) {
                    return wal_failure_response(&err);
                }
                HttpResponse::json_clinical(200, &serde_json::json!({ "status": "seen" }))
            }
            TodayStandingAction::Done => {
                let updated = self.today_engine.mark_done(&whoami.physician_id, &item);
                if !updated {
                    return HttpResponse::bad_request("item cannot be marked done");
                }
                if let Err(err) = self.wal.append(&WalEntry::MarkTodayDone {
                    physician_id: whoami.physician_id,
                    item,
                }) {
                    return wal_failure_response(&err);
                }
                HttpResponse::json_clinical(200, &serde_json::json!({ "status": "done" }))
            }
        }
    }

    /// Routes `POST /elysianfields/api/messages/ingest`.
    pub(crate) fn route_messages_ingest(&mut self, req: &HttpRequest) -> HttpResponse {
        if req.method != HttpMethod::Post {
            return HttpResponse::method_not_allowed();
        }
        let whoami = match self.authenticate_request(req) {
            Ok(w) => w,
            Err(resp) => return resp,
        };
        let body: IngestMessageBody = match serde_json::from_slice(&req.body) {
            Ok(parsed) => parsed,
            Err(err) => return HttpResponse::bad_request(err.to_string()),
        };

        let now_secs = self.clock_secs;
        let frame = if let Some(reason) = body.unreadable_reason.clone() {
            RawChannelFrame::Unreadable {
                cursor_seq: body.cursor_seq,
                reason,
            }
        } else {
            let (Some(conversation_id), Some(message_id), Some(msg_body)) = (
                body.conversation_id.clone(),
                body.message_id.clone(),
                body.body.clone(),
            ) else {
                return HttpResponse::bad_request(
                    "readable ingest requires conversation_id, message_id, and body",
                );
            };
            RawChannelFrame::Readable {
                cursor_seq: body.cursor_seq,
                conversation_id,
                message_id,
                body: msg_body,
            }
        };

        if let Err(err) =
            self.apply_ingest_message(&body.channel_id, frame, body.auto_signal.clone(), now_secs)
        {
            return matching_error_response(err);
        }

        if let Err(err) = self.wal.append(&WalEntry::IngestChannelMessage {
            channel_id: body.channel_id.clone(),
            cursor_seq: body.cursor_seq,
            conversation_id: body.conversation_id.clone(),
            message_id: body.message_id.clone(),
            body: body.body.clone(),
            unreadable_reason: body.unreadable_reason.clone(),
            auto_signal: body.auto_signal.clone(),
            now_secs,
        }) {
            return wal_failure_response(&err);
        }

        let conv_state = body
            .conversation_id
            .as_deref()
            .and_then(|id| self.matching_engine.conversation(id))
            .map(|conv| conv.state.clone());
        let to_match: Vec<ToMatchItemDto> = self
            .matching_engine
            .visible_to_match_for(&whoami.physician_id, |phys, ws| {
                self.evidence
                    .require_clinical_access(&whoami.practice_id, phys, ws)
                    .is_ok()
            })
            .into_iter()
            .map(|item| ToMatchItemDto {
                conversation_id: item.conversation_id,
                suggested_patient: item.suggested_patient,
                message_count: item.message_count,
            })
            .collect();

        HttpResponse::json_clinical(
            200,
            &serde_json::json!({
                "ingested": true,
                "channel_id": body.channel_id,
                "conversation_id": body.conversation_id,
                "state": conv_state,
                "to_match": to_match,
                "unreadable_count": self.matching_engine.visible_unreadable_entries().len(),
            }),
        )
    }

    /// Routes `POST /elysianfields/api/messages/decide`.
    pub(crate) fn route_messages_decide(&mut self, req: &HttpRequest) -> HttpResponse {
        if req.method != HttpMethod::Post {
            return HttpResponse::method_not_allowed();
        }
        let whoami = match self.authenticate_request(req) {
            Ok(w) => w,
            Err(resp) => return resp,
        };
        let body: DecideMessageBody = match serde_json::from_slice(&req.body) {
            Ok(parsed) => parsed,
            Err(err) => return HttpResponse::bad_request(err.to_string()),
        };

        let now_secs = self.clock_secs;
        let domain_dest: MoveDestination = body.destination.clone().into();
        if let Err(err) = self.apply_decide_move(
            &whoami.practice_id,
            &whoami.physician_id,
            &body.conversation_id,
            domain_dest,
            now_secs,
        ) {
            return matching_error_response(err);
        }

        if let Err(err) = self.wal.append(&WalEntry::DecideMessageMove {
            physician_id: whoami.physician_id,
            practice_id: whoami.practice_id,
            conversation_id: body.conversation_id.clone(),
            destination: body.destination,
            now_secs,
        }) {
            return wal_failure_response(&err);
        }

        let conv = self.matching_engine.conversation(&body.conversation_id);
        let undo_dto = conv.and_then(|c| c.undo.as_ref()).map(|u| UndoWindowDto {
            target: WalMoveDestination::from(&u.target),
            opened_at: u.opened_at,
            expires_at: u.expires_at,
        });
        HttpResponse::json_clinical(
            200,
            &serde_json::json!({
                "conversation_id": body.conversation_id,
                "state": conv.map(|c| &c.state),
                "undo": undo_dto,
            }),
        )
    }

    /// Routes `POST /elysianfields/api/messages/undo`.
    pub(crate) fn route_messages_undo(&mut self, req: &HttpRequest) -> HttpResponse {
        if req.method != HttpMethod::Post {
            return HttpResponse::method_not_allowed();
        }
        let _whoami = match self.authenticate_request(req) {
            Ok(w) => w,
            Err(resp) => return resp,
        };
        let body: UndoMessageBody = match serde_json::from_slice(&req.body) {
            Ok(parsed) => parsed,
            Err(err) => return HttpResponse::bad_request(err.to_string()),
        };

        if let Err(err) = self.matching_engine.undo_move(&body.conversation_id) {
            return matching_error_response(err);
        }

        if let Err(err) = self.wal.append(&WalEntry::UndoMessageMove {
            conversation_id: body.conversation_id.clone(),
        }) {
            return wal_failure_response(&err);
        }

        let conv = self.matching_engine.conversation(&body.conversation_id);
        let undo_dto = conv.and_then(|c| c.undo.as_ref()).map(|u| UndoWindowDto {
            target: WalMoveDestination::from(&u.target),
            opened_at: u.opened_at,
            expires_at: u.expires_at,
        });
        HttpResponse::json_clinical(
            200,
            &serde_json::json!({
                "conversation_id": body.conversation_id,
                "state": conv.map(|c| &c.state),
                "undo": undo_dto,
            }),
        )
    }

    /// Routes `GET /elysianfields/api/replies/:conversation_id` and `POST /elysianfields/api/replies/:conversation_id/<action>`.
    pub(crate) fn route_replies(
        &mut self,
        req: &HttpRequest,
        conversation_id: &str,
        action: Option<&str>,
    ) -> HttpResponse {
        let whoami = match self.authenticate_request(req) {
            Ok(w) => w,
            Err(resp) => return resp,
        };

        let practice_id = whoami.practice_id;
        let physician_id = whoami.physician_id;

        match (req.method, action) {
            (HttpMethod::Get, None) => {
                match self.reply_engine.derive_reply_card(
                    &physician_id,
                    conversation_id,
                    |phys, ws| {
                        self.evidence
                            .require_clinical_access(&practice_id, phys, ws)
                            .is_ok()
                    },
                ) {
                    Ok(card) => HttpResponse::json_clinical(
                        200,
                        &ReplyCardResponse::from_card(conversation_id.to_owned(), card),
                    ),
                    Err(err) => reply_error_response(err),
                }
            }
            (HttpMethod::Post, Some("review")) => {
                let body: CompleteReplyReviewBody = match serde_json::from_slice(&req.body) {
                    Ok(parsed) => parsed,
                    Err(err) => return HttpResponse::bad_request(err.to_string()),
                };
                if let Some(ref text) = body.proposed_reply_text
                    && contains_internal_citation_markup(text)
                {
                    return reply_error_response(ReplyError::InternalMarkupForbidden);
                }
                // Authorize before executing the clinical review.
                if let Err(err) = self.reply_engine.derive_reply_card(
                    &physician_id,
                    conversation_id,
                    |phys, ws| {
                        self.evidence
                            .require_clinical_access(&practice_id, phys, ws)
                            .is_ok()
                    },
                ) {
                    return reply_error_response(err);
                }
                let (review_id, for_patient) = match self.reply_engine.start_review(conversation_id)
                {
                    Ok(pair) => pair,
                    Err(err) => return reply_error_response(err),
                };
                let completion = ReviewCompletion {
                    review_id,
                    for_patient: for_patient.clone(),
                    proposed_reply_text: body.proposed_reply_text.clone(),
                    basis_citations: body.basis_citations.clone(),
                    flagged_concern: body.flagged_concern.clone(),
                    answered_by_human_reply: body.answered_by_human_reply,
                };
                match self
                    .reply_engine
                    .complete_review(conversation_id, completion)
                {
                    Ok(applied) => {
                        if let Err(err) = self.wal.append(&WalEntry::ReplyCompleteReview {
                            conversation_id: conversation_id.to_owned(),
                            for_patient,
                            proposed_reply_text: body.proposed_reply_text,
                            basis_citations: body.basis_citations,
                            flagged_concern: body.flagged_concern,
                            answered_by_human_reply: body.answered_by_human_reply,
                        }) {
                            return wal_failure_response(&err);
                        }
                        let card = self
                            .reply_engine
                            .derive_reply_card(&physician_id, conversation_id, |phys, ws| {
                                self.evidence
                                    .require_clinical_access(&practice_id, phys, ws)
                                    .is_ok()
                            })
                            .ok()
                            .map(|c| ReplyCardResponse::from_card(conversation_id.to_owned(), c));
                        HttpResponse::json_clinical(
                            200,
                            &serde_json::json!({
                                "applied": applied,
                                "card": card,
                            }),
                        )
                    }
                    Err(err) => reply_error_response(err),
                }
            }
            (HttpMethod::Post, Some("draft")) => {
                let body: EditReplyDraftBody = match serde_json::from_slice(&req.body) {
                    Ok(parsed) => parsed,
                    Err(err) => return HttpResponse::bad_request(err.to_string()),
                };
                match self.reply_engine.edit_personal_draft(
                    &physician_id,
                    conversation_id,
                    body.text.clone(),
                    |phys, ws| {
                        self.evidence
                            .require_clinical_access(&practice_id, phys, ws)
                            .is_ok()
                    },
                ) {
                    Ok(()) => {
                        if let Err(err) = self.wal.append(&WalEntry::ReplyEditDraft {
                            physician_id: physician_id.clone(),
                            practice_id: practice_id.clone(),
                            conversation_id: conversation_id.to_owned(),
                            draft_text: body.text,
                        }) {
                            return wal_failure_response(&err);
                        }
                        self.reply_card_json_response(&practice_id, &physician_id, conversation_id)
                    }
                    Err(err) => reply_error_response(err),
                }
            }
            (HttpMethod::Post, Some("restore")) => {
                match self.reply_engine.restore_system_reply(
                    &physician_id,
                    conversation_id,
                    |phys, ws| {
                        self.evidence
                            .require_clinical_access(&practice_id, phys, ws)
                            .is_ok()
                    },
                ) {
                    Ok(()) => {
                        if let Err(err) = self.wal.append(&WalEntry::ReplyRestoreSystem {
                            physician_id: physician_id.clone(),
                            practice_id: practice_id.clone(),
                            conversation_id: conversation_id.to_owned(),
                        }) {
                            return wal_failure_response(&err);
                        }
                        self.reply_card_json_response(&practice_id, &physician_id, conversation_id)
                    }
                    Err(err) => reply_error_response(err),
                }
            }
            (HttpMethod::Post, Some("copy")) => {
                match self
                    .reply_engine
                    .copy_reply(&physician_id, conversation_id, |phys, ws| {
                        self.evidence
                            .require_clinical_access(&practice_id, phys, ws)
                            .is_ok()
                    }) {
                    Ok(copied) => HttpResponse::json_clinical(
                        200,
                        &serde_json::json!({
                            "conversation_id": conversation_id,
                            "copied_text": copied,
                        }),
                    ),
                    Err(err) => reply_error_response(err),
                }
            }
            (HttpMethod::Post, Some("close")) => {
                match self
                    .reply_engine
                    .close_reply(&physician_id, conversation_id, |phys, ws| {
                        self.evidence
                            .require_clinical_access(&practice_id, phys, ws)
                            .is_ok()
                    }) {
                    Ok(()) => {
                        if let Err(err) = self.wal.append(&WalEntry::ReplyClose {
                            physician_id: physician_id.clone(),
                            practice_id: practice_id.clone(),
                            conversation_id: conversation_id.to_owned(),
                        }) {
                            return wal_failure_response(&err);
                        }
                        self.reply_card_json_response(&practice_id, &physician_id, conversation_id)
                    }
                    Err(err) => reply_error_response(err),
                }
            }
            (HttpMethod::Post, Some("undo")) => {
                match self.reply_engine.undo_close_reply(
                    &physician_id,
                    conversation_id,
                    |phys, ws| {
                        self.evidence
                            .require_clinical_access(&practice_id, phys, ws)
                            .is_ok()
                    },
                ) {
                    Ok(()) => {
                        if let Err(err) = self.wal.append(&WalEntry::ReplyUndoClose {
                            physician_id: physician_id.clone(),
                            practice_id: practice_id.clone(),
                            conversation_id: conversation_id.to_owned(),
                        }) {
                            return wal_failure_response(&err);
                        }
                        self.reply_card_json_response(&practice_id, &physician_id, conversation_id)
                    }
                    Err(err) => reply_error_response(err),
                }
            }
            _ => HttpResponse::method_not_allowed(),
        }
    }

    fn reply_card_json_response(
        &self,
        practice_id: &PracticeId,
        physician_id: &PhysicianId,
        conversation_id: &str,
    ) -> HttpResponse {
        match self
            .reply_engine
            .derive_reply_card(physician_id, conversation_id, |phys, ws| {
                self.evidence
                    .require_clinical_access(practice_id, phys, ws)
                    .is_ok()
            }) {
            Ok(card) => HttpResponse::json_clinical(
                200,
                &ReplyCardResponse::from_card(conversation_id.to_owned(), card),
            ),
            Err(err) => reply_error_response(err),
        }
    }

    /// Routes `GET` and `POST /elysianfields/api/patients/:id/memory`.
    pub(crate) fn route_patient_memory(
        &mut self,
        req: &HttpRequest,
        raw_workspace_id: &str,
    ) -> HttpResponse {
        let whoami = match self.authenticate_request(req) {
            Ok(w) => w,
            Err(resp) => return resp,
        };
        let workspace_id = match WorkspaceId::new(raw_workspace_id) {
            Ok(id) => id,
            Err(err) => return HttpResponse::bad_request(err.to_string()),
        };
        if let Err(err) = self.evidence.require_clinical_access(
            &whoami.practice_id,
            &whoami.physician_id,
            &workspace_id,
        ) {
            return workspace_error_response(err);
        }

        match req.method {
            HttpMethod::Get => {
                let snapshot = self.patient_memory_snapshot(&workspace_id);
                HttpResponse::json_clinical(200, &snapshot)
            }
            HttpMethod::Post => {
                let mutation: MemoryMutationRequest = match serde_json::from_slice(&req.body) {
                    Ok(parsed) => parsed,
                    Err(err) => return HttpResponse::bad_request(err.to_string()),
                };
                let store = self
                    .patient_memories
                    .entry(workspace_id.clone())
                    .or_insert_with(|| PatientMemoryStore::new(workspace_id.clone()));

                let outcome_json = match mutation {
                    MemoryMutationRequest::Record {
                        topic,
                        statement,
                        observed_at,
                        authority,
                    } => {
                        let entry_id = store.record_entry(
                            topic.clone(),
                            statement.clone(),
                            observed_at.clone(),
                            authority,
                        );
                        if let Err(err) = self.wal.append(&WalEntry::MemoryRecordEntry {
                            workspace_id: workspace_id.clone(),
                            topic,
                            statement,
                            observed_at,
                            authority,
                        }) {
                            return wal_failure_response(&err);
                        }
                        serde_json::json!({ "op": "record", "entry_id": entry_id })
                    }
                    MemoryMutationRequest::RunRounds {
                        observations,
                        abort_mid_run,
                    } => {
                        let outcome = store.run_rounds(&observations, abort_mid_run);
                        if let Err(err) = self.wal.append(&WalEntry::MemoryRunRounds {
                            workspace_id: workspace_id.clone(),
                            observations,
                            abort_mid_run,
                        }) {
                            return wal_failure_response(&err);
                        }
                        match outcome {
                            RoundsRunOutcome::QuietNoOp { watermark } => serde_json::json!({
                                "op": "run_rounds",
                                "outcome": "quiet_no_op",
                                "watermark": watermark,
                            }),
                            RoundsRunOutcome::Updated {
                                new_watermark,
                                added_entries,
                                disagreements_preserved,
                            } => serde_json::json!({
                                "op": "run_rounds",
                                "outcome": "updated",
                                "new_watermark": new_watermark,
                                "added_entries": added_entries,
                                "disagreements_preserved": disagreements_preserved,
                            }),
                            RoundsRunOutcome::Aborted {
                                unchanged_watermark,
                            } => serde_json::json!({
                                "op": "run_rounds",
                                "outcome": "aborted",
                                "unchanged_watermark": unchanged_watermark,
                            }),
                        }
                    }
                    MemoryMutationRequest::Retire {
                        entry_id,
                        resolved_at,
                        reason,
                    } => {
                        let retired =
                            store.retire_entry(&entry_id, resolved_at.clone(), reason.clone());
                        if !retired {
                            return HttpResponse::not_found();
                        }
                        if let Err(err) = self.wal.append(&WalEntry::MemoryRetireEntry {
                            workspace_id: workspace_id.clone(),
                            entry_id: entry_id.clone(),
                            resolved_at,
                            reason,
                        }) {
                            return wal_failure_response(&err);
                        }
                        serde_json::json!({ "op": "retire", "entry_id": entry_id, "retired": true })
                    }
                };

                let snapshot = self.patient_memory_snapshot(&workspace_id);
                HttpResponse::json_clinical(
                    200,
                    &serde_json::json!({
                        "result": outcome_json,
                        "memory": snapshot,
                    }),
                )
            }
            HttpMethod::Head | HttpMethod::Put => HttpResponse::method_not_allowed(),
        }
    }

    fn patient_memory_snapshot(&self, workspace_id: &WorkspaceId) -> PatientMemorySnapshotResponse {
        match self.patient_memories.get(workspace_id) {
            Some(store) => PatientMemorySnapshotResponse {
                workspace_id: workspace_id.clone(),
                rounds_watermark: store.rounds_watermark(),
                mutation_count: store.mutation_count(),
                entries: store.all_entries().to_vec(),
            },
            None => PatientMemorySnapshotResponse {
                workspace_id: workspace_id.clone(),
                rounds_watermark: 0,
                mutation_count: 0,
                entries: Vec::new(),
            },
        }
    }

    /// Routes `POST /elysianfields/api/connected-care/invitations`.
    pub(crate) fn route_connected_care_create_invitation(
        &mut self,
        req: &HttpRequest,
    ) -> HttpResponse {
        if req.method != HttpMethod::Post {
            return HttpResponse::method_not_allowed();
        }
        let whoami = match self.authenticate_request(req) {
            Ok(w) => w,
            Err(resp) => return resp,
        };
        let body: CreateInvitationBody = match serde_json::from_slice(&req.body) {
            Ok(parsed) => parsed,
            Err(err) => return HttpResponse::bad_request(err.to_string()),
        };

        if let Some(ws_id) = &body.workspace_id
            && let Err(err) = self.evidence.require_clinical_access(
                &whoami.practice_id,
                &whoami.physician_id,
                ws_id,
            )
        {
            return workspace_error_response(err);
        }

        let dob = match DateOfBirth::new(
            body.expected_dob.year,
            body.expected_dob.month,
            body.expected_dob.day,
        ) {
            Ok(d) => d,
            Err(err) => return bridge_error_response(err),
        };

        let pair = PairKey::new(whoami.practice_id, body.patient_id);
        let created_at = body
            .created_at
            .unwrap_or_else(|| format!("epoch:{}", self.clock_secs));
        let request = CreateInvitationRequest {
            pair: pair.clone(),
            issued_by: whoami.physician_id,
            practice_name: body.practice_name,
            physician_name: body.physician_name,
            expected_dob: dob,
            created_at,
        };

        let issued = match self.connected_care.create_invitation(request.clone()) {
            Ok(issued) => issued,
            Err(err) => return bridge_error_response(err),
        };
        self.invitation_pairs
            .insert(issued.invitation_id.clone(), pair.clone());

        if let Err(err) = self
            .wal
            .append(&WalEntry::CreateConnectedCareInvitation { request })
        {
            return wal_failure_response(&err);
        }

        let standing = self.connected_care.clinic_standing(&pair);
        HttpResponse::json_clinical(
            200,
            &IssuedInvitationResponse {
                invitation_id: issued.invitation_id,
                link_token: issued.link_token,
                short_code: issued.short_code,
                standing,
            },
        )
    }

    /// Routes `GET /elysianfields/api/connected-care/invitations/:id`.
    pub(crate) fn route_connected_care_get_invitation(
        &mut self,
        req: &HttpRequest,
        raw_invitation_id: &str,
    ) -> HttpResponse {
        if req.method != HttpMethod::Get {
            return HttpResponse::method_not_allowed();
        }
        let whoami = match self.authenticate_request(req) {
            Ok(w) => w,
            Err(resp) => return resp,
        };
        let inv_id = match InvitationId::new(raw_invitation_id) {
            Ok(id) => id,
            Err(err) => return HttpResponse::bad_request(err.to_string()),
        };
        let Some(pair) = self.invitation_pairs.get(&inv_id).cloned() else {
            return HttpResponse::not_found();
        };
        if pair.practice_id != whoami.practice_id {
            return HttpResponse::not_found();
        }
        let standing = self.connected_care.clinic_standing(&pair);
        HttpResponse::json_clinical(200, &standing)
    }

    /// Routes `POST /elysianfields/api/connected-care/invitations/:id/cancel`.
    pub(crate) fn route_connected_care_cancel_invitation(
        &mut self,
        req: &HttpRequest,
        raw_invitation_id: &str,
    ) -> HttpResponse {
        if req.method != HttpMethod::Post {
            return HttpResponse::method_not_allowed();
        }
        let whoami = match self.authenticate_request(req) {
            Ok(w) => w,
            Err(resp) => return resp,
        };
        let inv_id = match InvitationId::new(raw_invitation_id) {
            Ok(id) => id,
            Err(err) => return HttpResponse::bad_request(err.to_string()),
        };
        let Some(pair) = self.invitation_pairs.get(&inv_id).cloned() else {
            return HttpResponse::not_found();
        };
        if pair.practice_id != whoami.practice_id {
            return HttpResponse::not_found();
        }
        let standing_before = self.connected_care.clinic_standing(&pair);
        if standing_before.active_invitation_id.as_ref() != Some(&inv_id) {
            return HttpResponse::json_clinical(
                409,
                &serde_json::json!({
                    "error": "invitation_not_open",
                    "standing": standing_before,
                }),
            );
        }

        let occurred_at = parse_optional_occurred_at(&req.body, self.clock_secs);
        let cancelled_id = match self
            .connected_care
            .cancel_invitation(&pair, occurred_at.clone())
        {
            Ok(id) => id,
            Err(err) => return bridge_error_response(err),
        };

        if let Err(err) = self.wal.append(&WalEntry::CancelConnectedCareInvitation {
            pair: pair.clone(),
            cancelled_at: occurred_at,
        }) {
            return wal_failure_response(&err);
        }

        let standing = self.connected_care.clinic_standing(&pair);
        HttpResponse::json_clinical(
            200,
            &serde_json::json!({
                "cancelled_invitation_id": cancelled_id,
                "standing": standing,
            }),
        )
    }

    /// Routes `POST /elysianfields/api/connected-care/invitations/:id/resend`.
    pub(crate) fn route_connected_care_resend_invitation(
        &mut self,
        req: &HttpRequest,
        raw_invitation_id: &str,
    ) -> HttpResponse {
        if req.method != HttpMethod::Post {
            return HttpResponse::method_not_allowed();
        }
        let whoami = match self.authenticate_request(req) {
            Ok(w) => w,
            Err(resp) => return resp,
        };
        let inv_id = match InvitationId::new(raw_invitation_id) {
            Ok(id) => id,
            Err(err) => return HttpResponse::bad_request(err.to_string()),
        };
        let Some(pair) = self.invitation_pairs.get(&inv_id).cloned() else {
            return HttpResponse::not_found();
        };
        if pair.practice_id != whoami.practice_id {
            return HttpResponse::not_found();
        }
        let standing_before = self.connected_care.clinic_standing(&pair);
        if standing_before.active_invitation_id.as_ref() != Some(&inv_id) {
            return HttpResponse::json_clinical(
                409,
                &serde_json::json!({
                    "error": "invitation_not_open",
                    "standing": standing_before,
                }),
            );
        }

        let occurred_at = parse_optional_occurred_at(&req.body, self.clock_secs);
        let issued = match self
            .connected_care
            .resend_invitation(&pair, occurred_at.clone())
        {
            Ok(issued) => issued,
            Err(err) => return bridge_error_response(err),
        };

        if let Err(err) = self.wal.append(&WalEntry::ResendConnectedCareInvitation {
            pair: pair.clone(),
            resent_at: occurred_at,
        }) {
            return wal_failure_response(&err);
        }

        let standing = self.connected_care.clinic_standing(&pair);
        HttpResponse::json_clinical(
            200,
            &IssuedInvitationResponse {
                invitation_id: issued.invitation_id,
                link_token: issued.link_token,
                short_code: issued.short_code,
                standing,
            },
        )
    }

    pub(crate) fn apply_ingest_message(
        &mut self,
        channel_id: &str,
        frame: RawChannelFrame,
        auto_signal: Option<WalAutoMatchSignal>,
        now_secs: u64,
    ) -> Result<(), MatchingError> {
        if self
            .matching_engine
            .backlog_consent_state(channel_id)
            .is_none()
        {
            self.matching_engine
                .register_channel(channel_id, ChannelHealthStatus::Connected);
        }
        let readable_info = match &frame {
            RawChannelFrame::Readable {
                conversation_id,
                message_id,
                body,
                ..
            } => Some((conversation_id.clone(), message_id.clone(), body.clone())),
            RawChannelFrame::Unreadable { .. } => None,
        };

        self.matching_engine
            .ingest_channel_frame(channel_id, frame, now_secs)?;

        if let (Some((conv_id, msg_id, msg_body)), Some(signal)) = (readable_info, auto_signal) {
            let domain_signal = match signal {
                WalAutoMatchSignal::UnambiguousSingle { patient } => {
                    AutomatedMatchSignal::UnambiguousSingle(patient)
                }
                WalAutoMatchSignal::AmbiguousOrContradicted { suggested } => {
                    AutomatedMatchSignal::AmbiguousOrContradicted { suggested }
                }
                WalAutoMatchSignal::LikelyNonPatient => AutomatedMatchSignal::LikelyNonPatient,
            };
            self.matching_engine
                .apply_automated_match(&conv_id, domain_signal, now_secs)?;

            if let Some(conv) = self.matching_engine.conversation(&conv_id)
                && let ConversationMatchState::Linked { patient } = &conv.state
            {
                self.reply_engine
                    .register_conversation(conv_id.clone(), patient.clone(), true);
                let _ = self
                    .reply_engine
                    .receive_patient_message(&conv_id, msg_id, msg_body);
            }
        }
        Ok(())
    }

    pub(crate) fn apply_decide_move(
        &mut self,
        practice_id: &PracticeId,
        physician_id: &PhysicianId,
        conversation_id: &str,
        destination: MoveDestination,
        now_secs: u64,
    ) -> Result<(), MatchingError> {
        self.matching_engine.move_conversation(
            physician_id,
            conversation_id,
            destination.clone(),
            |phys, ws| {
                self.evidence
                    .require_clinical_access(practice_id, phys, ws)
                    .is_ok()
            },
            now_secs,
        )?;

        // Sync MessageReplyEngine (`WithheldAfterMove` + register newly linked conversation).
        if self
            .reply_engine
            .on_conversation_moved(conversation_id, destination.clone())
            .is_err()
            && let MoveDestination::Patient(patient) = destination
            && let Some(conv) = self.matching_engine.conversation(conversation_id)
        {
            self.reply_engine
                .register_conversation(conversation_id.to_owned(), patient, true);
            for msg in &conv.messages {
                let _ = self.reply_engine.receive_patient_message(
                    conversation_id,
                    msg.message_id.clone(),
                    msg.body.clone(),
                );
            }
        }
        Ok(())
    }
}

fn parse_optional_occurred_at(body: &[u8], clock_secs: u64) -> String {
    if !body.is_empty()
        && let Ok(parsed) = serde_json::from_slice::<InvitationActionTimestampBody>(body)
        && let Some(ts) = parsed.occurred_at
    {
        return ts;
    }
    format!("epoch:{clock_secs}")
}

fn matching_error_response(err: MatchingError) -> HttpResponse {
    let status = match err {
        MatchingError::ConversationNotFound { .. } => 404,
        MatchingError::UnauthorizedPatient { .. } => 403,
        MatchingError::CursorSequenceMismatch { .. }
        | MatchingError::NoActiveUndo { .. }
        | MatchingError::BacklogConsentRequired { .. }
        | MatchingError::ChannelDisconnected { .. } => 409,
    };
    HttpResponse::json_clinical(
        status,
        &serde_json::json!({
            "error": "message_matching_error",
            "detail": err.to_string(),
        }),
    )
}

fn reply_error_response(err: ReplyError) -> HttpResponse {
    let status = match err {
        ReplyError::ConversationNotFound { .. } => 404,
        ReplyError::Unauthorized { .. } => 403,
        ReplyError::InternalMarkupForbidden => 400,
        ReplyError::ReviewAlreadyRunning { .. }
        | ReplyError::NoReadyReply
        | ReplyError::NoUndoAvailable => 409,
    };
    HttpResponse::json_clinical(
        status,
        &serde_json::json!({
            "error": "message_reply_error",
            "detail": err.to_string(),
        }),
    )
}

fn bridge_error_response(err: BridgeError) -> HttpResponse {
    let status = match err {
        BridgeError::InvalidId(_)
        | BridgeError::InvalidDateOfBirth { .. }
        | BridgeError::InvalidAccountIncarnation(_)
        | BridgeError::EmptyField { .. }
        | BridgeError::EmptyTransferEnvelope => 400,
        BridgeError::UnauthorizedSender { .. } | BridgeError::UnauthorizedPractice { .. } => 403,
        BridgeError::InvalidOrRevokedCredential { .. } | BridgeError::TransferNotFound { .. } => {
            404
        }
        BridgeError::CannotInviteInState { .. }
        | BridgeError::NoOpenInvitation { .. }
        | BridgeError::InvitationLocked { .. }
        | BridgeError::InvitationCancelled { .. }
        | BridgeError::DobVerificationRequired { .. }
        | BridgeError::ConnectionNotActive { .. }
        | BridgeError::Part2WrittenConsentRequired
        | BridgeError::Part2ConsentScopeMismatch
        | BridgeError::Part2VerbatimNoticeMissing { .. }
        | BridgeError::InvalidTransferTransition { .. }
        | BridgeError::AccountDeleted { .. } => 409,
    };
    HttpResponse::json_clinical(
        status,
        &serde_json::json!({
            "error": "connected_care_error",
            "detail": err.to_string(),
        }),
    )
}
