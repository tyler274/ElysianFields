//! Single-owner clinical workstation workflow resource for Curie Clinic Desktop's
//! 4 tabs: `Chat`, `Today`, `Messages`, and `Memory & Care`.
//!
//! Delegates all queue sorting, channel health banner derivation (`TodaySaysSo`),
//! message reply drafting/withholding/undo (`MessageReply.tla`), longitudinal memory
//! provenance/disagreement preservation, and Connected Care invitation issuance directly
//! to the authoritative `ef-clinic-workflows` and `ef-connected-care` domain owners.

use std::collections::BTreeMap;

use bevy::prelude::Resource;
use ef_clinic_workflows::memory::{
    MemoryAuthority, MemoryEntry, MemoryEntryState, PatientMemoryStore,
};
use ef_clinic_workflows::replies::{
    DerivedReplyCard, MessageReplyEngine, ReplyError, ReviewCompletion, VisibleReplyOrigin,
};
use ef_clinic_workflows::today::{
    ChannelHealthStatus, RenderedTodayItem, ScheduledVisitEntry, TodayChannelBanner, TodayItemKind,
    TodayQueueEngine, TodayQueueItem, TodayUrgency, TodayViewState,
};
use ef_connected_care::{
    BridgeError, ConnectedCareBridge, ConnectedCareState, CreateInvitationRequest, DateOfBirth,
    IssuedInvitation, PairKey,
};
use ef_core::{PatientId, PhysicianId, PracticeId, SourceKey, WorkspaceId};
use ef_ui_catalog::{Locale, UiKey, translate};

use crate::worker::{LiveReplyCard, LiveReplyOrigin, TodayStandingUpdate};

/// Live-server Today snapshot received asynchronously from `GET /elysianfields/api/today`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveTodaySnapshot {
    pub items: Vec<RenderedTodayItem>,
    pub scheduled_visits: Vec<ScheduledVisitEntry>,
    pub channel_banners: Vec<TodayChannelBanner>,
}

/// Live-server Connected Care invitation issued via
/// `POST /elysianfields/api/workspaces/{workspace_id}/connected-care/invitations`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveIssuedInvite {
    pub invitation_id: String,
    pub invite_code: String,
}

/// Explicit typed active tab in the 4-tab Curie Clinic Physician Workstation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ClinicWorkspaceTab {
    #[default]
    Chat,
    Today,
    Messages,
    MemoryCare,
}

impl ClinicWorkspaceTab {
    pub const ALL: [Self; 4] = [Self::Chat, Self::Today, Self::Messages, Self::MemoryCare];

    /// Parses a validated `--view <chat|today|messages|memory>` CLI argument.
    #[must_use]
    pub fn parse_cli(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "chat" => Some(Self::Chat),
            "today" => Some(Self::Today),
            "messages" => Some(Self::Messages),
            "memory" => Some(Self::MemoryCare),
            _ => None,
        }
    }

    #[must_use]
    pub const fn ui_key(self) -> UiKey {
        match self {
            Self::Chat => UiKey::TabChat,
            Self::Today => UiKey::TabToday,
            Self::Messages => UiKey::TabMessages,
            Self::MemoryCare => UiKey::TabMemoryCare,
        }
    }

    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Chat => 0,
            Self::Today => 1,
            Self::Messages => 2,
            Self::MemoryCare => 3,
        }
    }
}

/// Authoritative workstation workflow state backed by real domain engines.
#[derive(Debug, Resource)]
pub struct ClinicWorkflowState {
    pub active_tab: ClinicWorkspaceTab,
    pub physician_id: PhysicianId,
    pub practice_id: PracticeId,
    pub today_engine: TodayQueueEngine,
    pub today_candidates: Vec<TodayQueueItem>,
    pub scheduled_visits: Vec<ScheduledVisitEntry>,
    pub today_also_reviewed: usize,
    pub reply_engine: MessageReplyEngine,
    pub active_conversation_id: String,
    pub memories: BTreeMap<WorkspaceId, PatientMemoryStore>,
    pub connected_care: ConnectedCareBridge,
    pub active_invites: BTreeMap<WorkspaceId, IssuedInvitation>,
    pub live_today: Option<LiveTodaySnapshot>,
    pub live_memories: BTreeMap<WorkspaceId, Vec<MemoryEntry>>,
    pub live_invites: BTreeMap<WorkspaceId, LiveIssuedInvite>,
    pub live_replies: BTreeMap<WorkspaceId, Vec<LiveReplyCard>>,
    pub reply_draft_editor: String,
}

