//! Bevy `0.20.1` `CurieClinicClientPlugin` built on `EfUiCatalogPlugin`.
//!
//! Wires sign-in & multi-tab session shell, Today & Patient Roster (`j`/`k`/`Enter`),
//! Patient Workspace (virtualized incremental streaming transcript, single-owner IME
//! composer + optimistic-concurrency draft autosave & `409 Conflict` resolution,
//! non-modal Cited-Source side panel with 5,000-row virtualized lab table, Stop/Resume/Try again,
//! global `Cmd/Ctrl+Shift+L` privacy cover, 9-locale runtime switcher with Arabic RTL mirroring,
//! single-owner AccessKit + narrow ARIA bridge, and zero-redraw `IdleRedrawGate`).

use bevy::a11y::AccessibilityNode;
use bevy::prelude::*;
use ef_clinic_domain::auth::{
    AuthError, DeepLinkTarget, SignInInitiationResponse, TabViewState, WorkspaceOpenView,
};
use ef_clinic_domain::chat::{
    AnswerState, CommandEnvelope, CommandPayload, ComposerDraftController, DeliveryMode,
    DraftSaveOutcome, ResumeWorkPayload, StopWorkPayload, SubmitMessagePayload,
};
use ef_clinic_domain::workspace::{
    CitationChipRef, CitationFetchOutcome, CitationPanelState, MembershipTier, PatientRosterState,
    WorkspaceAccessError,
};
use ef_core::{
    ChatId, IdempotencyKey, RefusalCode, RequestId, RunId, SessionId, TabId, WorkItemId,
    WorkspaceId,
};
use ef_ui_catalog::{
    A11yNodeMarker, A11ySemanticKey, ComposerAction, ComposerOutcome, ComposerSurface,
    EditingContext, EfUiCatalogPlugin, FocusId, FrameDecision, IdleRedrawGate,
    Locale as UiLocaleKind, NodeInteractivity, OverlayKind, PoliteLiveQueue, RedrawReason,
    SearchQueryState, ShortcutAction, ShortcutContext, ShortcutInput, ShortcutRouter, UiKey,
    UiLocale, translate,
};

use crate::state::{
    ActiveWorkspaceView, ClientClock, ClientRuntimeConfig, ClinicSessionShell,
    OpenPatientWorkspace, SharedBrowserSessionHub, SharedLocaleStorage, SignInFormStatus,
    StreamApplyEffect, TodayRosterView,
};
use crate::transport::{ClinicTransport, TransportError};
use crate::wire::{ClientBootConfig, PutDraftRequest, SignInRequest, VerifySignInRequest};

/// Resource holding the active typed [`ClinicTransport`].
#[derive(Resource)]
pub struct ClinicTransportResource {
    transport: Box<dyn ClinicTransport>,
}

impl ClinicTransportResource {
    #[must_use]
    pub fn new(transport: impl ClinicTransport + 'static) -> Self {
        Self {
            transport: Box::new(transport),
        }
    }

    pub fn transport_mut(&mut self) -> &mut dyn ClinicTransport {
        self.transport.as_mut()
    }

    #[must_use]
    pub fn transport(&self) -> &dyn ClinicTransport {
        self.transport.as_ref()
    }
}

/// Sign-in verification entry channel enforcing `Lands` vs `TodayFirst`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInEntryMethod {
    DirectForm,
    EmailedLink,
}

/// Explicit physician choice when resolving a `409 Conflict` on composer draft save.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftConflictChoice {
    KeepMine,
    UseNewer,
}

/// Typed client actions dispatched into `CurieClinicClientPlugin`.
#[derive(Debug, Clone, PartialEq)]
pub enum ClinicAction {
    BootTab,
    SubmitSignInEmail {
        email: String,
    },
    VerifySignInToken {
        email: String,
        token: String,
        entry: SignInEntryMethod,
    },
    SignOut,
    HandleSessionEndedRefusal {
        failed_session: SessionId,
    },
    ReconcilePeerTabs,
    LoadPatientRoster {
        query: Option<String>,
    },
    OpenPatientWorkspace {
        workspace_id: WorkspaceId,
        chat_id: Option<ChatId>,
    },
    ComposerEdit(ComposerAction),
    AutosaveDraft,
    ResolveDraftConflict(DraftConflictChoice),
    SubmitMessage {
        idempotency_key: IdempotencyKey,
        delivery: Option<DeliveryMode>,
    },
    StopActiveWork {
        idempotency_key: IdempotencyKey,
    },
    ResumeOrRetryWork {
        idempotency_key: IdempotencyKey,
        work_item_id: WorkItemId,
        expected_run_id: RunId,
    },
    PollStream,
    OpenCitationChip(CitationChipRef),
    DeliverCitationResponse {
        request_id: RequestId,
        outcome: CitationFetchOutcome,
    },
    CloseCitationPanel,
    ScrollLabTable {
        offset_px: f32,
    },
    ScrollTranscript {
        offset_px: f32,
    },
    DispatchShortcut(ShortcutInput),
    SwitchLocale(UiLocaleKind),
}

/// ECS message delivering one typed [`ClinicAction`] to the client update loop.
#[derive(Message, Debug, Clone)]
pub struct ClinicActionMessage(pub ClinicAction);

/// Marker component tagging managed Bevy UI + AccessKit entities rebuilt on view changes.
#[derive(Component, Debug)]
pub struct ManagedClinicA11yNode;

/// Tracks the last `IdleRedrawGate` frame decision for deterministic inspection in tests.
#[derive(Resource, Debug, Clone, PartialEq, Eq, Default)]
pub struct ClientFrameReport {
    pub last_decision: Option<FrameDecision>,
    pub a11y_tree_generation: u64,
    pub last_rendered_a11y_generation: u64,
}

/// Complete Bevy `0.20.1` plugin for the Curie Clinic M2 vertical slice.
pub struct CurieClinicClientPlugin {
    pub tab_id: TabId,
    pub boot_config: ClientBootConfig,
    pub os_preferred_languages: Vec<String>,
    pub session_hub: SharedBrowserSessionHub,
    pub locale_storage: SharedLocaleStorage,
}

impl CurieClinicClientPlugin {
    #[must_use]
    pub fn new(
        tab_id: TabId,
        boot_config: ClientBootConfig,
        session_hub: SharedBrowserSessionHub,
        locale_storage: SharedLocaleStorage,
    ) -> Self {
        Self {
            tab_id,
            boot_config,
            os_preferred_languages: vec!["en-US".to_owned()],
            session_hub,
            locale_storage,
        }
    }
}

