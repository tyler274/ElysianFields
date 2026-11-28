//! Headless Bevy `0.20.1` Curie Mobile client plugin built on [`EfUiCatalogPlugin`].

use std::collections::VecDeque;

use bevy::a11y::AccessibilityNode;
use bevy::prelude::*;
use ef_connected_care::{
    ConnectionId, ConnectionStanding, DateOfBirth, DobVerificationOutcome, EnvelopeItem,
    InviteChallenge, InviteCredential, Part2WrittenConsent, TransferEnvelope,
};
use ef_core::IdempotencyKey;
use ef_mobile_domain::{
    AccountFencedHealthBatch, ConnectivityState, HealthSampleEntry, HealthTransactionalOutbox,
    IncomingBatchItem, MobileClientScreen, MobileDeviceAuthState, MobileSessionId, PatientRecord,
    PortablePatientExport, PushTokenRegistry, SavepointBatchReport,
};
use ef_ui_catalog::{
    A11yNodeMarker, A11ySemanticKey, ChromeProduct, ComposerAction, ComposerSurface,
    EfUiCatalogPlugin, FrameDecision, IdleRedrawGate, Locale as UiLocaleKind, NodeInteractivity,
    RedrawReason, SoftKeyboardInsets, UiLocale, localized_chrome,
};

use crate::transport::{
    AccountDeleteReceipt, LocalMobileTransport, MobileClientTransport, MobileTransportError,
};

/// Active navigation tab in Curie Mobile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MobileTab {
    #[default]
    Home,
    Records,
    HealthTrends,
    ForMyDoctor,
    Settings,
}

/// Mobile OS foreground/background visibility gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MobileVisibility {
    #[default]
    Foreground,
    BackgroundPaused,
}

/// Commands dispatched into the Curie Mobile Bevy runtime.
#[derive(Debug, Clone)]
pub enum MobileClientCommand {
    SetConnectivity(ConnectivityState),
    SetVisibility(MobileVisibility),
    SetSoftKeyboardInsetPx(f32),
    SwitchTab(MobileTab),
    SwitchLocale(UiLocaleKind),
    SignIn {
        email: String,
    },
    EditLocalDraft {
        text: String,
    },
    SignOut {
        device_id: Option<String>,
    },
    FlushOwedSignOuts,
    IngestRecords(Vec<IncomingBatchItem>),
    QueueHealthBatch(AccountFencedHealthBatch),
    SyncHealthOutboxAndRefreshTrends,
    RefreshRecords,
    ConnectedCareLookup {
        credential: InviteCredential,
    },
    ConnectedCareVerifyDob {
        credential: InviteCredential,
        dob: DateOfBirth,
    },
    ConnectedCareShare {
        credential: InviteCredential,
    },
    SendForMyDoctor {
        idempotency_key: IdempotencyKey,
        connection_id: ConnectionId,
        items: Vec<EnvelopeItem>,
        part2_consent: Option<Part2WrittenConsent>,
    },
    ConnectedCareDisconnect {
        connection_id: ConnectionId,
    },
    ExportData,
    DeleteAccount {
        event_id: String,
        occurred_at: String,
    },
}

#[derive(Resource, Default)]
pub struct MobileClientCommandQueue {
    pending: VecDeque<MobileClientCommand>,
}

impl MobileClientCommandQueue {
    pub fn push(&mut self, cmd: MobileClientCommand) {
        self.pending.push_back(cmd);
    }
}

/// Marker component tagging managed Curie Mobile Bevy UI + AccessKit entities.
#[derive(Component, Debug)]
pub struct ManagedMobileA11yNode;

/// Single authoritative Curie Mobile client UI state resource.
#[derive(Resource)]
pub struct MobileClientState {
    pub auth_device: MobileDeviceAuthState,
    pub active_tab: MobileTab,
    pub visibility: MobileVisibility,
    pub outbox: HealthTransactionalOutbox,
    pub push_registry: PushTokenRegistry,
    /// Synced health trends read strictly via [`MobileClientTransport`] (never native bridges).
    pub visible_health_trends: Vec<HealthSampleEntry>,
    /// Synced patient records read strictly via [`MobileClientTransport`].
    pub visible_records: Vec<PatientRecord>,
    pub last_ingest_report: Option<SavepointBatchReport>,
    pub last_invite_challenge: Option<InviteChallenge>,
    pub last_dob_outcome: Option<DobVerificationOutcome>,
    pub last_connection_standing: Option<ConnectionStanding>,
    pub sent_envelopes: Vec<TransferEnvelope>,
    pub last_export: Option<PortablePatientExport>,
    pub last_delete_receipt: Option<AccountDeleteReceipt>,
    pub errors: Vec<String>,
    pub frames_drawn: u64,
    pub frames_skipped_background: u64,
    pub clock_secs: u64,
    pub a11y_tree_generation: u64,
    pub last_rendered_a11y_generation: u64,
}

