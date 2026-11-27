//! Headless Bevy `0.20.1` Curie Mobile client plugin built on [`EfUiCatalogPlugin`].

use std::collections::VecDeque;

use bevy::prelude::*;
use ef_mobile_domain::{
    AccountFencedHealthBatch, ConnectivityState, HealthSampleEntry, HealthTransactionalOutbox,
    MobileClientScreen, MobileDeviceAuthState, MobileSessionId, PatientRecord, PushTokenRegistry,
};
use ef_ui_catalog::{
    ComposerAction, ComposerSurface, EfUiCatalogPlugin, FrameDecision, IdleRedrawGate,
    RedrawReason, SoftKeyboardInsets,
};

use crate::transport::{LocalMobileTransport, MobileClientTransport, MobileTransportError};

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
    SignIn { email: String },
    EditLocalDraft { text: String },
    SignOut { device_id: Option<String> },
    FlushOwedSignOuts,
    QueueHealthBatch(AccountFencedHealthBatch),
    SyncHealthOutboxAndRefreshTrends,
    RefreshRecords,
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
    pub errors: Vec<String>,
    pub frames_drawn: u64,
    pub frames_skipped_background: u64,
    pub clock_secs: u64,
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
            errors: Vec::new(),
            frames_drawn: 0,
            frames_skipped_background: 0,
            clock_secs: 1_760_000_000,
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
                (process_mobile_client_commands, drive_mobile_redraw_gate).chain(),
            );
    }
}

fn process_mobile_client_commands(
    mut queue: ResMut<MobileClientCommandQueue>,
    mut state: ResMut<MobileClientState>,
    mut transport_res: ResMut<MobileTransportResource>,
    mut composer: ResMut<ComposerSurface>,
    mut gate: ResMut<IdleRedrawGate>,
) {
    while let Some(cmd) = queue.pending.pop_front() {
        match cmd {
            MobileClientCommand::SetConnectivity(conn) => {
                state.auth_device.set_connectivity(conn);
                gate.request_redraw(RedrawReason::StreamDelta);
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
                        state.errors.push(err.to_string());
                        continue;
                    }
                };
                match transport_res.transport.verify_sign_in(&email, &token, now) {
                    Ok(verified) => {
                        // Clean handoff: clear any visible trends/records/composer before entering SignedIn.
                        state.visible_health_trends.clear();
                        state.visible_records.clear();
                        let _ = composer.apply(ComposerAction::SelectAll);
                        let _ = composer.apply(ComposerAction::Backspace);
                        state.auth_device.complete_sign_in(verified);
                        gate.request_redraw(RedrawReason::StreamDelta);
                    }
                    Err(err) => {
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
            }
            MobileClientCommand::SignOut { device_id } => {
                if let Some(dev_id) = device_id {
                    let _ = state.push_registry.revoke_device(&dev_id);
                }
                // Clear all visible trends, records, and composer buffer immediately (`SignInHoldsNothing`).
                state.visible_health_trends.clear();
                state.visible_records.clear();
                let _ = composer.apply(ComposerAction::SelectAll);
                let _ = composer.apply(ComposerAction::Backspace);

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
                let mut all_ok = true;
                for sess in &owed_sessions {
                    if let Err(err) = transport_res.transport.revoke_session(sess) {
                        state.errors.push(err.to_string());
                        all_ok = false;
                    }
                }
                if all_ok {
                    let mut dummy = ef_mobile_domain::MobileAuthAuthority::new();
                    let _ = state.auth_device.flush_owed_sign_outs(&mut dummy);
                }
                gate.request_redraw(RedrawReason::StreamDelta);
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
                let MobileClientScreen::SignedIn { session_id, .. } =
                    state.auth_device.screen().clone()
                else {
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
        }
    }
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