impl Plugin for CurieClinicClientPlugin {
    fn build(&self, app: &mut App) {
        let persisted_pref = self.locale_storage.read();
        let os_refs: Vec<&str> = self
            .os_preferred_languages
            .iter()
            .map(String::as_str)
            .collect();
        let resolved_locale = UiLocaleKind::resolve(
            self.boot_config.query_lang.as_deref(),
            persisted_pref.as_deref(),
            &os_refs,
        );

        self.session_hub
            .register_tab(self.tab_id.clone(), self.boot_config.deep_link.clone());

        let mut runtime_cfg = ClientRuntimeConfig::new(
            self.boot_config.clone(),
            self.os_preferred_languages.clone(),
        );
        runtime_cfg.sidebar_edge = resolved_locale.sidebar_edge();
        runtime_cfg.citation_panel_edge = resolved_locale.citation_panel_edge();

        app.add_plugins(EfUiCatalogPlugin)
            .insert_resource(UiLocale::new(resolved_locale))
            .insert_resource(runtime_cfg)
            .insert_resource(self.session_hub.clone())
            .insert_resource(self.locale_storage.clone())
            .insert_resource(ClinicSessionShell::new(
                self.tab_id.clone(),
                self.boot_config.deep_link.clone(),
            ))
            .init_resource::<ClientClock>()
            .init_resource::<TodayRosterView>()
            .init_resource::<ActiveWorkspaceView>()
            .init_resource::<ClientFrameReport>()
            .add_message::<ClinicActionMessage>()
            .add_systems(
                Update,
                (
                    process_clinic_actions_system,
                    sync_clinic_a11y_entities_system,
                    consume_idle_redraw_gate_system,
                )
                    .chain(),
            );
    }
}