impl Default for ClinicWorkflowState {
    fn default() -> Self {
        Self::seeded_demo(ClinicWorkspaceTab::Chat)
    }
}

impl ClinicWorkflowState {
    /// Seeds realistic synthetic Today, Messages, Longitudinal Memory, and Connected Care
    /// state through the exact `ef-clinic-workflows` and `ef-connected-care` domain APIs.
    #[must_use]
    pub fn seeded_demo(initial_tab: ClinicWorkspaceTab) -> Self {
        let practice_id =
            PracticeId::new("prac-desktop-demo").unwrap_or_else(|_| unreachable!("static id"));
        let physician_id =
            PhysicianId::new("phys-desktop-demo").unwrap_or_else(|_| unreachable!("static id"));

        let ws_avery =
            WorkspaceId::new("ws-avery-jordan").unwrap_or_else(|_| unreachable!("static id"));
        let ws_patel =
            WorkspaceId::new("ws-patel-priya").unwrap_or_else(|_| unreachable!("static id"));
        let ws_cohen =
            WorkspaceId::new("ws-cohen-avi").unwrap_or_else(|_| unreachable!("static id"));

        let src_avery_1 =
            SourceKey::new("src-avery-note-1").unwrap_or_else(|_| unreachable!("static id"));
        let src_avery_2 =
            SourceKey::new("src-avery-labs-2").unwrap_or_else(|_| unreachable!("static id"));

        // 1. Today Queue Engine (`TodaySaysSo` channel health + urgency sorting Now before Today).
        let mut today_engine = TodayQueueEngine::new();
        today_engine.set_channel_health(
            "spruce-sms",
            ChannelHealthStatus::TempError {
                detail: "Temporary carrier delay; retrying automatically.".to_owned(),
            },
        );
        today_engine.set_channel_health("practice-gmail", ChannelHealthStatus::Connected);

        // Intentionally seed a `Today` item ahead of `Now` items so `build_view` proves sorting.
        let today_candidates = vec![
            TodayQueueItem {
                item_id: "today-patel-lipids".to_owned(),
                workspace_id: Some(ws_patel.clone()),
                urgency: TodayUrgency::Today,
                waiting_since_epoch_secs: 1_760_000_900,
                kind: TodayItemKind::SameDayClinicalConcern,
                version_id: 1,
                summary: "Patel, Priya: Lipid panel LDL 84 mg/dL ready for routine sign-off."
                    .to_owned(),
                can_mark_done: true,
            },
            TodayQueueItem {
                item_id: "today-avery-glucose".to_owned(),
                workspace_id: Some(ws_avery.clone()),
                urgency: TodayUrgency::Now,
                waiting_since_epoch_secs: 1_760_000_120,
                kind: TodayItemKind::UrgentClinicalConcern,
                version_id: 2,
                summary:
                    "Avery, Jordan: Post-prandial glucose 186 mg/dL with Metformin dosing question."
                        .to_owned(),
                can_mark_done: true,
            },
            TodayQueueItem {
                item_id: "today-cohen-dizziness".to_owned(),
                workspace_id: Some(ws_cohen.clone()),
                urgency: TodayUrgency::Now,
                waiting_since_epoch_secs: 1_760_000_360,
                kind: TodayItemKind::UnreadPatientMessage,
                version_id: 1,
                summary: "Cohen, Avi: Patient message reporting mild orthostatic lightheadedness."
                    .to_owned(),
                can_mark_done: true,
            },
        ];

        let scheduled_visits = vec![
            ScheduledVisitEntry {
                workspace_id: ws_avery.clone(),
                display_name: "Avery, Jordan".to_owned(),
                scheduled_at_epoch_secs: 1_760_001_800,
                briefing_ready: true,
            },
            ScheduledVisitEntry {
                workspace_id: ws_patel.clone(),
                display_name: "Patel, Priya".to_owned(),
                scheduled_at_epoch_secs: 1_760_003_600,
                briefing_ready: true,
            },
            ScheduledVisitEntry {
                workspace_id: ws_cohen.clone(),
                display_name: "Cohen, Avi".to_owned(),
                scheduled_at_epoch_secs: 1_760_005_400,
                briefing_ready: false,
            },
        ];

        // 2. Message Reply Engine (`MessageReply.tla` invariants: `CopiesWhatYouSee`, `ConcernStays`, `UndoBringsBack`).
        let mut reply_engine = MessageReplyEngine::new();
        let active_conversation_id = "conv-avery-msg-1".to_owned();
        reply_engine.register_conversation(&active_conversation_id, ws_avery.clone(), true);
        let _ = reply_engine.receive_patient_message(
            &active_conversation_id,
            "msg-avery-1",
            "Hi Dr. Curie, my fingerstick after lunch was 186 mg/dL. Should I change my Metformin dose before Friday's visit?",
        );
        if let Ok((review_id, for_patient)) = reply_engine.start_review(&active_conversation_id) {
            let _ = reply_engine.complete_review(
                &active_conversation_id,
                ReviewCompletion {
                    review_id,
                    for_patient,
                    proposed_reply_text: Some(
                        "Thank you for sharing your afternoon reading. Please continue Metformin 1000 mg twice daily for now, log your readings over the next few days, and we will review dose options together at your upcoming visit."
                            .to_owned(),
                    ),
                    basis_citations: vec![src_avery_1, src_avery_2],
                    flagged_concern: Some(
                        "Post-prandial hyperglycemia (186 mg/dL); assess titration at follow-up."
                            .to_owned(),
                    ),
                    answered_by_human_reply: false,
                },
            );
        }

        // 3. Longitudinal Patient Memory Stores (`ChartRecord` vs `PhysicianInstruction`, disagreement preservation, retirement).
        let mut memories = BTreeMap::new();

        let mut avery_mem = PatientMemoryStore::new(ws_avery.clone());
        avery_mem.record_entry(
            "glycemic_plan",
            "Metabolic panel shows HbA1c 6.8% (down from 7.2%) with eGFR 82 mL/min/1.73m2.",
            "2026-10-16",
            MemoryAuthority::ChartRecord,
        );
        avery_mem.record_entry(
            "glycemic_plan",
            "Hold Metformin at 1000 mg BID until 5-day post-prandial log reviewed; avoid automatic escalation.",
            "2026-10-18",
            MemoryAuthority::PhysicianInstruction,
        );
        let prior_id = avery_mem.record_entry(
            "prior_hba1c",
            "HbA1c 7.4% before evening Metformin adherence improved.",
            "2026-07-10",
            MemoryAuthority::ChartRecord,
        );
        avery_mem.retire_entry(&prior_id, "2026-10-16", "Superseded by Oct 2026 HbA1c 6.8%");
        memories.insert(ws_avery, avery_mem);

        let mut patel_mem = PatientMemoryStore::new(ws_patel.clone());
        patel_mem.record_entry(
            "lipid_control",
            "LDL-C 84 mg/dL on Atorvastatin 20 mg PO QHS; home BP 118/74 mmHg.",
            "2026-10-12",
            MemoryAuthority::ChartRecord,
        );
        patel_mem.record_entry(
            "lipid_control",
            "Continue annual lipid panel; patient prefers morning lab slots.",
            "2026-10-12",
            MemoryAuthority::PhysicianInstruction,
        );
        memories.insert(ws_patel, patel_mem);

        let mut cohen_mem = PatientMemoryStore::new(ws_cohen.clone());
        cohen_mem.record_entry(
            "renal_monitoring",
            "eGFR 74 mL/min/1.73m2, Urine ACR 18 mg/g; stable on Lisinopril.",
            "2026-10-10",
            MemoryAuthority::ChartRecord,
        );
        cohen_mem.record_entry(
            "renal_monitoring",
            "Check orthostatic BP at next visit if lightheadedness persists.",
            "2026-10-18",
            MemoryAuthority::PhysicianInstruction,
        );
        memories.insert(ws_cohen, cohen_mem);

        Self {
            active_tab: initial_tab,
            physician_id,
            practice_id,
            today_engine,
            today_candidates,
            scheduled_visits,
            today_also_reviewed: 4,
            reply_engine,
            active_conversation_id,
            memories,
            connected_care: ConnectedCareBridge::new(),
            active_invites: BTreeMap::new(),
            live_today: None,
            live_memories: BTreeMap::new(),
            live_invites: BTreeMap::new(),
            live_replies: BTreeMap::new(),
            reply_draft_editor: String::new(),
        }
    }