impl Default for MobileClientState {
    fn default() -> Self {
        Self {
            auth_device: MobileDeviceAuthState::new(),
            active_tab: MobileTab::Home,
            visibility: MobileVisibility::Foreground,
            outbox: HealthTransactionalOutbox::new(),
            push_registry: PushTokenRegistry::new(),
            visible_health_trends: Vec::new(),
            visible_records: Vec::new(),
            last_ingest_report: None,
            last_invite_challenge: None,
            last_dob_outcome: None,
            last_connection_standing: None,
            sent_envelopes: Vec::new(),
            last_export: None,
            last_delete_receipt: None,
            errors: Vec::new(),
            frames_drawn: 0,
            frames_skipped_background: 0,
            clock_secs: 1_760_000_000,
            a11y_tree_generation: 1,
            last_rendered_a11y_generation: 0,
        }
    }
}

impl MobileClientState {
    fn clear_account_visible_state(&mut self, composer: &mut ComposerSurface) {
        self.visible_health_trends.clear();
        self.visible_records.clear();
        self.last_ingest_report = None;
        self.last_invite_challenge = None;
        self.last_dob_outcome = None;
        self.last_connection_standing = None;
        self.sent_envelopes.clear();
        self.last_export = None;
        let _ = composer.apply(ComposerAction::SelectAll);
        let _ = composer.apply(ComposerAction::Backspace);
    }

    fn current_session_id(&self) -> Option<MobileSessionId> {
        match self.auth_device.screen() {
            MobileClientScreen::SignedIn { session_id, .. } => Some(session_id.clone()),
            MobileClientScreen::EnterYourEmail
            | MobileClientScreen::CheckYourEmail { .. }
            | MobileClientScreen::SessionEnded => None,
        }
    }
}

/// Bevy resource wrapping the typed [`MobileClientTransport`].
#[derive(Resource)]
pub struct MobileTransportResource {
    pub transport: Box<dyn MobileClientTransport>,
}

impl Default for MobileTransportResource {
    fn default() -> Self {
        Self {
            transport: Box::new(LocalMobileTransport::new()),
        }
    }
}

/// Curie Mobile Bevy `0.20.1` plugin built directly on [`EfUiCatalogPlugin`].
pub struct CurieMobileClientPlugin;

impl Plugin for CurieMobileClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EfUiCatalogPlugin)
            .init_resource::<MobileClientCommandQueue>()
            .init_resource::<MobileClientState>()
            .init_resource::<MobileTransportResource>()
            .add_systems(
                Update,
                (
                    process_mobile_client_commands,
                    sync_mobile_a11y_entities_system,
                    drive_mobile_redraw_gate,
                )
                    .chain(),
            );
    }
}