#[allow(clippy::too_many_arguments)]
fn process_clinic_actions_system(
    mut actions: MessageReader<ClinicActionMessage>,
    mut transport_res: ResMut<ClinicTransportResource>,
    session_hub: Res<SharedBrowserSessionHub>,
    locale_storage: Res<SharedLocaleStorage>,
    mut runtime_cfg: ResMut<ClientRuntimeConfig>,
    mut session: ResMut<ClinicSessionShell>,
    mut roster: ResMut<TodayRosterView>,
    mut workspace: ResMut<ActiveWorkspaceView>,
    mut composer: ResMut<ComposerSurface>,
    mut router: ResMut<ShortcutRouter>,
    mut ui_locale: ResMut<UiLocale>,
    mut live: ResMut<PoliteLiveQueue>,
    mut gate: ResMut<IdleRedrawGate>,
    clock: Res<ClientClock>,
    mut frame_report: ResMut<ClientFrameReport>,
) {
    let mut tree_dirty = false;

    for msg in actions.read() {
        let now_secs = clock.now_secs;
        let now_ms = clock.now_ms;

        match msg.0.clone() {
            ClinicAction::BootTab => {
                if let Some(next) = session_hub.resolve_opening(
                    &session.tab_id,
                    transport_res.transport_mut(),
                    now_secs,
                ) {
                    apply_tab_transition(
                        next,
                        &runtime_cfg.boot,
                        transport_res.transport_mut(),
                        &mut session,
                        &mut roster,
                        &mut workspace,
                        &mut composer,
                        &mut router,
                        ui_locale.current,
                        &mut live,
                        now_secs,
                        now_ms,
                    );
                    gate.request_redraw(RedrawReason::FocusTransition);
                    tree_dirty = true;
                }
            }
            ClinicAction::SubmitSignInEmail { email } => {
                let req = SignInRequest { email };
                match transport_res
                    .transport_mut()
                    .initiate_sign_in(&req, now_secs)
                {
                    Ok(SignInInitiationResponse::Unavailable { message, .. }) => {
                        session.sign_in_form = SignInFormStatus::LinkRequested {
                            message: message.clone(),
                            proof_token: None,
                        };
                        let _ = live.announce_status(message, now_ms);
                    }
                    Ok(SignInInitiationResponse::TokenIssued { token, message }) => {
                        session.sign_in_form = SignInFormStatus::LinkRequested {
                            message: message.clone(),
                            proof_token: Some(token),
                        };
                        let _ = live.announce_status(message, now_ms);
                    }
                    Err(TransportError::Auth(AuthError::MailDeliveryFailed)) => {
                        let detail = "Sign-in email could not be delivered right now.".to_owned();
                        session.sign_in_form = SignInFormStatus::MailDeliveryFailed {
                            detail: detail.clone(),
                        };
                        let _ = live.announce_assertive(detail, now_ms);
                    }
                    Err(TransportError::Auth(AuthError::Refused(refusal))) => {
                        session.sign_in_form = SignInFormStatus::Refused {
                            code: refusal.reason,
                            detail: refusal.detail.clone(),
                        };
                        let _ = live.announce_assertive(refusal.detail, now_ms);
                    }
                    Err(err) => {
                        session.sign_in_form = SignInFormStatus::Refused {
                            code: RefusalCode::SessionEnded,
                            detail: err.to_string(),
                        };
                        let _ = live.announce_assertive(err.to_string(), now_ms);
                    }
                }
                gate.request_redraw(RedrawReason::UserInput);
                tree_dirty = true;
            }
            ClinicAction::VerifySignInToken {
                email,
                token,
                entry,
            } => {
                if entry == SignInEntryMethod::EmailedLink {
                    session_hub.begin_email_sign_in_link(&session.tab_id);
                }
                let req = VerifySignInRequest { email, token };
                match transport_res.transport_mut().verify_sign_in(&req, now_secs) {
                    Ok(verified) => {
                        session.sign_in_form = SignInFormStatus::Idle;
                        session.session_banner = None;
                        if let Some(next) =
                            session_hub.complete_sign_in(&session.tab_id, verified.session)
                        {
                            apply_tab_transition(
                                next,
                                &runtime_cfg.boot,
                                transport_res.transport_mut(),
                                &mut session,
                                &mut roster,
                                &mut workspace,
                                &mut composer,
                                &mut router,
                                ui_locale.current,
                                &mut live,
                                now_secs,
                                now_ms,
                            );
                        }
                    }
                    Err(TransportError::Auth(AuthError::Refused(refusal))) => {
                        if entry == SignInEntryMethod::EmailedLink {
                            // NoDeadlock: leave SignInLink cleanly back to SignInPage(Today).
                            if let Some(next) = session_hub.reconcile_tab(
                                &session.tab_id,
                                transport_res.transport_mut(),
                                now_secs,
                            ) {
                                session.view_state = match next {
                                    TabViewState::SignInLink { .. } => TabViewState::SignInPage {
                                        wants: DeepLinkTarget::Today,
                                    },
                                    other => other,
                                };
                            }
                        }
                        session.sign_in_form = SignInFormStatus::Refused {
                            code: refusal.reason,
                            detail: refusal.detail.clone(),
                        };
                        session.session_banner = Some(refusal.detail.clone());
                        let _ = live.announce_assertive(refusal.detail, now_ms);
                    }
                    Err(err) => {
                        session.sign_in_form = SignInFormStatus::Refused {
                            code: RefusalCode::SessionEnded,
                            detail: err.to_string(),
                        };
                        let _ = live.announce_assertive(err.to_string(), now_ms);
                    }
                }
                gate.request_redraw(RedrawReason::FocusTransition);
                tree_dirty = true;
            }
            ClinicAction::SignOut => {
                if let Err(err) =
                    session_hub.sign_out(&session.tab_id, transport_res.transport_mut(), now_secs)
                {
                    let _ = live.announce_assertive(err.to_string(), now_ms);
                }
                clear_all_protected_views(&mut session, &mut roster, &mut workspace, &mut composer);
                session.view_state = TabViewState::SignInPage {
                    wants: DeepLinkTarget::Today,
                };
                session.sse_connected = false;
                router.set_focus(FocusId(A11ySemanticKey::MainLandmark));
                let _ = live
                    .announce_status(translate(ui_locale.current, UiKey::ActionSignOut), now_ms);
                gate.request_redraw(RedrawReason::FocusTransition);
                tree_dirty = true;
            }
            ClinicAction::HandleSessionEndedRefusal { failed_session } => {
                if let Some(next) = session_hub.handle_session_ended_response(
                    &session.tab_id,
                    &failed_session,
                    transport_res.transport_mut(),
                    now_secs,
                ) {
                    apply_tab_transition(
                        next,
                        &runtime_cfg.boot,
                        transport_res.transport_mut(),
                        &mut session,
                        &mut roster,
                        &mut workspace,
                        &mut composer,
                        &mut router,
                        ui_locale.current,
                        &mut live,
                        now_secs,
                        now_ms,
                    );
                    gate.request_redraw(RedrawReason::FocusTransition);
                    tree_dirty = true;
                }
            }
            ClinicAction::ReconcilePeerTabs => {
                if let Some(next) = session_hub.reconcile_tab(
                    &session.tab_id,
                    transport_res.transport_mut(),
                    now_secs,
                ) && next != session.view_state
                {
                    apply_tab_transition(
                        next,
                        &runtime_cfg.boot,
                        transport_res.transport_mut(),
                        &mut session,
                        &mut roster,
                        &mut workspace,
                        &mut composer,
                        &mut router,
                        ui_locale.current,
                        &mut live,
                        now_secs,
                        now_ms,
                    );
                    gate.request_redraw(RedrawReason::FocusTransition);
                    tree_dirty = true;
                }
            }
            ClinicAction::LoadPatientRoster { query } => {
                let Some(token) = session_hub.active_session_for(&session.tab_id) else {
                    continue;
                };
                roster.search_query = query.clone().unwrap_or_default();
                match transport_res.transport_mut().fetch_patient_roster(
                    &token,
                    query.as_deref(),
                    now_secs,
                ) {
                    Ok(state) => {
                        match &state {
                            PatientRosterState::Empty { .. } => {
                                let _ = live.announce_status(
                                    translate(ui_locale.current, UiKey::ClinicPatientRosterEmpty),
                                    now_ms,
                                );
                            }
                            PatientRosterState::Failed { message } => {
                                let _ = live.announce_assertive(message.clone(), now_ms);
                            }
                            PatientRosterState::Loaded { .. } => {}
                        }
                        roster.apply_roster(state);
                    }
                    Err(TransportError::Auth(_)) => {
                        if let Some(next) = session_hub.handle_session_ended_response(
                            &session.tab_id,
                            &token,
                            transport_res.transport_mut(),
                            now_secs,
                        ) {
                            apply_tab_transition(
                                next,
                                &runtime_cfg.boot,
                                transport_res.transport_mut(),
                                &mut session,
                                &mut roster,
                                &mut workspace,
                                &mut composer,
                                &mut router,
                                ui_locale.current,
                                &mut live,
                                now_secs,
                                now_ms,
                            );
                        }
                    }
                    Err(err) => {
                        roster.apply_roster(PatientRosterState::Failed {
                            message: err.to_string(),
                        });
                        let _ = live.announce_assertive(err.to_string(), now_ms);
                    }
                }
                gate.request_redraw(RedrawReason::UserInput);
                tree_dirty = true;
            }
            ClinicAction::OpenPatientWorkspace {
                workspace_id,
                chat_id,
            } => {
                let Some(token) = session_hub.active_session_for(&session.tab_id) else {
                    continue;
                };
                open_patient_workspace_inner(
                    &token,
                    &workspace_id,
                    chat_id.as_ref(),
                    transport_res.transport_mut(),
                    &session_hub,
                    &mut session,
                    &mut workspace,
                    &mut composer,
                    &mut router,
                    ui_locale.current,
                    &mut live,
                    now_secs,
                    now_ms,
                );
                gate.request_redraw(RedrawReason::FocusTransition);
                tree_dirty = true;
            }
            ClinicAction::ComposerEdit(action) => {
                let outcome = composer.apply(action);
                match outcome {
                    ComposerOutcome::Edited => {
                        workspace.draft_controller.edit(composer.text());
                        workspace.refresh_draft_banner();
                        gate.request_redraw(RedrawReason::UserInput);
                        tree_dirty = true;
                    }
                    ComposerOutcome::SelectionChanged => {
                        gate.request_redraw(RedrawReason::UserInput);
                    }
                    ComposerOutcome::Submitted(text) => {
                        // Restore text into composer so SubmitMessage controls prefix clearing after server ACK.
                        let _ = composer.apply(ComposerAction::InsertText(text));
                        workspace.draft_controller.edit(composer.text());
                        workspace.refresh_draft_banner();
                        gate.request_redraw(RedrawReason::UserInput);
                        tree_dirty = true;
                    }
                    ComposerOutcome::NoOp => {}
                }
            }
            ClinicAction::AutosaveDraft => {
                let Some(token) = session_hub.active_session_for(&session.tab_id) else {
                    continue;
                };
                let Some(open) = workspace.open_workspace.as_ref() else {
                    continue;
                };
                if open.membership != MembershipTier::Allowed {
                    continue;
                }
                let ws_id = open.workspace_id.clone();
                let chat_target = open.chat_id.as_str().to_owned();
                if let Some((text, expected_revision, req_gen)) =
                    workspace.draft_controller.begin_autosave()
                {
                    let req = PutDraftRequest {
                        text,
                        expected_revision,
                    };
                    let outcome = match transport_res.transport_mut().put_draft(
                        &token,
                        &ws_id,
                        &chat_target,
                        &req,
                        now_secs,
                    ) {
                        Ok(res) => res,
                        Err(err) => DraftSaveOutcome::TransientFailure {
                            message: err.to_string(),
                        },
                    };
                    workspace
                        .draft_controller
                        .finish_autosave(req_gen, outcome.clone());
                    workspace.refresh_draft_banner();
                    match outcome {
                        DraftSaveOutcome::Saved(_) => {
                            let _ = live.announce_status(
                                translate(ui_locale.current, UiKey::ClinicDraftSaved),
                                now_ms,
                            );
                        }
                        DraftSaveOutcome::Conflict { .. } => {
                            let _ = live.announce_assertive(
                                translate(ui_locale.current, UiKey::ClinicDraftConflict),
                                now_ms,
                            );
                        }
                        DraftSaveOutcome::TransientFailure { .. } => {
                            let _ = live.announce_status(
                                translate(ui_locale.current, UiKey::ClinicDraftNotSaved),
                                now_ms,
                            );
                        }
                    }
                    gate.request_redraw(RedrawReason::UserInput);
                    tree_dirty = true;
                }
            }
            ClinicAction::ResolveDraftConflict(choice) => {
                let Some(token) = session_hub.active_session_for(&session.tab_id) else {
                    continue;
                };
                let Some(open) = workspace.open_workspace.as_ref() else {
                    continue;
                };
                let ws_id = open.workspace_id.clone();
                let chat_target = open.chat_id.as_str().to_owned();
                match choice {
                    DraftConflictChoice::KeepMine => {
                        if let Some((text, expected_revision, req_gen)) =
                            workspace.draft_controller.resolve_keep_mine()
                        {
                            let req = PutDraftRequest {
                                text,
                                expected_revision,
                            };
                            let outcome = match transport_res.transport_mut().put_draft(
                                &token,
                                &ws_id,
                                &chat_target,
                                &req,
                                now_secs,
                            ) {
                                Ok(res) => res,
                                Err(err) => DraftSaveOutcome::TransientFailure {
                                    message: err.to_string(),
                                },
                            };
                            workspace.draft_controller.finish_autosave(req_gen, outcome);
                        }
                    }
                    DraftConflictChoice::UseNewer => {
                        if workspace.draft_controller.resolve_use_newer() {
                            replace_composer_text(
                                &mut composer,
                                workspace.draft_controller.local_text(),
                            );
                        }
                    }
                }
                workspace.refresh_draft_banner();
                gate.request_redraw(RedrawReason::UserInput);
                tree_dirty = true;
            }
            ClinicAction::SubmitMessage {
                idempotency_key,
                delivery,
            } => {
                let Some(token) = session_hub.active_session_for(&session.tab_id) else {
                    continue;
                };
                let Some(open) = workspace.open_workspace.as_ref() else {
                    continue;
                };
                let submitted_text = composer.text().to_owned();
                let envelope = CommandEnvelope {
                    idempotency_key,
                    command: CommandPayload::MessageSubmit(SubmitMessagePayload {
                        patient_workspace_id: open.workspace_id.clone(),
                        chat_id: open.chat_id.clone(),
                        text: submitted_text.clone(),
                        physician_time_zone: "UTC".to_owned(),
                        delivery,
                    }),
                };
                let ws_id = open.workspace_id.clone();
                let chat_id = open.chat_id.clone();

                match transport_res
                    .transport_mut()
                    .submit_command(&token, &envelope, now_secs)
                {
                    Ok(_) => {
                        // Clear ONLY the submitted prefix and persist cleared draft.
                        workspace
                            .draft_controller
                            .clear_submitted_prefix(&submitted_text);
                        replace_composer_text(
                            &mut composer,
                            workspace.draft_controller.local_text(),
                        );
                        if let Some((text, expected_revision, req_gen)) =
                            workspace.draft_controller.begin_autosave()
                        {
                            let req = PutDraftRequest {
                                text,
                                expected_revision,
                            };
                            if let Ok(outcome) = transport_res.transport_mut().put_draft(
                                &token,
                                &ws_id,
                                chat_id.as_str(),
                                &req,
                                now_secs,
                            ) {
                                workspace.draft_controller.finish_autosave(req_gen, outcome);
                            }
                        }
                        workspace.refresh_draft_banner();
                        refresh_snapshot_inner(
                            &token,
                            &ws_id,
                            &chat_id,
                            transport_res.transport_mut(),
                            &mut workspace,
                            now_secs,
                        );
                        let _ = live.announce_status(
                            translate(ui_locale.current, UiKey::ClinicStatusWorking),
                            now_ms,
                        );
                    }
                    Err(err) => {
                        workspace.last_workspace_error = Some(err.to_string());
                        let _ = live.announce_assertive(err.to_string(), now_ms);
                    }
                }
                gate.request_redraw(RedrawReason::UserInput);
                tree_dirty = true;
            }
            ClinicAction::StopActiveWork { idempotency_key } => {
                let Some(token) = session_hub.active_session_for(&session.tab_id) else {
                    continue;
                };
                let Some(open) = workspace.open_workspace.as_ref() else {
                    continue;
                };
                let active_work_id = open.exchanges.iter().rev().find_map(|ex| match &ex.answer {
                    AnswerState::Working { stop, .. } => Some(stop.work_item_id.clone()),
                    _ => None,
                });
                let Some(work_item_id) = active_work_id else {
                    continue;
                };
                let ws_id = open.workspace_id.clone();
                let chat_id = open.chat_id.clone();
                let envelope = CommandEnvelope {
                    idempotency_key,
                    command: CommandPayload::WorkStop(StopWorkPayload {
                        patient_workspace_id: ws_id.clone(),
                        chat_id: chat_id.clone(),
                        work_item_id,
                    }),
                };
                match transport_res
                    .transport_mut()
                    .submit_command(&token, &envelope, now_secs)
                {
                    Ok(_) => {
                        refresh_snapshot_inner(
                            &token,
                            &ws_id,
                            &chat_id,
                            transport_res.transport_mut(),
                            &mut workspace,
                            now_secs,
                        );
                        let _ = live.announce_status(
                            translate(ui_locale.current, UiKey::ClinicStatusStopped),
                            now_ms,
                        );
                    }
                    Err(err) => {
                        workspace.last_workspace_error = Some(err.to_string());
                        let _ = live.announce_assertive(err.to_string(), now_ms);
                    }
                }
                gate.request_redraw(RedrawReason::UserInput);
                tree_dirty = true;
            }
            ClinicAction::ResumeOrRetryWork {
                idempotency_key,
                work_item_id,
                expected_run_id,
            } => {
                let Some(token) = session_hub.active_session_for(&session.tab_id) else {
                    continue;
                };
                let Some(open) = workspace.open_workspace.as_ref() else {
                    continue;
                };
                let ws_id = open.workspace_id.clone();
                let chat_id = open.chat_id.clone();
                let envelope = CommandEnvelope {
                    idempotency_key,
                    command: CommandPayload::WorkResume(ResumeWorkPayload {
                        patient_workspace_id: ws_id.clone(),
                        chat_id: chat_id.clone(),
                        work_item_id,
                        expected_run_id,
                    }),
                };
                match transport_res
                    .transport_mut()
                    .submit_command(&token, &envelope, now_secs)
                {
                    Ok(_) => {
                        refresh_snapshot_inner(
                            &token,
                            &ws_id,
                            &chat_id,
                            transport_res.transport_mut(),
                            &mut workspace,
                            now_secs,
                        );
                        let _ = live.announce_status(
                            translate(ui_locale.current, UiKey::ClinicStatusWorking),
                            now_ms,
                        );
                    }
                    Err(err) => {
                        workspace.last_workspace_error = Some(err.to_string());
                        let _ = live.announce_assertive(err.to_string(), now_ms);
                    }
                }
                gate.request_redraw(RedrawReason::UserInput);
                tree_dirty = true;
            }
            ClinicAction::PollStream => {
                let Some(token) = session_hub.active_session_for(&session.tab_id) else {
                    continue;
                };
                let since = workspace.last_event_id;
                match transport_res
                    .transport_mut()
                    .poll_stream_events(&token, since, now_secs)
                {
                    Ok(events) => {
                        if events.is_empty() {
                            continue;
                        }
                        let mut needs_resync = false;
                        for ev in &events {
                            match workspace.apply_stream_event(ev) {
                                StreamApplyEffect::DeltaAppended { delta } => {
                                    let _ = live.push_stream_chunk(&delta, now_ms);
                                    gate.request_redraw(RedrawReason::StreamDelta);
                                    tree_dirty = true;
                                }
                                StreamApplyEffect::StepUpdated => {
                                    gate.request_redraw(RedrawReason::StreamDelta);
                                    tree_dirty = true;
                                }
                                StreamApplyEffect::StateTransitioned => {
                                    let _ = live.finish_stream(now_ms);
                                    gate.request_redraw(RedrawReason::StreamDelta);
                                    tree_dirty = true;
                                }
                                StreamApplyEffect::NeedsSnapshotResync => {
                                    needs_resync = true;
                                }
                                StreamApplyEffect::Ignored => {}
                            }
                        }
                        if needs_resync && let Some(open) = workspace.open_workspace.as_ref() {
                            let ws_id = open.workspace_id.clone();
                            let chat_id = open.chat_id.clone();
                            refresh_snapshot_inner(
                                &token,
                                &ws_id,
                                &chat_id,
                                transport_res.transport_mut(),
                                &mut workspace,
                                now_secs,
                            );
                            let _ = live.finish_stream(now_ms);
                            gate.request_redraw(RedrawReason::StreamDelta);
                            tree_dirty = true;
                        }
                    }
                    Err(TransportError::Auth(_)) => {
                        if let Some(next) = session_hub.handle_session_ended_response(
                            &session.tab_id,
                            &token,
                            transport_res.transport_mut(),
                            now_secs,
                        ) {
                            apply_tab_transition(
                                next,
                                &runtime_cfg.boot,
                                transport_res.transport_mut(),
                                &mut session,
                                &mut roster,
                                &mut workspace,
                                &mut composer,
                                &mut router,
                                ui_locale.current,
                                &mut live,
                                now_secs,
                                now_ms,
                            );
                            gate.request_redraw(RedrawReason::FocusTransition);
                            tree_dirty = true;
                        }
                    }
                    Err(_) => {}
                }
            }
            ClinicAction::OpenCitationChip(chip) => {
                let Some(token) = session_hub.active_session_for(&session.tab_id) else {
                    continue;
                };
                let Some(open) = workspace.open_workspace.as_ref() else {
                    continue;
                };
                let ws_id = open.workspace_id.clone();
                let chat_id = open.chat_id.clone();

                router.set_focus(FocusId(A11ySemanticKey::CitationChip(u32::from(
                    chip.number,
                ))));
                let _ = router.open_overlay(OverlayKind::CitationPane);
                let req_id = workspace.citation_panel.open_chip(chip.clone());

                let outcome = match transport_res.transport_mut().fetch_cited_source(
                    &token,
                    &ws_id,
                    &chat_id,
                    &chip.source_key,
                    now_secs,
                ) {
                    Ok(payload) => CitationFetchOutcome::Loaded(Box::new(payload)),
                    Err(TransportError::Workspace(
                        WorkspaceAccessError::SourceWithdrawn
                        | WorkspaceAccessError::NotFound
                        | WorkspaceAccessError::ClinicalDisclosureForbidden,
                    )) => CitationFetchOutcome::Unavailable {
                        eyebrow: translate(ui_locale.current, UiKey::ClinicCitedSourcesHeading)
                            .to_owned(),
                        title: translate(ui_locale.current, UiKey::ClinicCitationUnavailable)
                            .to_owned(),
                        reason: "Cited record has been withdrawn, redacted, or is outside authorized access."
                            .to_owned(),
                    },
                    Err(err) => CitationFetchOutcome::Failed {
                        message: err.to_string(),
                    },
                };

                let _applied = workspace.citation_panel.apply_response(req_id, outcome);
                workspace.sync_citation_panel_projection(ui_locale.current);
                match workspace.citation_panel.state() {
                    CitationPanelState::Loaded { source, .. } => {
                        let _ = live.announce_status(source.title.clone(), now_ms);
                    }
                    CitationPanelState::Unavailable { title, .. } => {
                        let _ = live.announce_assertive(title.clone(), now_ms);
                    }
                    CitationPanelState::Failed { message, .. } => {
                        let _ = live.announce_assertive(message.clone(), now_ms);
                    }
                    CitationPanelState::Idle | CitationPanelState::Loading { .. } => {}
                }
                gate.request_redraw(RedrawReason::OverlayTransition);
                tree_dirty = true;
            }
            ClinicAction::DeliverCitationResponse {
                request_id,
                outcome,
            } => {
                if workspace.citation_panel.apply_response(request_id, outcome) {
                    workspace.sync_citation_panel_projection(ui_locale.current);
                    gate.request_redraw(RedrawReason::OverlayTransition);
                    tree_dirty = true;
                }
            }
            ClinicAction::CloseCitationPanel => {
                workspace.citation_panel.close();
                workspace.sync_citation_panel_projection(ui_locale.current);
                gate.request_redraw(RedrawReason::OverlayTransition);
                tree_dirty = true;
            }
            ClinicAction::ScrollLabTable { offset_px } => {
                workspace.lab_table.scroll_vertical_to(offset_px);
                gate.request_redraw(RedrawReason::ScrollOrResize);
                tree_dirty = true;
            }
            ClinicAction::ScrollTranscript { offset_px } => {
                workspace.transcript_list.scroll_to(offset_px);
                gate.request_redraw(RedrawReason::ScrollOrResize);
                tree_dirty = true;
            }
            ClinicAction::DispatchShortcut(input) => {
                let editing = if matches!(
                    router.current_focus().0,
                    A11ySemanticKey::ComposerInput | A11ySemanticKey::PatientFinderInput
                ) {
                    EditingContext::ActiveTextField
                } else {
                    EditingContext::Inactive
                };
                let search_query = if roster.finder_open && !roster.search_query.is_empty() {
                    SearchQueryState::NonEmpty
                } else {
                    SearchQueryState::Empty
                };
                let action = router.route(
                    input,
                    ShortcutContext {
                        editing,
                        search_query,
                    },
                );
                match action {
                    ShortcutAction::PrivacyCoverActivated => {
                        let _ = live.announce_assertive(
                            translate(ui_locale.current, UiKey::PrivacyCoverTitle),
                            now_ms,
                        );
                        gate.request_redraw(RedrawReason::OverlayTransition);
                        tree_dirty = true;
                    }
                    ShortcutAction::PrivacyCoverDismissed { .. } => {
                        let _ = live.announce_status(
                            translate(ui_locale.current, UiKey::PrivacyCoverDismiss),
                            now_ms,
                        );
                        gate.request_redraw(RedrawReason::OverlayTransition);
                        tree_dirty = true;
                    }
                    ShortcutAction::OpenPatientFinder { .. } => {
                        roster.finder_open = true;
                        gate.request_redraw(RedrawReason::OverlayTransition);
                        tree_dirty = true;
                    }
                    ShortcutAction::ClearSearchInput => {
                        roster.search_query.clear();
                        gate.request_redraw(RedrawReason::UserInput);
                        tree_dirty = true;
                    }
                    ShortcutAction::DismissOverlay { dismissed, .. } => {
                        match dismissed {
                            OverlayKind::PatientFinder => roster.finder_open = false,
                            OverlayKind::CitationPane => {
                                workspace.citation_panel.close();
                                workspace.sync_citation_panel_projection(ui_locale.current);
                            }
                            OverlayKind::ShortcutsHelp
                            | OverlayKind::MobileDrawer
                            | OverlayKind::ConfirmModal => {}
                        }
                        gate.request_redraw(RedrawReason::OverlayTransition);
                        tree_dirty = true;
                    }
                    ShortcutAction::ListMoveNext => {
                        if let Some(next_idx) = roster.list.move_focus_next() {
                            router.set_focus(FocusId(A11ySemanticKey::ListRow(
                                u32::try_from(next_idx).unwrap_or(u32::MAX),
                            )));
                            gate.request_redraw(RedrawReason::FocusTransition);
                            tree_dirty = true;
                        }
                    }
                    ShortcutAction::ListMovePrevious => {
                        if let Some(prev_idx) = roster.list.move_focus_prev() {
                            router.set_focus(FocusId(A11ySemanticKey::ListRow(
                                u32::try_from(prev_idx).unwrap_or(u32::MAX),
                            )));
                            gate.request_redraw(RedrawReason::FocusTransition);
                            tree_dirty = true;
                        }
                    }
                    ShortcutAction::ActivateFocused { .. } => {
                        if let Some(patient) = roster.focused_patient().cloned()
                            && let Some(token) = session_hub.active_session_for(&session.tab_id)
                        {
                            roster.finder_open = false;
                            open_patient_workspace_inner(
                                &token,
                                &patient.patient_workspace_id,
                                None,
                                transport_res.transport_mut(),
                                &session_hub,
                                &mut session,
                                &mut workspace,
                                &mut composer,
                                &mut router,
                                ui_locale.current,
                                &mut live,
                                now_secs,
                                now_ms,
                            );
                            gate.request_redraw(RedrawReason::FocusTransition);
                            tree_dirty = true;
                        }
                    }
                    ShortcutAction::OpenShortcutsHelp { .. }
                    | ShortcutAction::ModalFocusTrapTab { .. } => {
                        gate.request_redraw(RedrawReason::FocusTransition);
                        tree_dirty = true;
                    }
                    ShortcutAction::Unhandled => {}
                }
            }
            ClinicAction::SwitchLocale(next_locale) => {
                ui_locale.current = next_locale;
                locale_storage.write(next_locale);
                runtime_cfg.sidebar_edge = next_locale.sidebar_edge();
                runtime_cfg.citation_panel_edge = next_locale.citation_panel_edge();
                workspace.sync_citation_panel_projection(next_locale);
                let _ = live.announce_status(next_locale.native_autonym(), now_ms);
                gate.request_redraw(RedrawReason::LocaleOrThemeChange);
                tree_dirty = true;
            }
        }
    }

    if tree_dirty {
        frame_report.a11y_tree_generation = frame_report.a11y_tree_generation.saturating_add(1);
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_tab_transition(
    next: TabViewState,
    boot: &ClientBootConfig,
    transport: &mut dyn ClinicTransport,
    session: &mut ClinicSessionShell,
    roster: &mut TodayRosterView,
    workspace: &mut ActiveWorkspaceView,
    composer: &mut ComposerSurface,
    router: &mut ShortcutRouter,
    locale: UiLocaleKind,
    live: &mut PoliteLiveQueue,
    now_secs: u64,
    now_ms: u64,
) {
    match &next {
        TabViewState::SessionEndedPrompt => {
            clear_all_protected_views(session, roster, workspace, composer);
            let ended_msg = translate(locale, UiKey::ClinicSessionEnded).to_owned();
            session.session_banner = Some(ended_msg.clone());
            session.sse_connected = false;
            router.set_focus(FocusId(A11ySemanticKey::MainLandmark));
            let _ = live.announce_assertive(ended_msg, now_ms);
        }
        TabViewState::SignInPage { .. }
        | TabViewState::Opening { .. }
        | TabViewState::SignInLink { .. } => {
            clear_all_protected_views(session, roster, workspace, composer);
            session.sse_connected = false;
            router.set_focus(FocusId(A11ySemanticKey::MainLandmark));
        }
        TabViewState::Workspace {
            holds,
            opens,
            sse_connected,
        } => {
            session.sse_connected = *sse_connected;
            session.session_banner = None;
            if let Ok(state) = transport.fetch_patient_roster(holds, None, now_secs) {
                roster.apply_roster(state);
            }
            match opens {
                WorkspaceOpenView::Today => {
                    workspace.clear_protected();
                    replace_composer_text(composer, "");
                    if matches!(roster.state, PatientRosterState::Loaded { .. }) {
                        router.set_focus(FocusId(A11ySemanticKey::ListRow(0)));
                    } else {
                        router.set_focus(FocusId(A11ySemanticKey::MainLandmark));
                    }
                }
                WorkspaceOpenView::LinkedPatient(ws_id) => {
                    let chat_opt = boot.deep_link_chat.as_ref();
                    load_workspace_direct(
                        holds, ws_id, chat_opt, transport, workspace, composer, router, locale,
                        live, now_secs, now_ms,
                    );
                }
            }
        }
    }
    session.view_state = next;
}

#[allow(clippy::too_many_arguments)]
fn open_patient_workspace_inner(
    session_token: &SessionId,
    workspace_id: &WorkspaceId,
    chat_override: Option<&ChatId>,
    transport: &mut dyn ClinicTransport,
    session_hub: &SharedBrowserSessionHub,
    session: &mut ClinicSessionShell,
    workspace: &mut ActiveWorkspaceView,
    composer: &mut ComposerSurface,
    router: &mut ShortcutRouter,
    locale: UiLocaleKind,
    live: &mut PoliteLiveQueue,
    now_secs: u64,
    now_ms: u64,
) {
    load_workspace_direct(
        session_token,
        workspace_id,
        chat_override,
        transport,
        workspace,
        composer,
        router,
        locale,
        live,
        now_secs,
        now_ms,
    );
    if workspace.open_workspace.is_some() {
        let opens = WorkspaceOpenView::LinkedPatient(workspace_id.clone());
        session_hub.set_open_workspace(&session.tab_id, opens.clone());
        if let TabViewState::Workspace {
            opens: local_opens, ..
        } = &mut session.view_state
        {
            *local_opens = opens;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn load_workspace_direct(
    session_token: &SessionId,
    workspace_id: &WorkspaceId,
    chat_override: Option<&ChatId>,
    transport: &mut dyn ClinicTransport,
    workspace: &mut ActiveWorkspaceView,
    composer: &mut ComposerSurface,
    router: &mut ShortcutRouter,
    locale: UiLocaleKind,
    live: &mut PoliteLiveQueue,
    now_secs: u64,
    now_ms: u64,
) {
    let detail = match transport.fetch_workspace_detail(session_token, workspace_id, now_secs) {
        Ok(detail) => detail,
        Err(err) => {
            workspace.last_workspace_error = Some(err.to_string());
            let _ = live.announce_assertive(err.to_string(), now_ms);
            return;
        }
    };

    let chat_id = chat_override
        .cloned()
        .unwrap_or_else(|| detail.default_chat_id.clone());

    let exchanges = if detail.membership == MembershipTier::Allowed {
        let snap = transport
            .fetch_chat_snapshot(session_token, workspace_id, &chat_id, now_secs)
            .map(|s| s.transcript)
            .unwrap_or_default();
        if let Ok(draft) =
            transport.get_draft(session_token, workspace_id, chat_id.as_str(), now_secs)
        {
            workspace.draft_controller = ComposerDraftController::from_persisted(draft.clone());
            replace_composer_text(composer, &draft.text);
        }
        snap
    } else {
        // ClinicDemographic: NEVER disclose clinical transcript, citations, or composer drafts.
        replace_composer_text(composer, "");
        Vec::new()
    };

    let turn_count = exchanges.len();
    workspace.open_workspace = Some(OpenPatientWorkspace {
        workspace_id: detail.patient_workspace_id,
        chat_id,
        display_name: detail.display_name.clone(),
        membership: detail.membership,
        synopsis: detail.patient_synopsis,
        exchanges,
    });
    workspace.transcript_list.set_total_rows(turn_count);
    workspace.citation_panel.close();
    workspace.sync_citation_panel_projection(locale);
    workspace.refresh_draft_banner();
    workspace.last_workspace_error = None;
    router.set_focus(FocusId(A11ySemanticKey::MainLandmark));
    let _ = live.announce_status(detail.display_name, now_ms);
}

fn refresh_snapshot_inner(
    session_token: &SessionId,
    workspace_id: &WorkspaceId,
    chat_id: &ChatId,
    transport: &mut dyn ClinicTransport,
    workspace: &mut ActiveWorkspaceView,
    now_secs: u64,
) {
    if let Ok(snap) = transport.fetch_chat_snapshot(session_token, workspace_id, chat_id, now_secs)
        && let Some(open) = workspace.open_workspace.as_mut()
    {
        open.exchanges = snap.transcript;
        workspace
            .transcript_list
            .set_total_rows(open.exchanges.len());
    }
}

fn clear_all_protected_views(
    session: &mut ClinicSessionShell,
    roster: &mut TodayRosterView,
    workspace: &mut ActiveWorkspaceView,
    composer: &mut ComposerSurface,
) {
    roster.clear_protected();
    workspace.clear_protected();
    replace_composer_text(composer, "");
    session.sse_connected = false;
}

fn replace_composer_text(composer: &mut ComposerSurface, next_text: &str) {
    let _ = composer.apply(ComposerAction::SelectAll);
    let _ = composer.apply(ComposerAction::Backspace);
    if !next_text.is_empty() {
        let _ = composer.apply(ComposerAction::InsertText(next_text.to_owned()));
    }
}

/// Maintains the single-owner Bevy ECS + AccessKit node tree (`0` duplicates) matching
/// the active view, privacy cover, citation panel, and virtualized lab rows.
#[allow(clippy::too_many_arguments)]
fn sync_clinic_a11y_entities_system(
    mut commands: Commands,
    existing: Query<Entity, With<ManagedClinicA11yNode>>,
    session: Res<ClinicSessionShell>,
    roster: Res<TodayRosterView>,
    workspace: Res<ActiveWorkspaceView>,
    composer: Res<ComposerSurface>,
    router: Res<ShortcutRouter>,
    ui_locale: Res<UiLocale>,
    mut frame_report: ResMut<ClientFrameReport>,
) {
    if frame_report.last_rendered_a11y_generation == frame_report.a11y_tree_generation
        && frame_report.last_rendered_a11y_generation != 0
    {
        return;
    }

    for entity in &existing {
        commands.entity(entity).despawn();
    }

    let locale = ui_locale.current;
    let mut order = 0usize;

    let mut spawn_node =
        |key: A11ySemanticKey, interactivity: NodeInteractivity, node: accesskit::Node| {
            commands.spawn((
                ManagedClinicA11yNode,
                Node::default(),
                A11yNodeMarker {
                    key,
                    reading_order: order,
                    interactivity,
                },
                AccessibilityNode(node),
            ));
            order += 1;
        };

    // Privacy Screen Cover replaces all clinical content behind an opaque modal dialog.
    if router.is_privacy_cover_active() {
        let mut dialog = accesskit::Node::new(accesskit::Role::Dialog);
        dialog.set_label(translate(locale, UiKey::PrivacyCoverTitle));
        dialog.set_modal();
        spawn_node(
            A11ySemanticKey::PrivacyCoverDialog,
            NodeInteractivity::Structural,
            dialog,
        );

        let mut dismiss_btn = accesskit::Node::new(accesskit::Role::Button);
        dismiss_btn.set_label(translate(locale, UiKey::PrivacyCoverDismiss));
        spawn_node(
            A11ySemanticKey::PrivacyCoverDismissButton,
            NodeInteractivity::Interactive,
            dismiss_btn,
        );
        frame_report.last_rendered_a11y_generation = frame_report.a11y_tree_generation;
        return;
    }

    // Unauthenticated or expired views expose ZERO protected clinical nodes (`PromptIsEmpty`).
    if !session.view_state.shows_protected_content() {
        let mut main = accesskit::Node::new(accesskit::Role::Main);
        let label = if matches!(session.view_state, TabViewState::SessionEndedPrompt) {
            translate(locale, UiKey::ClinicSessionEnded)
        } else {
            translate(locale, UiKey::ActionSignIn)
        };
        main.set_label(label);
        spawn_node(
            A11ySemanticKey::MainLandmark,
            NodeInteractivity::Structural,
            main,
        );

        let mut sign_in_btn = accesskit::Node::new(accesskit::Role::Button);
        sign_in_btn.set_label(translate(locale, UiKey::ActionSignIn));
        spawn_node(
            A11ySemanticKey::SendButton,
            NodeInteractivity::Interactive,
            sign_in_btn,
        );

        frame_report.last_rendered_a11y_generation = frame_report.a11y_tree_generation;
        return;
    }

    // Authenticated Workspace landmarks.
    let mut nav = accesskit::Node::new(accesskit::Role::Navigation);
    nav.set_label(translate(locale, UiKey::ClinicPatientsHeading));
    spawn_node(
        A11ySemanticKey::NavigationLandmark,
        NodeInteractivity::Structural,
        nav,
    );

    let mut main = accesskit::Node::new(accesskit::Role::Main);
    let main_label = workspace.open_workspace.as_ref().map_or_else(
        || translate(locale, UiKey::AppBrandClinic).to_owned(),
        |ws| ws.display_name.clone(),
    );
    main.set_label(main_label);
    spawn_node(
        A11ySemanticKey::MainLandmark,
        NodeInteractivity::Structural,
        main,
    );

    // Patient Finder overlay when open.
    if roster.finder_open {
        let mut finder = accesskit::Node::new(accesskit::Role::Dialog);
        finder.set_label(translate(locale, UiKey::ActionSearch));
        finder.set_modal();
        spawn_node(
            A11ySemanticKey::PatientFinderDialog,
            NodeInteractivity::Structural,
            finder,
        );

        let mut finder_input = accesskit::Node::new(accesskit::Role::SearchInput);
        finder_input.set_label(translate(locale, UiKey::ClinicPatientFinderPlaceholder));
        finder_input.set_value(roster.search_query.clone());
        spawn_node(
            A11ySemanticKey::PatientFinderInput,
            NodeInteractivity::Interactive,
            finder_input,
        );
    }

    // Virtualized visible patient roster rows (`O(viewport)`).
    if let PatientRosterState::Loaded { patients, .. } = &roster.state {
        let (range, visible) = roster.list.slice_visible(patients);
        for (offset, patient) in visible.iter().enumerate() {
            let row_idx = range.start_row + offset;
            let mut row = accesskit::Node::new(accesskit::Role::Button);
            row.set_label(patient.display_name.clone());
            spawn_node(
                A11ySemanticKey::ListRow(u32::try_from(row_idx).unwrap_or(u32::MAX)),
                NodeInteractivity::Interactive,
                row,
            );
        }
    }

    // Active Allowed Patient Workspace controls (Composer, Send/Stop/Resume/Try-again, Citations).
    if let Some(open) = &workspace.open_workspace
        && open.membership == MembershipTier::Allowed
    {
        let mut composer_node = accesskit::Node::new(accesskit::Role::MultilineTextInput);
        composer_node.set_label(translate(locale, UiKey::ClinicComposerPlaceholder));
        composer_node.set_value(composer.display_text());
        spawn_node(
            A11ySemanticKey::ComposerInput,
            NodeInteractivity::Interactive,
            composer_node,
        );

        let mut send_btn = accesskit::Node::new(accesskit::Role::Button);
        send_btn.set_label(translate(locale, UiKey::ClinicSendMessage));
        spawn_node(
            A11ySemanticKey::SendButton,
            NodeInteractivity::Interactive,
            send_btn,
        );

        if let Some(last) = open.exchanges.last() {
            match &last.answer {
                AnswerState::Working { .. } => {
                    let mut stop_btn = accesskit::Node::new(accesskit::Role::Button);
                    stop_btn.set_label(translate(locale, UiKey::ClinicStopWork));
                    spawn_node(
                        A11ySemanticKey::StopButton,
                        NodeInteractivity::Interactive,
                        stop_btn,
                    );
                }
                AnswerState::Stopped { resume: Some(_) } => {
                    let mut resume_btn = accesskit::Node::new(accesskit::Role::Button);
                    resume_btn.set_label(translate(locale, UiKey::ClinicResumeWork));
                    spawn_node(
                        A11ySemanticKey::ResumeButton,
                        NodeInteractivity::Interactive,
                        resume_btn,
                    );
                }
                AnswerState::CouldntFinish { try_again: Some(_) }
                | AnswerState::CutOff { try_again: Some(_) } => {
                    let mut retry_btn = accesskit::Node::new(accesskit::Role::Button);
                    retry_btn.set_label(translate(locale, UiKey::ActionRetry));
                    spawn_node(
                        A11ySemanticKey::TryAgainButton,
                        NodeInteractivity::Interactive,
                        retry_btn,
                    );
                }
                _ => {}
            }
        }

        // Deduplicated inline citation chips.
        let mut seen_numbers = std::collections::BTreeSet::new();
        for chip in workspace.available_citations() {
            if seen_numbers.insert(chip.number) {
                let mut chip_node = accesskit::Node::new(accesskit::Role::Button);
                chip_node.set_label(chip.label);
                spawn_node(
                    A11ySemanticKey::CitationChip(u32::from(chip.number)),
                    NodeInteractivity::Interactive,
                    chip_node,
                );
            }
        }

        // Non-modal Cited-Source reading panel + virtualized lab table rows (`O(viewport)`).
        if !matches!(workspace.citation_panel.state(), CitationPanelState::Idle) {
            let mut panel = accesskit::Node::new(accesskit::Role::Region);
            panel.set_label(translate(locale, UiKey::ClinicCitedSourcesHeading));
            spawn_node(
                A11ySemanticKey::CitationPanelRegion,
                NodeInteractivity::Structural,
                panel,
            );

            if let CitationPanelState::Loaded { source, .. } = workspace.citation_panel.state() {
                let (range, rows) = workspace.lab_table.visible_rows_slice(&source.fields);
                for (offset, field) in rows.iter().enumerate() {
                    let row_idx = range.start_row + offset;
                    let mut row = accesskit::Node::new(accesskit::Role::Row);
                    row.set_label(format!("{} {} {}", field.name, field.value, field.unit));
                    spawn_node(
                        A11ySemanticKey::TableRow(u32::try_from(row_idx).unwrap_or(u32::MAX)),
                        NodeInteractivity::Structural,
                        row,
                    );
                }
            }
        }
    }

    frame_report.last_rendered_a11y_generation = frame_report.a11y_tree_generation;
}

fn consume_idle_redraw_gate_system(
    mut gate: ResMut<IdleRedrawGate>,
    mut frame_report: ResMut<ClientFrameReport>,
) {
    frame_report.last_decision = Some(gate.consume_frame());
}