    /// Applies a live server workflow snapshot loaded via `LivePromptWorker`.
    pub fn apply_live_workflow_snapshot(
        &mut self,
        workspace_id: WorkspaceId,
        today_items: Vec<RenderedTodayItem>,
        scheduled_visits: Vec<ScheduledVisitEntry>,
        channel_banners: Vec<TodayChannelBanner>,
        memory_entries: Vec<MemoryEntry>,
        reply_cards: Vec<LiveReplyCard>,
    ) {
        self.live_today = Some(LiveTodaySnapshot {
            items: today_items,
            scheduled_visits,
            channel_banners,
        });
        self.live_memories
            .insert(workspace_id.clone(), memory_entries);
        if let Some(first) = reply_cards.first() {
            self.active_conversation_id = first.conversation_id.clone();
            if self.reply_draft_editor.is_empty()
                && let Some(ref visible) = first.visible_reply_text
            {
                self.reply_draft_editor = visible.clone();
            }
        }
        self.live_replies.insert(workspace_id, reply_cards);
    }

    /// Updates or inserts a server-confirmed [`LiveReplyCard`] for `workspace_id`.
    pub fn apply_live_reply_card(&mut self, workspace_id: WorkspaceId, card: LiveReplyCard) {
        self.active_conversation_id = card.conversation_id.clone();
        self.reply_draft_editor = card.visible_reply_text.clone().unwrap_or_default();
        let cards = self.live_replies.entry(workspace_id).or_default();
        if let Some(existing) = cards
            .iter_mut()
            .find(|c| c.conversation_id == card.conversation_id)
        {
            *existing = card;
        } else {
            cards.push(card);
        }
    }