fn process_mobile_client_commands(
    mut queue: ResMut<MobileClientCommandQueue>,
    mut state: ResMut<MobileClientState>,
    mut transport_res: ResMut<MobileTransportResource>,
    mut composer: ResMut<ComposerSurface>,
    mut ui_locale: ResMut<UiLocale>,
    mut gate: ResMut<IdleRedrawGate>,
) {
    let mut tree_dirty = false;
    while let Some(cmd) = queue.pending.pop_front() {
        match cmd {
            MobileClientCommand::SetConnectivity(conn) => {
                state.auth_device.set_connectivity(conn);
                gate.request_redraw(RedrawReason::StreamDelta);
                tree_dirty = true;
            }
            MobileClientCommand::SetVisibility(vis) => {
                state.visibility = vis;
                if vis == MobileVisibility::Foreground {
                    gate.request_redraw(RedrawReason::UserInput);
                }
            }
            MobileClientCommand::SetSoftKeyboardInsetPx(bottom_inset_px) => {
                composer.set_soft_keyboard_insets(SoftKeyboardInsets { bottom_inset_px });
                gate.request_redraw(RedrawReason::ScrollOrResize);
            }
            MobileClientCommand::SwitchTab(tab) => {
                state.active_tab = tab;
                gate.request_redraw(RedrawReason::UserInput);
                tree_dirty = true;
            }
            MobileClientCommand::SwitchLocale(next_locale) => {
                ui_locale.current = next_locale;
                composer.set_direction(next_locale.direction());
                gate.request_redraw(RedrawReason::LocaleOrThemeChange);
                tree_dirty = true;
            }
            MobileClientCommand::SignIn { email } => {
                let now = state.clock_secs;
                if state.auth_device.connectivity() == ConnectivityState::Offline {
                    state.errors.push(MobileTransportError::Offline.to_string());
                    continue;
                }
                if let Err(err) = state.auth_device.submit_email(&email) {
                    state.errors.push(err.to_string());
                    continue;
                }
                let token = match transport_res.transport.request_sign_in_link(&email, now) {
                    Ok(t) => t,
                    Err(err) => {
                        state.auth_device.return_to_sign_in();
                        state.errors.push(err.to_string());
                        continue;
                    }
                };
                match transport_res.transport.verify_sign_in(&email, &token, now) {
                    Ok(verified) => {
                        // Clean handoff: clear any visible trends/records/composer before entering SignedIn.
                        state.clear_account_visible_state(&mut composer);
                        state.auth_device.complete_sign_in(verified);
                        gate.request_redraw(RedrawReason::StreamDelta);
                    }
                    Err(err) => {
                        state.auth_device.return_to_sign_in();
                        state.errors.push(err.to_string());
                    }
                }
            }
            MobileClientCommand::EditLocalDraft { text } => {
                if let Err(err) = state.auth_device.set_local_draft(text.clone()) {
                    state.errors.push(err.to_string());
                    continue;
                }
                let _ = composer.apply(ComposerAction::SelectAll);
                let _ = composer.apply(ComposerAction::Backspace);
                let _ = composer.apply(ComposerAction::InsertText(text));
                gate.request_redraw(RedrawReason::UserInput);
                tree_dirty = true;
            }
            MobileClientCommand::SignOut { device_id } => {
                if let Some(dev_id) = device_id {
                    let _ = state.push_registry.revoke_device(&dev_id);
                }
                // Clear all visible trends, records, and composer buffer immediately (`SignInHoldsNothing`).
                state.clear_account_visible_state(&mut composer);

                // Extract signed-out session & queue offline or revoke online through transport.
                let mut local_auth_shadow = ef_mobile_domain::MobileAuthAuthority::new();
                let connectivity = state.auth_device.connectivity();
                if let Some(revoked_session) = state.auth_device.sign_out(&mut local_auth_shadow)
                    && connectivity == ConnectivityState::Online
                    && let Err(err) = transport_res.transport.revoke_session(&revoked_session)
                {
                    state.errors.push(err.to_string());
                }
                gate.request_redraw(RedrawReason::UserInput);
            }
            MobileClientCommand::FlushOwedSignOuts => {
                if state.auth_device.connectivity() == ConnectivityState::Offline {
                    continue;
                }
                let owed_sessions: Vec<MobileSessionId> = state
                    .auth_device
                    .owed_sign_outs()
                    .iter()
                    .map(|o| o.session_id.clone())
                    .collect();
                if owed_sessions.is_empty() {
                    continue;
                }
                match transport_res
                    .transport
                    .revoke_sessions(None, &owed_sessions)
                {
                    Ok(_) => {
                        let mut dummy = ef_mobile_domain::MobileAuthAuthority::new();
                        let _ = state.auth_device.flush_owed_sign_outs(&mut dummy);
                    }
                    Err(err) => state.errors.push(err.to_string()),
                }
                gate.request_redraw(RedrawReason::StreamDelta);
            }
            MobileClientCommand::IngestRecords(items) => {
                let Some(session_id) = state.current_session_id() else {
                    continue;
                };
                let now = state.clock_secs;
                match transport_res
                    .transport
                    .ingest_records(&session_id, items, now)
                {
                    Ok(report) => {
                        state.last_ingest_report = Some(report);
                        gate.request_redraw(RedrawReason::StreamDelta);
                    }
                    Err(err) => state.errors.push(err.to_string()),
                }
            }
            MobileClientCommand::QueueHealthBatch(batch) => {
                if let Err(err) = state.outbox.enqueue(batch) {
                    state.errors.push(err.to_string());
                }
            }
            MobileClientCommand::SyncHealthOutboxAndRefreshTrends => {
                if state.auth_device.connectivity() == ConnectivityState::Offline {
                    continue;
                }
                let MobileClientScreen::SignedIn {
                    account_id,
                    incarnation,
                    session_id,
                    ..
                } = state.auth_device.screen().clone()
                else {
                    continue;
                };
                let now = state.clock_secs;
                let batches = state.outbox.pending_for_account(&account_id, incarnation);
                for batch in batches {
                    let key = batch.idempotency_key.clone();
                    match transport_res
                        .transport
                        .upload_health_batch(&session_id, batch, now)
                    {
                        Ok(_) => state.outbox.mark_uploaded(&account_id, &key),
                        Err(err) => state.errors.push(err.to_string()),
                    }
                }
                match transport_res
                    .transport
                    .fetch_health_trends(&session_id, now)
                {
                    Ok(trends) => {
                        state.visible_health_trends = trends;
                        gate.request_redraw(RedrawReason::StreamDelta);
                    }
                    Err(err) => state.errors.push(err.to_string()),
                }
            }
            MobileClientCommand::RefreshRecords => {
                let Some(session_id) = state.current_session_id() else {
                    continue;
                };
                let now = state.clock_secs;
                match transport_res.transport.fetch_records(&session_id, now) {
                    Ok(records) => {
                        state.visible_records = records;
                        gate.request_redraw(RedrawReason::StreamDelta);
                    }
                    Err(err) => state.errors.push(err.to_string()),
                }
            }
            MobileClientCommand::ConnectedCareLookup { credential } => {
                let Some(session_id) = state.current_session_id() else {
                    continue;
                };
                let now = state.clock_secs;
                match transport_res
                    .transport
                    .connected_care_lookup(&session_id, credential, now)
                {
                    Ok(challenge) => {
                        state.last_invite_challenge = Some(challenge);
                        gate.request_redraw(RedrawReason::StreamDelta);
                    }
                    Err(err) => state.errors.push(err.to_string()),
                }
            }
            MobileClientCommand::ConnectedCareVerifyDob { credential, dob } => {
                let Some(session_id) = state.current_session_id() else {
                    continue;
                };
                let now = state.clock_secs;
                match transport_res.transport.connected_care_verify_dob(
                    &session_id,
                    credential,
                    dob,
                    now,
                ) {
                    Ok(outcome) => {
                        state.last_dob_outcome = Some(outcome);
                        gate.request_redraw(RedrawReason::StreamDelta);
                    }
                    Err(err) => state.errors.push(err.to_string()),
                }
            }
            MobileClientCommand::ConnectedCareShare { credential } => {
                let Some(session_id) = state.current_session_id() else {
                    continue;
                };
                let now = state.clock_secs;
                match transport_res
                    .transport
                    .connected_care_share(&session_id, credential, now)
                {
                    Ok(standing) => {
                        state.last_connection_standing = Some(standing);
                        gate.request_redraw(RedrawReason::StreamDelta);
                    }
                    Err(err) => state.errors.push(err.to_string()),
                }
            }
            MobileClientCommand::SendForMyDoctor {
                idempotency_key,
                connection_id,
                items,
                part2_consent,
            } => {
                let Some(session_id) = state.current_session_id() else {
                    continue;
                };
                let now = state.clock_secs;
                match transport_res.transport.connected_care_transfer(
                    &session_id,
                    idempotency_key,
                    connection_id,
                    items,
                    part2_consent,
                    now,
                ) {
                    Ok(envelope) => {
                        state.sent_envelopes.push(envelope);
                        gate.request_redraw(RedrawReason::StreamDelta);
                    }
                    Err(err) => state.errors.push(err.to_string()),
                }
            }
            MobileClientCommand::ConnectedCareDisconnect { connection_id } => {
                let Some(session_id) = state.current_session_id() else {
                    continue;
                };
                let now = state.clock_secs;
                match transport_res.transport.connected_care_disconnect(
                    &session_id,
                    connection_id,
                    now,
                ) {
                    Ok(standing) => {
                        state.last_connection_standing = Some(standing);
                        gate.request_redraw(RedrawReason::StreamDelta);
                    }
                    Err(err) => state.errors.push(err.to_string()),
                }
            }
            MobileClientCommand::ExportData => {
                let Some(session_id) = state.current_session_id() else {
                    continue;
                };
                let now = state.clock_secs;
                match transport_res
                    .transport
                    .export_patient_data(&session_id, now)
                {
                    Ok(export) => {
                        state.last_export = Some(export);
                        gate.request_redraw(RedrawReason::StreamDelta);
                    }
                    Err(err) => state.errors.push(err.to_string()),
                }
            }
            MobileClientCommand::DeleteAccount {
                event_id,
                occurred_at,
            } => {
                let Some(session_id) = state.current_session_id() else {
                    continue;
                };
                let now = state.clock_secs;
                match transport_res.transport.delete_account(
                    &session_id,
                    &event_id,
                    &occurred_at,
                    now,
                ) {
                    Ok(receipt) => {
                        state.last_delete_receipt = Some(receipt);
                        state.clear_account_visible_state(&mut composer);
                        let mut shadow = ef_mobile_domain::MobileAuthAuthority::new();
                        let _ = state.auth_device.sign_out(&mut shadow);
                        gate.request_redraw(RedrawReason::StreamDelta);
                    }
                    Err(err) => state.errors.push(err.to_string()),
                }
            }
        }
    }
    if tree_dirty {
        state.a11y_tree_generation = state.a11y_tree_generation.saturating_add(1);
    }
}

