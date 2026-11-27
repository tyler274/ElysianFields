//! Client presentation state, multi-tab session coordinator, virtualized transcript/citation
//! projections, and single-owner composer draft synchronization.
//!
//! Holds NO second authoritative clinical store; authoritative data lives strictly behind
//! [`crate::transport::ClinicTransport`].

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use bevy::prelude::Resource;
use ef_clinic_domain::auth::{
    CrossTabSessionNotice, DeepLinkTarget, TabViewState, WorkspaceOpenView,
};
use ef_clinic_domain::chat::{
    AnswerState, ChatStreamEvent, ComposerDraftController, DraftSyncState, ExchangeRecord,
    ExchangeSteps, PersistedDraft, ResumeHandle,
};
use ef_clinic_domain::workspace::{
    CitationChipRef, CitationPanelController, CitationPanelState, MembershipTier,
    PatientRosterState, PatientWorkspaceSummary,
};
use ef_core::{ChatId, RefusalCode, SessionId, TabId, WorkspaceId};
use ef_ui_catalog::{
    ColumnAlignment, HorizontalEdge, Locale as UiLocaleKind, TableColumn, UiKey,
    VirtualizedListView, VirtualizedTableView, translate,
};

use crate::transport::{ClinicTransport, TransportError, stream_event_id};
use crate::wire::ClientBootConfig;

/// Total exhaustive conversion from `ef_core::Locale` to `ef_ui_catalog::Locale`.
#[must_use]
pub const fn core_to_ui_locale(locale: ef_core::Locale) -> UiLocaleKind {
    match locale {
        ef_core::Locale::En => UiLocaleKind::En,
        ef_core::Locale::Es => UiLocaleKind::Es,
        ef_core::Locale::Fr => UiLocaleKind::Fr,
        ef_core::Locale::Pt => UiLocaleKind::Pt,
        ef_core::Locale::De => UiLocaleKind::De,
        ef_core::Locale::ZhHans => UiLocaleKind::ZhHans,
        ef_core::Locale::Ja => UiLocaleKind::Ja,
        ef_core::Locale::Ko => UiLocaleKind::Ko,
        ef_core::Locale::Ar => UiLocaleKind::Ar,
    }
}

/// Exhaustive conversion from `ef_ui_catalog::Locale` to `ef_core::Locale` when represented
/// in the core clinical wire locale set (`None` for UI-extended locales such as `he`, `vi`, `hi`).
#[must_use]
pub const fn ui_to_core_locale(locale: UiLocaleKind) -> Option<ef_core::Locale> {
    match locale {
        UiLocaleKind::En => Some(ef_core::Locale::En),
        UiLocaleKind::Es => Some(ef_core::Locale::Es),
        UiLocaleKind::Fr => Some(ef_core::Locale::Fr),
        UiLocaleKind::Pt => Some(ef_core::Locale::Pt),
        UiLocaleKind::De => Some(ef_core::Locale::De),
        UiLocaleKind::ZhHans => Some(ef_core::Locale::ZhHans),
        UiLocaleKind::Ja => Some(ef_core::Locale::Ja),
        UiLocaleKind::Ko => Some(ef_core::Locale::Ko),
        UiLocaleKind::Ar => Some(ef_core::Locale::Ar),
        UiLocaleKind::He | UiLocaleKind::Vi | UiLocaleKind::Hi => None,
    }
}

/// Deterministic client clock for headless testing and live-announcement timestamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Resource)]
pub struct ClientClock {
    pub now_secs: u64,
    pub now_ms: u64,
}

impl Default for ClientClock {
    fn default() -> Self {
        Self {
            now_secs: 1_000,
            now_ms: 0,
        }
    }
}

impl ClientClock {
    pub fn advance_secs(&mut self, delta_secs: u64) {
        self.now_secs = self.now_secs.saturating_add(delta_secs);
        self.now_ms = self.now_ms.saturating_add(delta_secs.saturating_mul(1_000));
    }

    pub fn advance_ms(&mut self, delta_ms: u64) {
        self.now_ms = self.now_ms.saturating_add(delta_ms);
        self.now_secs = self.now_secs.saturating_add(delta_ms / 1_000);
    }
}