    /// Returns the active live reply card for `workspace_id`, when synchronized from `--server`.
    #[must_use]
    pub fn active_live_reply_card(
        &self,
        workspace_id: Option<&WorkspaceId>,
    ) -> Option<&LiveReplyCard> {
        let ws = workspace_id?;
        self.live_replies.get(ws).and_then(|cards| cards.first())
    }

    /// Returns the active reply conversation ID for `workspace_id` (live when present, else demo).
    #[must_use]
    pub fn active_reply_conversation_id(&self, workspace_id: Option<&WorkspaceId>) -> String {
        if let Some(card) = self.active_live_reply_card(workspace_id) {
            return card.conversation_id.clone();
        }
        self.active_conversation_id.clone()
    }

    /// Applies a server-confirmed Today standing update (`Seen` or `Done`).
    pub fn apply_live_today_standing(
        &mut self,
        item_id: &str,
        standing: TodayStandingUpdate,
        version_id: u64,
    ) {
        if let Some(live) = self.live_today.as_mut() {
            match standing {
                TodayStandingUpdate::Seen => {
                    if let Some(entry) = live.items.iter_mut().find(|r| r.item.item_id == item_id)
                        && version_id >= entry.item.version_id
                    {
                        entry.seen = true;
                    }
                }
                TodayStandingUpdate::Done => {
                    live.items.retain(|r| {
                        !(r.item.item_id == item_id && version_id >= r.item.version_id)
                    });
                }
            }
            return;
        }

        match standing {
            TodayStandingUpdate::Seen => {
                self.today_engine
                    .mark_seen(&self.physician_id, item_id, version_id);
            }
            TodayStandingUpdate::Done => {
                if let Some(item) = self
                    .today_candidates
                    .iter()
                    .find(|c| c.item_id == item_id)
                    .cloned()
                {
                    let mut effective = item;
                    effective.version_id = version_id;
                    let _ = self.today_engine.mark_done(&self.physician_id, &effective);
                }
            }
        }
    }

    /// Records a server-issued Connected Care invitation (`EF-XXXXXX`) for `workspace_id`.
    pub fn apply_live_invite_issued(
        &mut self,
        workspace_id: WorkspaceId,
        invite_code: String,
        invitation_id: String,
    ) {
        self.live_invites.insert(
            workspace_id,
            LiveIssuedInvite {
                invitation_id,
                invite_code,
            },
        );
    }

    /// Evaluates the current Today view through `TodayQueueEngine::build_view`.
    #[must_use]
    pub fn build_today_view(&self) -> TodayViewState {
        self.today_engine.build_view(
            &self.physician_id,
            &self.today_candidates,
            self.scheduled_visits.clone(),
            self.today_also_reviewed,
            Vec::new(),
        )
    }