fn sync_mobile_a11y_entities_system(
    mut commands: Commands,
    existing: Query<Entity, With<ManagedMobileA11yNode>>,
    mut state: ResMut<MobileClientState>,
    composer: Res<ComposerSurface>,
    ui_locale: Res<UiLocale>,
) {
    if state.last_rendered_a11y_generation == state.a11y_tree_generation
        && state.last_rendered_a11y_generation != 0
    {
        return;
    }

    for entity in &existing {
        commands.entity(entity).despawn();
    }

    let chrome = localized_chrome(ui_locale.current, ChromeProduct::Mobile);
    let mut order = 0usize;
    let mut spawn_node =
        |key: A11ySemanticKey, interactivity: NodeInteractivity, node: accesskit::Node| {
            commands.spawn((
                ManagedMobileA11yNode,
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

    let mut nav = accesskit::Node::new(accesskit::Role::Navigation);
    nav.set_label(format!("{} — {}", chrome.app_brand, chrome.nav_account));
    spawn_node(
        A11ySemanticKey::NavigationLandmark,
        NodeInteractivity::Structural,
        nav,
    );

    let mut crisis = accesskit::Node::new(accesskit::Role::Region);
    crisis.set_label(chrome.crisis_heading.clone());
    spawn_node(
        A11ySemanticKey::CitationPanelRegion,
        NodeInteractivity::Structural,
        crisis,
    );

    let mut main = accesskit::Node::new(accesskit::Role::Main);
    main.set_label(chrome.app_brand.clone());
    spawn_node(
        A11ySemanticKey::MainLandmark,
        NodeInteractivity::Structural,
        main,
    );

    let mut share_btn = accesskit::Node::new(accesskit::Role::Button);
    share_btn.set_label(format!("{} — {}", chrome.nav_connected_care, chrome.share));
    spawn_node(
        A11ySemanticKey::ListRow(0),
        NodeInteractivity::Interactive,
        share_btn,
    );

    let mut note_input = accesskit::Node::new(accesskit::Role::MultilineTextInput);
    note_input.set_label(chrome.nav_for_my_doctor.clone());
    note_input.set_value(composer.display_text());
    spawn_node(
        A11ySemanticKey::ComposerInput,
        NodeInteractivity::Interactive,
        note_input,
    );

    let mut signout_btn = accesskit::Node::new(accesskit::Role::Button);
    signout_btn.set_label(chrome.sign_out);
    spawn_node(
        A11ySemanticKey::SendButton,
        NodeInteractivity::Interactive,
        signout_btn,
    );

    state.last_rendered_a11y_generation = state.a11y_tree_generation;
}

fn drive_mobile_redraw_gate(
    mut state: ResMut<MobileClientState>,
    mut gate: ResMut<IdleRedrawGate>,
) {
    if state.visibility == MobileVisibility::BackgroundPaused {
        state.frames_skipped_background = state.frames_skipped_background.saturating_add(1);
        return;
    }
    if matches!(gate.consume_frame(), FrameDecision::Render { .. }) {
        state.frames_drawn = state.frames_drawn.saturating_add(1);
    }
}