/// Shared origin-scoped storage persisting `curie.locale.v1` across simulated reloads.
#[derive(Debug, Clone, Default, Resource)]
pub struct SharedLocaleStorage {
    stored_bcp47: Arc<Mutex<Option<String>>>,
}

impl SharedLocaleStorage {
    #[must_use]
    pub fn new(initial: Option<&str>) -> Self {
        Self {
            stored_bcp47: Arc::new(Mutex::new(initial.map(str::to_owned))),
        }
    }

    #[must_use]
    pub fn read(&self) -> Option<String> {
        self.stored_bcp47.lock().ok().and_then(|g| g.clone())
    }

    pub fn write(&self, locale: UiLocaleKind) {
        if let Ok(mut guard) = self.stored_bcp47.lock() {
            *guard = Some(locale.bcp47_tag().to_owned());
        }
    }
}

/// Boot configuration and resolved layout edges stored as a Bevy resource.
#[derive(Debug, Clone, Resource)]
pub struct ClientRuntimeConfig {
    pub boot: ClientBootConfig,
    pub os_preferred_languages: Vec<String>,
    pub sidebar_edge: HorizontalEdge,
    pub citation_panel_edge: HorizontalEdge,
}

impl ClientRuntimeConfig {
    #[must_use]
    pub fn new(boot: ClientBootConfig, os_preferred_languages: Vec<String>) -> Self {
        Self {
            boot,
            os_preferred_languages,
            sidebar_edge: HorizontalEdge::Left,
            citation_panel_edge: HorizontalEdge::Right,
        }
    }
}

/// Shared origin-scoped browser storage + `BroadcastChannel` coordinator shared across
/// multiple Bevy client tabs in the same browser profile.
#[derive(Debug, Default)]
pub struct BrowserSessionHubInner {
    pub shared_storage: Option<SessionId>,
    pub tabs: BTreeMap<TabId, TabViewState>,
    pub pending_notices: VecDeque<CrossTabSessionNotice>,
    pub ever_signed_out_sessions: Vec<SessionId>,
}

/// Thread-safe handle to the shared multi-tab browser session coordinator.
#[derive(Debug, Clone, Default, Resource)]
pub struct SharedBrowserSessionHub {
    inner: Arc<Mutex<BrowserSessionHubInner>>,
}