    /// Returns urgency-sorted (`Now` before `Today`) visible Today items.
    #[must_use]
    pub fn visible_today_items(&self) -> Vec<RenderedTodayItem> {
        if let Some(ref live) = self.live_today {
            return live.items.clone();
        }
        match self.build_today_view() {
            TodayViewState::Actionable { items, .. } => items,
            TodayViewState::Quiet { .. } => Vec::new(),
        }
    }

    /// Returns scheduled visits from the live snapshot when present, or the local demo engine.
    #[must_use]
    pub fn scheduled_visits_list(&self) -> Vec<ScheduledVisitEntry> {
        if let Some(ref live) = self.live_today {
            return live.scheduled_visits.clone();
        }
        match self.build_today_view() {
            TodayViewState::Actionable {
                scheduled_visits, ..
            }
            | TodayViewState::Quiet {
                scheduled_visits, ..
            } => scheduled_visits,
        }
    }

    /// Marks the `index`-th visible Today item as Seen at its current `version_id`.
    pub fn mark_today_seen(&mut self, index: usize) -> Option<String> {
        let items = self.visible_today_items();
        let rendered = items.get(index)?;
        self.today_engine.mark_seen(
            &self.physician_id,
            &rendered.item.item_id,
            rendered.item.version_id,
        );
        Some(rendered.item.item_id.clone())
    }

    /// Marks the `index`-th visible Today item as Done at its current `version_id`.
    pub fn mark_today_done(&mut self, index: usize) -> Option<String> {
        let items = self.visible_today_items();
        let rendered = items.get(index)?;
        if self
            .today_engine
            .mark_done(&self.physician_id, &rendered.item)
        {
            Some(rendered.item.item_id.clone())
        } else {
            None
        }
    }

    /// Returns the workspace ID for the `index`-th sorted scheduled visit.
    #[must_use]
    pub fn scheduled_visit_workspace(&self, index: usize) -> Option<WorkspaceId> {
        self.scheduled_visits_list()
            .get(index)
            .map(|v| v.workspace_id.clone())
    }

    /// Derives the physician-scoped reply card through `MessageReplyEngine::derive_reply_card`.
    pub fn derive_active_reply_card(&self) -> Result<DerivedReplyCard, ReplyError> {
        self.reply_engine.derive_reply_card(
            &self.physician_id,
            &self.active_conversation_id,
            |_, _| true,
        )
    }

    /// Saves a physician-scoped personal draft edit (`YourDraft`) on the local demo engine.
    pub fn edit_active_personal_draft(&mut self, draft_text: String) -> Result<(), ReplyError> {
        self.reply_engine.edit_personal_draft(
            &self.physician_id,
            &self.active_conversation_id,
            draft_text,
            |_, _| true,
        )
    }

    /// Discards the physician's personal draft and restores the underlying system AI reply.
    pub fn restore_active_system_reply(&mut self) -> Result<(), ReplyError> {
        self.reply_engine.restore_system_reply(
            &self.physician_id,
            &self.active_conversation_id,
            |_, _| true,
        )
    }

    /// Copies the exact currently visible reply text (`CopiesWhatYouSee`) with zero mutation.
    pub fn copy_active_reply(&self) -> Result<String, ReplyError> {
        self.reply_engine
            .copy_reply(&self.physician_id, &self.active_conversation_id, |_, _| {
                true
            })
    }

    /// Closes the active reply (`ConcernStays` + `UndoBringsBack` snapshot).
    pub fn close_active_reply(&mut self) -> Result<(), ReplyError> {
        self.reply_engine
            .close_reply(&self.physician_id, &self.active_conversation_id, |_, _| {
                true
            })
    }

    /// Restores the closed reply and reopens its patient messages (`UndoBringsBack`).
    pub fn undo_close_active_reply(&mut self) -> Result<(), ReplyError> {
        self.reply_engine.undo_close_reply(
            &self.physician_id,
            &self.active_conversation_id,
            |_, _| true,
        )
    }

    /// Returns all active and retired memory entries for `workspace_id`.
    #[must_use]
    pub fn memory_entries_for(&self, workspace_id: &WorkspaceId) -> &[MemoryEntry] {
        if let Some(live) = self.live_memories.get(workspace_id) {
            return live.as_slice();
        }
        self.memories
            .get(workspace_id)
            .map_or(&[], PatientMemoryStore::all_entries)
    }

    /// Returns the Connected Care state for `workspace_id` via `ConnectedCareBridge::clinic_standing`.
    #[must_use]
    pub fn connected_care_state_for(&self, workspace_id: &WorkspaceId) -> ConnectedCareState {
        if self.live_invites.contains_key(workspace_id) {
            return ConnectedCareState::Invited;
        }
        let Ok(patient_id) = PatientId::new(workspace_id.as_str()) else {
            return ConnectedCareState::NotInvited;
        };
        let pair = PairKey::new(self.practice_id.clone(), patient_id);
        self.connected_care.clinic_standing(&pair).state
    }

    /// Returns the currently active `IssuedInvitation` for `workspace_id`, if any.
    #[must_use]
    pub fn active_invite_for(&self, workspace_id: &WorkspaceId) -> Option<&IssuedInvitation> {
        self.active_invites.get(workspace_id)
    }

    /// Returns the active `EF-XXXXXX` invite code from either live server state or local demo bridge.
    #[must_use]
    pub fn active_invite_code_for(&self, workspace_id: &WorkspaceId) -> Option<&str> {
        if let Some(live) = self.live_invites.get(workspace_id) {
            return Some(live.invite_code.as_str());
        }
        self.active_invites
            .get(workspace_id)
            .map(|inv| inv.short_code.as_str())
    }

    /// Issues (or returns existing open) 6-digit `EF-XXXXXX` Connected Care invitation
    /// through `ConnectedCareBridge::create_invitation` (`AtMostOneOpen`).
    pub fn issue_connected_care_invite(
        &mut self,
        workspace_id: &WorkspaceId,
    ) -> Result<IssuedInvitation, BridgeError> {
        if let Some(existing) = self.active_invites.get(workspace_id)
            && self.connected_care_state_for(workspace_id) == ConnectedCareState::Invited
        {
            return Ok(existing.clone());
        }

        let Ok(patient_id) = PatientId::new(workspace_id.as_str()) else {
            return Err(BridgeError::EmptyField {
                field: "practice_patient_id",
            });
        };
        let pair = PairKey::new(self.practice_id.clone(), patient_id);
        let expected_dob = DateOfBirth::new(1985, 4, 12)?;
        let issued = self
            .connected_care
            .create_invitation(CreateInvitationRequest {
                pair,
                issued_by: self.physician_id.clone(),
                practice_name: "Curie Clinic Desktop Practice".to_owned(),
                physician_name: "Dr. Curie Synthetic".to_owned(),
                expected_dob,
                created_at: "2026-10-18T15:00:00Z".to_owned(),
            })?;
        self.active_invites
            .insert(workspace_id.clone(), issued.clone());
        Ok(issued)
    }

    /// Formats the `TodaySaysSo` channel health banner text (`TempError` never asks to reconnect).
    #[must_use]
    pub fn format_channel_health_banner(&self, locale: Locale) -> String {
        let banners = if let Some(ref live) = self.live_today {
            live.channel_banners.clone()
        } else {
            match self.build_today_view() {
                TodayViewState::Actionable {
                    channel_banners, ..
                }
                | TodayViewState::Quiet {
                    channel_banners, ..
                } => channel_banners,
            }
        };
        let heading = translate(locale, UiKey::ChannelHealth);
        if banners.is_empty() {
            return format!("{heading}: OK");
        }
        let parts: Vec<String> = banners
            .iter()
            .filter_map(|b| match b {
                TodayChannelBanner::None => None,
                TodayChannelBanner::TemporaryTrouble {
                    channel_id, detail, ..
                } => Some(format!("{channel_id}: {detail}")),
                TodayChannelBanner::ReconnectRequired { channel_id, detail } => {
                    Some(format!("{channel_id} (reconnect required): {detail}"))
                }
                TodayChannelBanner::Disconnected { channel_id } => {
                    Some(format!("{channel_id}: disconnected"))
                }
            })
            .collect();
        format!("{heading}: {}", parts.join(" | "))
    }