impl SharedBrowserSessionHub {
    #[must_use]
    pub fn new(initial_storage: Option<SessionId>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(BrowserSessionHubInner {
                shared_storage: initial_storage,
                tabs: BTreeMap::new(),
                pending_notices: VecDeque::new(),
                ever_signed_out_sessions: Vec::new(),
            })),
        }
    }

    /// Registers a tab in `Opening` preserving its `?patient=` deep-link target.
    pub fn register_tab(&self, tab_id: TabId, wants: DeepLinkTarget) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.tabs.insert(tab_id, TabViewState::Opening { wants });
        }
    }

    /// Reads the current view state for `tab_id`.
    #[must_use]
    pub fn tab_state(&self, tab_id: &TabId) -> Option<TabViewState> {
        self.inner
            .lock()
            .ok()
            .and_then(|guard| guard.tabs.get(tab_id).cloned())
    }

    /// Reads the current shared browser storage session.
    #[must_use]
    pub fn shared_storage(&self) -> Option<SessionId> {
        self.inner
            .lock()
            .ok()
            .and_then(|guard| guard.shared_storage.clone())
    }

    /// Resolves `Opening` state against `transport.whoami`, preserving deep-link targets (`Lands`).
    pub fn resolve_opening(
        &self,
        tab_id: &TabId,
        transport: &mut dyn ClinicTransport,
        now_secs: u64,
    ) -> Option<TabViewState> {
        let (wants, stored) = {
            let guard = self.inner.lock().ok()?;
            let Some(TabViewState::Opening { wants }) = guard.tabs.get(tab_id).cloned() else {
                return guard.tabs.get(tab_id).cloned();
            };
            (wants, guard.shared_storage.clone())
        };

        let next = match stored {
            None => TabViewState::SignInPage { wants },
            Some(token) => match transport.whoami(Some(&token), now_secs) {
                Ok(_) => {
                    let opens = match wants {
                        DeepLinkTarget::Today => WorkspaceOpenView::Today,
                        DeepLinkTarget::LinkedPatient(ws) => WorkspaceOpenView::LinkedPatient(ws),
                    };
                    TabViewState::Workspace {
                        holds: token,
                        opens,
                        sse_connected: true,
                    }
                }
                Err(_) => {
                    if let Ok(mut guard) = self.inner.lock()
                        && guard.shared_storage.as_ref() == Some(&token)
                    {
                        guard.shared_storage = None;
                    }
                    TabViewState::SignInPage { wants }
                }
            },
        };

        if let Ok(mut guard) = self.inner.lock() {
            guard.tabs.insert(tab_id.clone(), next.clone());
        }
        Some(next)
    }

    /// Marks a tab as verifying an emailed sign-in link (`TodayFirst`: resets `wants` to `Today`).
    pub fn begin_email_sign_in_link(&self, tab_id: &TabId) {
        if let Ok(mut guard) = self.inner.lock()
            && let Some(state) = guard.tabs.get_mut(tab_id)
        {
            let prior_held = state.held_session().cloned();
            *state = TabViewState::SignInLink { prior_held };
        }
    }

    /// Completes sign-in in `tab_id`, updates shared storage, and queues cross-tab broadcast notice.
    pub fn complete_sign_in(&self, tab_id: &TabId, new_session: SessionId) -> Option<TabViewState> {
        let mut guard = self.inner.lock().ok()?;
        guard.shared_storage = Some(new_session.clone());
        let next = {
            let current = guard.tabs.get(tab_id);
            let opens = match current {
                Some(
                    TabViewState::Opening {
                        wants: DeepLinkTarget::LinkedPatient(ws),
                    }
                    | TabViewState::SignInPage {
                        wants: DeepLinkTarget::LinkedPatient(ws),
                    },
                ) => WorkspaceOpenView::LinkedPatient(ws.clone()),
                _ => WorkspaceOpenView::Today,
            };
            TabViewState::Workspace {
                holds: new_session.clone(),
                opens,
                sse_connected: true,
            }
        };
        guard.tabs.insert(tab_id.clone(), next.clone());
        guard.pending_notices.push_back(CrossTabSessionNotice {
            origin_tab: tab_id.clone(),
            stored_session: Some(new_session),
        });
        Some(next)
    }

    /// Handles an ended-session refusal (`401 session_ended` / `403 access_removed`) while
    /// enforcing `Expiry`, `PromptIsEmpty`, `KeepsWork`, and `TodayFirst`.
    pub fn handle_session_ended_response(
        &self,
        tab_id: &TabId,
        failed_session: &SessionId,
        transport: &mut dyn ClinicTransport,
        now_secs: u64,
    ) -> Option<TabViewState> {
        let candidate_stored = {
            let guard = self.inner.lock().ok()?;
            guard.shared_storage.clone()
        };

        // KeepsWork: if another tab already established a newer live session in shared storage,
        // adopt that session cleanly rather than kicking this tab back to sign-in.
        if let Some(stored) = candidate_stored
            && &stored != failed_session
            && transport.whoami(Some(&stored), now_secs).is_ok()
        {
            let mut guard = self.inner.lock().ok()?;
            let next = {
                let current = guard.tabs.get(tab_id);
                let opens = match current {
                    Some(
                        TabViewState::Opening {
                            wants: DeepLinkTarget::LinkedPatient(ws),
                        }
                        | TabViewState::SignInPage {
                            wants: DeepLinkTarget::LinkedPatient(ws),
                        },
                    ) => WorkspaceOpenView::LinkedPatient(ws.clone()),
                    _ => WorkspaceOpenView::Today,
                };
                TabViewState::Workspace {
                    holds: stored,
                    opens,
                    sse_connected: true,
                }
            };
            guard.tabs.insert(tab_id.clone(), next.clone());
            return Some(next);
        }

        let mut guard = self.inner.lock().ok()?;
        if guard.shared_storage.as_ref() == Some(failed_session) {
            guard.shared_storage = None;
            guard.pending_notices.push_back(CrossTabSessionNotice {
                origin_tab: tab_id.clone(),
                stored_session: None,
            });
        }
        let next = match guard.tabs.get(tab_id).cloned() {
            Some(TabViewState::Workspace { .. } | TabViewState::SessionEndedPrompt) => {
                TabViewState::SessionEndedPrompt
            }
            Some(TabViewState::Opening { wants } | TabViewState::SignInPage { wants }) => {
                TabViewState::SignInPage { wants }
            }
            Some(TabViewState::SignInLink { .. }) | None => TabViewState::SignInPage {
                wants: DeepLinkTarget::Today,
            },
        };
        guard.tabs.insert(tab_id.clone(), next.clone());
        Some(next)
    }

    /// Executes explicit sign-out (`OnceOut` + `ServerRefusesSignedOut`), terminating BOTH
    /// shared stored session and this tab's held session on the server via `transport.sign_out`.
    pub fn sign_out(
        &self,
        tab_id: &TabId,
        transport: &mut dyn ClinicTransport,
        now_secs: u64,
    ) -> Result<(), TransportError> {
        let (stored, held) = {
            let mut guard = self
                .inner
                .lock()
                .map_err(|err| TransportError::Unavailable {
                    message: format!("session hub lock poisoned: {err}"),
                })?;
            let stored = guard.shared_storage.take();
            let held = guard
                .tabs
                .get(tab_id)
                .and_then(TabViewState::held_session)
                .cloned();
            (stored, held)
        };

        if let Some(ref token) = stored {
            transport.sign_out(token, now_secs)?;
        }
        if let Some(ref token) = held
            && stored.as_ref() != Some(token)
        {
            transport.sign_out(token, now_secs)?;
        }

        if let Ok(mut guard) = self.inner.lock() {
            if let Some(token) = stored {
                guard.ever_signed_out_sessions.push(token);
            }
            if let Some(token) = held
                && !guard.ever_signed_out_sessions.contains(&token)
            {
                guard.ever_signed_out_sessions.push(token);
            }
            guard.tabs.insert(
                tab_id.clone(),
                TabViewState::SignInPage {
                    wants: DeepLinkTarget::Today,
                },
            );
            guard.pending_notices.push_back(CrossTabSessionNotice {
                origin_tab: tab_id.clone(),
                stored_session: None,
            });
        }
        Ok(())
    }

    /// Reconciles `tab_id` against current shared storage and server liveness (`CatchesUp`, `Lands`, `OnceOut`).
    pub fn reconcile_tab(
        &self,
        tab_id: &TabId,
        transport: &mut dyn ClinicTransport,
        now_secs: u64,
    ) -> Option<TabViewState> {
        let (current, stored) = {
            let guard = self.inner.lock().ok()?;
            (
                guard.tabs.get(tab_id).cloned()?,
                guard.shared_storage.clone(),
            )
        };

        let next = match (current, stored) {
            (TabViewState::Opening { wants } | TabViewState::SignInPage { wants }, Some(token)) => {
                if transport.whoami(Some(&token), now_secs).is_ok() {
                    let opens = match wants {
                        DeepLinkTarget::Today => WorkspaceOpenView::Today,
                        DeepLinkTarget::LinkedPatient(ws) => WorkspaceOpenView::LinkedPatient(ws),
                    };
                    TabViewState::Workspace {
                        holds: token,
                        opens,
                        sse_connected: true,
                    }
                } else {
                    TabViewState::SignInPage { wants }
                }
            }
            (TabViewState::SessionEndedPrompt, Some(token)) => {
                if transport.whoami(Some(&token), now_secs).is_ok() {
                    TabViewState::Workspace {
                        holds: token,
                        opens: WorkspaceOpenView::Today,
                        sse_connected: true,
                    }
                } else {
                    TabViewState::SessionEndedPrompt
                }
            }
            (TabViewState::Workspace { holds, opens, .. }, Some(token)) => {
                if holds == token && transport.whoami(Some(&holds), now_secs).is_ok() {
                    TabViewState::Workspace {
                        holds,
                        opens,
                        sse_connected: true,
                    }
                } else if transport.whoami(Some(&token), now_secs).is_ok() {
                    TabViewState::Workspace {
                        holds: token,
                        opens: WorkspaceOpenView::Today,
                        sse_connected: true,
                    }
                } else {
                    TabViewState::SessionEndedPrompt
                }
            }
            (TabViewState::Workspace { .. }, None) => TabViewState::SignInPage {
                wants: DeepLinkTarget::Today,
            },
            (TabViewState::Opening { wants }, None) => TabViewState::SignInPage { wants },
            (other, _) => other,
        };

        if let Ok(mut guard) = self.inner.lock() {
            guard.tabs.insert(tab_id.clone(), next.clone());
        }
        Some(next)
    }

    /// Updates a workspace tab's currently opened patient view (`Today` or `LinkedPatient`).
    pub fn set_open_workspace(&self, tab_id: &TabId, opens: WorkspaceOpenView) {
        if let Ok(mut guard) = self.inner.lock()
            && let Some(TabViewState::Workspace {
                opens: current_opens,
                ..
            }) = guard.tabs.get_mut(tab_id)
        {
            *current_opens = opens;
        }
    }

    /// Checks `ActsAsShown`: returns the tab's held session ONLY if in `Workspace` and matching shared storage.
    #[must_use]
    pub fn active_session_for(&self, tab_id: &TabId) -> Option<SessionId> {
        let guard = self.inner.lock().ok()?;
        let Some(TabViewState::Workspace { holds, .. }) = guard.tabs.get(tab_id) else {
            return None;
        };
        if guard.shared_storage.as_ref() == Some(holds) {
            Some(holds.clone())
        } else {
            None
        }
    }

    /// Verifies post-convergence sign-in safety invariants across all registered tabs.
    pub fn check_safety_invariants(
        &self,
        transport: &mut dyn ClinicTransport,
        now_secs: u64,
    ) -> Result<(), String> {
        let (dead_sessions, stored, tabs) = {
            let guard = self
                .inner
                .lock()
                .map_err(|err| format!("session hub lock poisoned: {err}"))?;
            (
                guard.ever_signed_out_sessions.clone(),
                guard.shared_storage.clone(),
                guard.tabs.clone(),
            )
        };

        for dead in &dead_sessions {
            if transport.whoami(Some(dead), now_secs).is_ok() {
                return Err(format!(
                    "ServerRefusesSignedOut violated: session {} still accepted",
                    dead.as_str()
                ));
            }
        }

        let live_stored = stored
            .as_ref()
            .filter(|s| transport.whoami(Some(s), now_secs).is_ok());

        for (tab_id, state) in &tabs {
            match state {
                TabViewState::Workspace { holds, .. } => {
                    if live_stored.is_none() {
                        return Err(format!(
                            "OnceOut violated: tab {} in Workspace without live stored session",
                            tab_id.as_str()
                        ));
                    }
                    if transport.whoami(Some(holds), now_secs).is_err() {
                        return Err(format!(
                            "SignedIn violated: tab {} holds dead session {}",
                            tab_id.as_str(),
                            holds.as_str()
                        ));
                    }
                    if stored.as_ref() != Some(holds) {
                        return Err(format!(
                            "ActsAsShown violated: tab {} holds {} != stored {:?}",
                            tab_id.as_str(),
                            holds.as_str(),
                            stored
                        ));
                    }
                }
                TabViewState::SessionEndedPrompt => {
                    if state.shows_protected_content() || state.held_session().is_some() {
                        return Err("PromptIsEmpty violated".to_owned());
                    }
                    if live_stored.is_some() {
                        return Err(format!(
                            "SignedIn violated: tab {} stuck on SessionEndedPrompt with live storage",
                            tab_id.as_str()
                        ));
                    }
                }
                TabViewState::SignInPage { .. } => {
                    if live_stored.is_some() {
                        return Err(format!(
                            "SignedIn violated: tab {} stuck on SignInPage with live storage",
                            tab_id.as_str()
                        ));
                    }
                }
                TabViewState::Opening { .. } | TabViewState::SignInLink { .. } => {
                    return Err(format!(
                        "NoDeadlock violated: tab {} stalled in {:?}",
                        tab_id.as_str(),
                        state
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Sign-in form status displayed on `SignInPage` / `SessionEndedPrompt`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SignInFormStatus {
    #[default]
    Idle,
    LinkRequested {
        message: String,
        proof_token: Option<String>,
    },
    Refused {
        code: RefusalCode,
        detail: String,
    },
    MailDeliveryFailed {
        detail: String,
    },
}

/// Per-tab session shell resource coordinating with [`SharedBrowserSessionHub`].
#[derive(Debug, Clone, Resource)]
pub struct ClinicSessionShell {
    pub tab_id: TabId,
    pub view_state: TabViewState,
    pub sign_in_form: SignInFormStatus,
    pub session_banner: Option<String>,
    pub sse_connected: bool,
}

impl ClinicSessionShell {
    #[must_use]
    pub fn new(tab_id: TabId, wants: DeepLinkTarget) -> Self {
        Self {
            tab_id,
            view_state: TabViewState::Opening { wants },
            sign_in_form: SignInFormStatus::Idle,
            session_banner: None,
            sse_connected: false,
        }
    }
}

/// Today & Patient Roster view model with honest Empty / Failed / Loaded states
/// and keyboard `j`/`k`/`Enter` virtualized navigation.
#[derive(Debug, Clone, Resource)]
pub struct TodayRosterView {
    pub state: PatientRosterState,
    pub list: VirtualizedListView,
    pub search_query: String,
    pub finder_open: bool,
}

impl Default for TodayRosterView {
    fn default() -> Self {
        Self {
            state: PatientRosterState::Empty { query: None },
            list: VirtualizedListView::new(0, 480.0, 44.0, 4),
            search_query: String::new(),
            finder_open: false,
        }
    }
}

impl TodayRosterView {
    pub fn apply_roster(&mut self, next: PatientRosterState) {
        let count = match &next {
            PatientRosterState::Loaded { patients, .. } => patients.len(),
            PatientRosterState::Empty { .. } | PatientRosterState::Failed { .. } => 0,
        };
        self.state = next;
        self.list.set_total_rows(count);
    }

    #[must_use]
    pub fn focused_patient(&self) -> Option<&PatientWorkspaceSummary> {
        let idx = self.list.focused_index()?;
        match &self.state {
            PatientRosterState::Loaded { patients, .. } => patients.get(idx),
            PatientRosterState::Empty { .. } | PatientRosterState::Failed { .. } => None,
        }
    }

    pub fn clear_protected(&mut self) {
        self.state = PatientRosterState::Empty { query: None };
        self.list.set_total_rows(0);
        self.search_query.clear();
        self.finder_open = false;
    }
}

/// Banner describing composer draft synchronization status.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DraftBannerState {
    #[default]
    Clean,
    Unsaved,
    Saving,
    Saved {
        revision: u64,
    },
    Conflict {
        server_draft: PersistedDraft,
    },
    NotSaved {
        message: String,
    },
}

/// Original-language provenance notice displayed when viewing clinical text under another UI locale.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum OriginalLanguageNotice {
    #[default]
    Hidden,
    Visible {
        source_locale: ef_core::Locale,
        ui_locale: UiLocaleKind,
        localized_text: &'static str,
    },
}

/// Active patient workspace view model: virtualized incremental streaming transcript,
/// draft autosave controller, and non-modal cited-source panel with 5,000-row lab table virtualization.
#[derive(Debug, Clone, Resource)]
pub struct ActiveWorkspaceView {
    pub open_workspace: Option<OpenPatientWorkspace>,
    pub transcript_list: VirtualizedListView,
    pub lab_table: VirtualizedTableView,
    pub citation_panel: CitationPanelController,
    pub original_language_notice: OriginalLanguageNotice,
    pub draft_controller: ComposerDraftController,
    pub draft_banner: DraftBannerState,
    pub last_event_id: u64,
    pub full_reparse_count: u64,
    pub delta_chunks_applied: u64,
    pub resync_count: u64,
    pub last_workspace_error: Option<String>,
}

/// Open patient workspace details & transcript projection.
#[derive(Debug, Clone)]
pub struct OpenPatientWorkspace {
    pub workspace_id: WorkspaceId,
    pub chat_id: ChatId,
    pub display_name: String,
    pub membership: MembershipTier,
    pub synopsis: Option<String>,
    pub exchanges: Vec<ExchangeRecord>,
}

impl Default for ActiveWorkspaceView {
    fn default() -> Self {
        let columns = vec![
            TableColumn {
                id: "name".to_owned(),
                header_key: UiKey::ClinicLabRecordsHeading,
                width_px: 180.0,
                alignment: ColumnAlignment::Leading,
            },
            TableColumn {
                id: "value".to_owned(),
                header_key: UiKey::ClinicCitedSourcesHeading,
                width_px: 100.0,
                alignment: ColumnAlignment::Trailing,
            },
            TableColumn {
                id: "unit".to_owned(),
                header_key: UiKey::LabelLanguage,
                width_px: 90.0,
                alignment: ColumnAlignment::Leading,
            },
            TableColumn {
                id: "range".to_owned(),
                header_key: UiKey::LabelSettings,
                width_px: 110.0,
                alignment: ColumnAlignment::Leading,
            },
        ];
        Self {
            open_workspace: None,
            transcript_list: VirtualizedListView::new(0, 520.0, 96.0, 4),
            lab_table: VirtualizedTableView::new(columns, 0, 420.0, 28.0, 3),
            citation_panel: CitationPanelController::new(),
            original_language_notice: OriginalLanguageNotice::Hidden,
            draft_controller: ComposerDraftController::from_persisted(PersistedDraft {
                text: String::new(),
                revision: 0,
            }),
            draft_banner: DraftBannerState::Clean,
            last_event_id: 0,
            full_reparse_count: 0,
            delta_chunks_applied: 0,
            resync_count: 0,
            last_workspace_error: None,
        }
    }
}

impl ActiveWorkspaceView {
    /// Immediately clears all protected patient clinical state (enforcing `PromptIsEmpty` and `OnceOut`).
    pub fn clear_protected(&mut self) {
        self.open_workspace = None;
        self.transcript_list.set_total_rows(0);
        self.lab_table.set_total_rows(0);
        self.citation_panel.close();
        self.original_language_notice = OriginalLanguageNotice::Hidden;
        self.draft_controller = ComposerDraftController::from_persisted(PersistedDraft {
            text: String::new(),
            revision: 0,
        });
        self.draft_banner = DraftBannerState::Clean;
        self.last_workspace_error = None;
    }

    /// Applies an incremental SSE stream event to the active exchange without reparsing prior turns.
    ///
    /// Returns `true` if a `ResyncRequired` or terminal state needs a snapshot refresh.
    pub fn apply_stream_event(&mut self, event: &ChatStreamEvent) -> StreamApplyEffect {
        let ev_id = stream_event_id(event);
        if ev_id > self.last_event_id {
            self.last_event_id = ev_id;
        }
        let Some(ws) = self.open_workspace.as_mut() else {
            return StreamApplyEffect::Ignored;
        };

        match event {
            ChatStreamEvent::DeltaChunk {
                work_item_id,
                text_delta,
                ..
            } => {
                if let Some(ex) = ws
                    .exchanges
                    .iter_mut()
                    .find(|ex| &ex.work_item_id == work_item_id)
                {
                    if let AnswerState::Working { writing, .. } = &mut ex.answer {
                        *writing = true;
                    }
                    ex.kept_partial_text.push_str(text_delta);
                    self.delta_chunks_applied = self.delta_chunks_applied.saturating_add(1);
                    return StreamApplyEffect::DeltaAppended {
                        delta: text_delta.clone(),
                    };
                }
                StreamApplyEffect::NeedsSnapshotResync
            }
            ChatStreamEvent::StepRecorded {
                work_item_id, step, ..
            } => {
                if let Some(ex) = ws
                    .exchanges
                    .iter_mut()
                    .find(|ex| &ex.work_item_id == work_item_id)
                {
                    if let ExchangeSteps::Read { steps } = &mut ex.steps {
                        steps.push(step.clone());
                    }
                    return StreamApplyEffect::StepUpdated;
                }
                StreamApplyEffect::NeedsSnapshotResync
            }
            ChatStreamEvent::Cancelled {
                work_item_id,
                expected_run_id,
                ..
            } => {
                if let Some(ex) = ws
                    .exchanges
                    .iter_mut()
                    .find(|ex| &ex.work_item_id == work_item_id)
                {
                    ex.answer = AnswerState::Stopped {
                        resume: Some(ResumeHandle {
                            work_item_id: work_item_id.clone(),
                            expected_run_id: expected_run_id.clone(),
                        }),
                    };
                }
                StreamApplyEffect::StateTransitioned
            }
            ChatStreamEvent::CouldntFinish {
                work_item_id,
                expected_run_id,
                ..
            } => {
                if let Some(ex) = ws
                    .exchanges
                    .iter_mut()
                    .find(|ex| &ex.work_item_id == work_item_id)
                {
                    ex.answer = AnswerState::CouldntFinish {
                        try_again: Some(ResumeHandle {
                            work_item_id: work_item_id.clone(),
                            expected_run_id: expected_run_id.clone(),
                        }),
                    };
                }
                StreamApplyEffect::StateTransitioned
            }
            ChatStreamEvent::Completed { .. } => StreamApplyEffect::NeedsSnapshotResync,
            ChatStreamEvent::ResyncRequired { .. } => {
                self.resync_count = self.resync_count.saturating_add(1);
                StreamApplyEffect::NeedsSnapshotResync
            }
            ChatStreamEvent::KeepAlive => StreamApplyEffect::Ignored,
        }
    }

    /// Recomputes original-language provenance notice and 5,000-row virtualized table row count
    /// for the current citation panel state.
    pub fn sync_citation_panel_projection(&mut self, active_ui_locale: UiLocaleKind) {
        match self.citation_panel.state() {
            CitationPanelState::Loaded { source, .. } => {
                self.lab_table.set_total_rows(source.fields.len());
                if core_to_ui_locale(source.original_language) != active_ui_locale {
                    self.original_language_notice = OriginalLanguageNotice::Visible {
                        source_locale: source.original_language,
                        ui_locale: active_ui_locale,
                        localized_text: translate(
                            active_ui_locale,
                            UiKey::ClinicOriginalLanguageNote,
                        ),
                    };
                } else {
                    self.original_language_notice = OriginalLanguageNotice::Hidden;
                }
            }
            CitationPanelState::Idle
            | CitationPanelState::Loading { .. }
            | CitationPanelState::Unavailable { .. }
            | CitationPanelState::Failed { .. } => {
                self.lab_table.set_total_rows(0);
                self.original_language_notice = OriginalLanguageNotice::Hidden;
            }
        }
    }

    /// Derives [`DraftBannerState`] directly from the single-owner [`ComposerDraftController`].
    pub fn refresh_draft_banner(&mut self) {
        self.draft_banner = match self.draft_controller.sync_state() {
            DraftSyncState::Saving { .. } => DraftBannerState::Saving,
            DraftSyncState::Conflict { server_draft } => DraftBannerState::Conflict {
                server_draft: server_draft.clone(),
            },
            DraftSyncState::NotSaved { message } => DraftBannerState::NotSaved {
                message: message.clone(),
            },
            DraftSyncState::Idle => {
                if self.draft_controller.is_dirty() {
                    DraftBannerState::Unsaved
                } else if self.draft_controller.saved_revision() > 0 {
                    DraftBannerState::Saved {
                        revision: self.draft_controller.saved_revision(),
                    }
                } else {
                    DraftBannerState::Clean
                }
            }
        };
    }

    /// Returns all citation chips present across loaded exchanges.
    #[must_use]
    pub fn available_citations(&self) -> Vec<CitationChipRef> {
        let Some(ws) = &self.open_workspace else {
            return Vec::new();
        };
        let mut chips = Vec::new();
        for ex in &ws.exchanges {
            if let AnswerState::Finished { segments, .. } = &ex.answer {
                for seg in segments {
                    if let ef_clinic_domain::workspace::AnswerSegment::Citation {
                        source_key,
                        number,
                        label,
                        ..
                    } = seg
                    {
                        chips.push(CitationChipRef {
                            source_key: source_key.clone(),
                            number: *number,
                            label: label.clone(),
                        });
                    }
                }
            }
        }
        chips
    }
}

/// Effect of applying one incremental SSE stream event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamApplyEffect {
    DeltaAppended { delta: String },
    StepUpdated,
    StateTransitioned,
    NeedsSnapshotResync,
    Ignored,
}