    /// Formats the active patient's longitudinal memory entries distinguishing `ChartRecord` vs
    /// `PhysicianInstruction` and preserving disagreements and retired history honestly.
    #[must_use]
    pub fn format_memory_entries(&self, workspace_id: &WorkspaceId, locale: Locale) -> String {
        let entries = self.memory_entries_for(workspace_id);
        if entries.is_empty() {
            return translate(locale, UiKey::ClinicTranscriptEmpty).to_owned();
        }
        let chart_badge = translate(locale, UiKey::ChartRecordBadge);
        let phys_badge = translate(locale, UiKey::PhysicianInstructionBadge);

        entries
            .iter()
            .map(|entry| {
                let auth = match entry.authority {
                    MemoryAuthority::ChartRecord => chart_badge,
                    MemoryAuthority::PhysicianInstruction => phys_badge,
                };
                match &entry.state {
                    MemoryEntryState::Active => format!(
                        "[ACTIVE | {auth} | {}] ({}) {}",
                        entry.observed_at, entry.topic, entry.statement
                    ),
                    MemoryEntryState::Retired {
                        resolved_at,
                        reason,
                    } => format!(
                        "[RETIRED {resolved_at} | {auth} | {}] ({}) {} -- {reason}",
                        entry.observed_at, entry.topic, entry.statement
                    ),
                }
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Returns `(has_ready_reply, is_personal_draft, can_undo_close)` for `workspace_id`,
    /// preferring the authoritative live server card when connected.
    #[must_use]
    pub fn reply_card_action_flags(
        &self,
        workspace_id: Option<&WorkspaceId>,
    ) -> (bool, bool, bool) {
        if let Some(card) = self.active_live_reply_card(workspace_id) {
            return (
                card.visible_reply_text.is_some(),
                card.visible_reply_origin == LiveReplyOrigin::PersonalDraft,
                card.undo_available,
            );
        }
        if let Ok(card) = self.derive_active_reply_card() {
            return (
                card.visible_reply_text.is_some(),
                card.visible_reply_origin == VisibleReplyOrigin::PersonalDraft,
                card.undo_available,
            );
        }
        (false, false, false)
    }

    /// Formats the derived Messages reply card text (`SystemReply` vs `PersonalDraft` + citations + concern)
    /// for `workspace_id`, preferring live server state when present.
    #[must_use]
    pub fn format_messages_card_for(
        &self,
        workspace_id: Option<&WorkspaceId>,
        locale: Locale,
    ) -> String {
        if let Some(card) = self.active_live_reply_card(workspace_id) {
            return format_reply_card_parts(
                &card.open_messages,
                card.visible_reply_text.as_deref(),
                card.visible_reply_origin.to_visible_origin(),
                &card.reply_state_label,
                &card.basis_citations,
                card.flagged_concern.as_deref(),
                locale,
            );
        }
        self.format_messages_card(locale)
    }

    /// Formats the derived Messages reply card text (`SystemReply` vs `PersonalDraft` + citations + concern).
    #[must_use]
    pub fn format_messages_card(&self, locale: Locale) -> String {
        let Ok(card) = self.derive_active_reply_card() else {
            return translate(locale, UiKey::ClinicTranscriptEmpty).to_owned();
        };
        format_reply_card_parts(
            &card.open_messages,
            card.visible_reply_text.as_deref(),
            card.visible_reply_origin,
            card.reply_state_label,
            &card.basis_citations,
            card.flagged_concern.as_deref(),
            locale,
        )
    }
}

fn format_reply_card_parts(
    open_messages: &[ef_clinic_workflows::replies::TrackedPatientMessage],
    visible_reply_text: Option<&str>,
    visible_reply_origin: VisibleReplyOrigin,
    reply_state_label: &str,
    basis_citations: &[SourceKey],
    flagged_concern: Option<&str>,
    locale: Locale,
) -> String {
    let mut lines = Vec::new();
    if open_messages.is_empty() {
        lines.push("Open patient messages: 0".to_owned());
    } else {
        for msg in open_messages {
            lines.push(format!("[Patient | {}] {}", msg.message_id, msg.body));
        }
    }
    let origin_label = match visible_reply_origin {
        VisibleReplyOrigin::SystemReply => translate(locale, UiKey::MessagesSystemDraftBadge),
        VisibleReplyOrigin::PersonalDraft => translate(locale, UiKey::MessagesPersonalDraftBadge),
        VisibleReplyOrigin::None => "-",
    };
    if let Some(reply_text) = visible_reply_text {
        lines.push(format!(
            "[Draft | {origin_label} | {reply_state_label}] {reply_text}"
        ));
    } else {
        lines.push(format!("[Reply State: {reply_state_label}]"));
    }
    if !basis_citations.is_empty() {
        let cites: Vec<&str> = basis_citations.iter().map(SourceKey::as_str).collect();
        lines.push(format!("Citations: {}", cites.join(", ")));
    }
    if let Some(concern) = flagged_concern {
        lines.push(format!("Flagged concern: {concern}"));
    }
    lines.join("\n\n")
}
