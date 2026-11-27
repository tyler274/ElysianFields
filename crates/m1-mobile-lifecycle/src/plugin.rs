//! Headless Bevy 0.20.1 plugin wiring the typed mobile lifecycle domain modules
//! and platform adapter traits without entangling domain state with ECS entities.

use std::collections::VecDeque;
use std::fmt;

use bevy::prelude::*;

use crate::deep_link::{DeepLinkError, DeepLinkResolution, DeepLinkRoute};
use crate::health::{
    HealthAccessAdapter, HealthCoordinator, HealthError, HealthMetricKind, HealthUiOutcome,
};
use crate::lifecycle::{
    AppLifecycleEvent, AppLifecycleState, LifecycleController, LifecycleTransitionError,
    RenderTickOutcome,
};
use crate::notifications::{NotificationPayload, NotificationRouter};
use crate::secure_storage::{SecretValue, SecureKey, SecureStorageAdapter, SecureStorageError};
use crate::state_restore::{
    LaunchMode, SavedScreenState, StatePersistenceAdapter, StatePersistenceError,
};
use crate::viewport::{ViewportInsets, ViewportLayoutError, ViewportSpec};

/// Authentication slot status visible to the UI shell without ever exposing raw tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthSessionSlot {
    Unauthenticated,
    SignInTokenStoredInKeychain,
}

/// Typed operational error recorded when a mobile command fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MobileRuntimeError {
    Lifecycle(LifecycleTransitionError),
    StatePersistence(StatePersistenceError),
    DeepLink(DeepLinkError),
    Health(HealthError),
    SecureStorage(SecureStorageError),
    Viewport(ViewportLayoutError),
}

impl fmt::Display for MobileRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lifecycle(err) => write!(f, "lifecycle error: {err}"),
            Self::StatePersistence(err) => write!(f, "state persistence error: {err}"),
            Self::DeepLink(err) => write!(f, "deep-link error: {err}"),
            Self::Health(err) => write!(f, "health adapter error: {err}"),
            Self::SecureStorage(err) => write!(f, "secure storage error: {err}"),
            Self::Viewport(err) => write!(f, "viewport layout error: {err}"),
        }
    }
}

impl std::error::Error for MobileRuntimeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Lifecycle(err) => Some(err),
            Self::StatePersistence(err) => Some(err),
            Self::DeepLink(err) => Some(err),
            Self::Health(err) => Some(err),
            Self::SecureStorage(err) => Some(err),
            Self::Viewport(err) => Some(err),
        }
    }
}

/// Commands dispatched by platform adapters or UI actions into the Bevy runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MobileCommand {
    Lifecycle(AppLifecycleEvent),
    OpenDeepLink {
        raw_uri: String,
        launch_mode: LaunchMode,
    },
    TapNotification(NotificationPayload),
    UpdateDraft {
        text: String,
    },
    UpdateScroll {
        offset_px: u32,
    },
    RequestHealthMetric {
        metric: HealthMetricKind,
    },
    UpdateViewport(ViewportSpec),
    PersistScreenState,
}

/// FIFO command queue consumed by the Bevy `Update` schedule.
#[derive(Resource, Debug, Default)]
pub struct MobileCommandQueue {
    pending: VecDeque<MobileCommand>,
}

impl MobileCommandQueue {
    /// Enqueues a mobile platform command for the next Bevy update tick.
    pub fn push(&mut self, command: MobileCommand) {
        self.pending.push_back(command);
    }

    fn pop(&mut self) -> Option<MobileCommand> {
        self.pending.pop_front()
    }
}

/// Bevy resource wrapping the authoritative [`LifecycleController`].
#[derive(Resource, Debug)]
pub struct MobileLifecycleResource {
    controller: LifecycleController,
    last_tick_outcome: Option<RenderTickOutcome>,
}

impl MobileLifecycleResource {
    /// Creates a lifecycle resource initialized in foreground state.
    #[must_use]
    pub const fn new_foreground() -> Self {
        Self {
            controller: LifecycleController::new_foreground(),
            last_tick_outcome: None,
        }
    }

    /// Creates a lifecycle resource starting from a terminated process state.
    #[must_use]
    pub const fn new_terminated() -> Self {
        Self {
            controller: LifecycleController::new_terminated(),
            last_tick_outcome: None,
        }
    }

    /// Returns a read-only view of the underlying lifecycle controller.
    #[must_use]
    pub const fn controller(&self) -> &LifecycleController {
        &self.controller
    }

    /// Returns the most recent render-gate outcome.
    #[must_use]
    pub const fn last_tick_outcome(&self) -> Option<RenderTickOutcome> {
        self.last_tick_outcome
    }
}

/// Bevy resource holding pure mobile domain state (independent of rendering entities).
#[derive(Resource, Debug)]
pub struct MobileAppShellResource {
    pub screen_state: SavedScreenState,
    pub auth_slot: AuthSessionSlot,
    pub last_health_outcome: Option<HealthUiOutcome>,
    pub viewport: ViewportInsets,
    pub errors: Vec<MobileRuntimeError>,
    pub diagnostic_events: Vec<String>,
}

impl Default for MobileAppShellResource {
    fn default() -> Self {
        Self {
            screen_state: SavedScreenState::default(),
            auth_slot: AuthSessionSlot::Unauthenticated,
            last_health_outcome: None,
            viewport: ViewportInsets::default(),
            errors: Vec::new(),
            diagnostic_events: Vec::new(),
        }
    }
}

/// Bevy resource holding the injected platform adapter implementations.
#[derive(Resource)]
pub struct PlatformAdaptersResource {
    state_store: Box<dyn StatePersistenceAdapter>,
    health_adapter: Box<dyn HealthAccessAdapter>,
    secure_storage: Box<dyn SecureStorageAdapter>,
}

impl PlatformAdaptersResource {
    /// Constructs a platform adapter bundle for a mobile runtime instance.
    #[must_use]
    pub fn new(
        state_store: Box<dyn StatePersistenceAdapter>,
        health_adapter: Box<dyn HealthAccessAdapter>,
        secure_storage: Box<dyn SecureStorageAdapter>,
    ) -> Self {
        Self {
            state_store,
            health_adapter,
            secure_storage,
        }
    }

    /// Returns a read-only reference to the secure storage adapter.
    #[must_use]
    pub fn secure_storage(&self) -> &dyn SecureStorageAdapter {
        self.secure_storage.as_ref()
    }

    /// Returns a read-only reference to the health adapter.
    #[must_use]
    pub fn health_adapter(&self) -> &dyn HealthAccessAdapter {
        self.health_adapter.as_ref()
    }

    /// Returns a read-only reference to the state persistence adapter.
    #[must_use]
    pub fn state_store(&self) -> &dyn StatePersistenceAdapter {
        self.state_store.as_ref()
    }
}

/// Bevy `0.20.1` plugin registering mobile lifecycle, restoration, deep-link,
/// notification, health, secure-storage, and viewport systems.
pub struct MobileLifecyclePrototypePlugin;

impl Plugin for MobileLifecyclePrototypePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MobileCommandQueue>()
            .init_resource::<MobileAppShellResource>()
            .add_systems(
                Update,
                (process_mobile_commands, run_render_gate_system).chain(),
            );
    }
}

fn process_mobile_commands(
    mut queue: ResMut<MobileCommandQueue>,
    mut lifecycle: ResMut<MobileLifecycleResource>,
    mut shell: ResMut<MobileAppShellResource>,
    mut adapters: ResMut<PlatformAdaptersResource>,
) {
    while let Some(command) = queue.pop() {
        match command {
            MobileCommand::Lifecycle(event) => {
                handle_lifecycle_event(event, &mut lifecycle, &mut shell, &mut adapters);
            }
            MobileCommand::OpenDeepLink {
                raw_uri,
                launch_mode,
            } => {
                handle_deep_link(&raw_uri, launch_mode, &mut shell, &mut adapters);
            }
            MobileCommand::TapNotification(payload) => {
                handle_notification(&payload, &mut lifecycle, &mut shell);
            }
            MobileCommand::UpdateDraft { text } => {
                shell.screen_state.draft_text = text;
            }
            MobileCommand::UpdateScroll { offset_px } => {
                shell.screen_state.scroll_offset_px = offset_px;
            }
            MobileCommand::RequestHealthMetric { metric } => {
                match HealthCoordinator::sync_metric(adapters.health_adapter.as_mut(), metric) {
                    Ok(outcome) => {
                        shell
                            .diagnostic_events
                            .push(format!("health_sync:{metric:?}:{outcome:?}"));
                        shell.last_health_outcome = Some(outcome);
                    }
                    Err(err) => {
                        shell.errors.push(MobileRuntimeError::Health(err));
                    }
                }
            }
            MobileCommand::UpdateViewport(spec) => match ViewportInsets::compute(&spec) {
                Ok(insets) => {
                    shell.viewport = insets;
                    shell.diagnostic_events.push(format!(
                        "viewport:{:?}:{}x{}",
                        insets.orientation,
                        insets.safe_content_rect.width,
                        insets.safe_content_rect.height
                    ));
                }
                Err(err) => {
                    shell.errors.push(MobileRuntimeError::Viewport(err));
                }
            },
            MobileCommand::PersistScreenState => {
                if let Err(err) = adapters.state_store.save_state(&shell.screen_state) {
                    shell.errors.push(MobileRuntimeError::StatePersistence(err));
                }
            }
        }
    }
}

fn handle_lifecycle_event(
    event: AppLifecycleEvent,
    lifecycle: &mut MobileLifecycleResource,
    shell: &mut MobileAppShellResource,
    adapters: &mut PlatformAdaptersResource,
) {
    let previous = lifecycle.controller.state();
    match lifecycle.controller.transition(event) {
        Ok(next) => {
            shell
                .diagnostic_events
                .push(format!("lifecycle:{previous:?}->{next:?}"));
            match next {
                AppLifecycleState::BackgroundPaused | AppLifecycleState::ProcessTerminated => {
                    if let Err(err) = adapters.state_store.save_state(&shell.screen_state) {
                        shell.errors.push(MobileRuntimeError::StatePersistence(err));
                    }
                }
                AppLifecycleState::Foreground if event == AppLifecycleEvent::ColdLaunch => {
                    match adapters.state_store.load_state() {
                        Ok(Some(restored)) => {
                            shell.screen_state = restored;
                            shell
                                .diagnostic_events
                                .push("state_restored_after_cold_launch".to_owned());
                        }
                        Ok(None) => {}
                        Err(err) => {
                            shell.errors.push(MobileRuntimeError::StatePersistence(err));
                        }
                    }
                }
                AppLifecycleState::Foreground => {}
            }
        }
        Err(err) => {
            shell.errors.push(MobileRuntimeError::Lifecycle(err));
        }
    }
}

fn handle_deep_link(
    raw_uri: &str,
    launch_mode: LaunchMode,
    shell: &mut MobileAppShellResource,
    adapters: &mut PlatformAdaptersResource,
) {
    let parsed = match DeepLinkRoute::parse(raw_uri) {
        Ok(route) => route,
        Err(err) => {
            // Never log the raw URI in case it carried a malformed secret query parameter.
            shell
                .diagnostic_events
                .push(format!("deep_link_rejected:{err}"));
            shell.errors.push(MobileRuntimeError::DeepLink(err));
            return;
        }
    };

    match parsed.resolve(launch_mode) {
        DeepLinkResolution::SignIn { screen, token } => {
            let secret = match SecretValue::from_utf8(token.expose_secret()) {
                Ok(secret) => secret,
                Err(err) => {
                    shell.errors.push(MobileRuntimeError::SecureStorage(err));
                    return;
                }
            };
            if let Err(err) = adapters
                .secure_storage
                .store(SecureKey::SignInExchangeToken, &secret)
            {
                shell.errors.push(MobileRuntimeError::SecureStorage(err));
                return;
            }
            shell.screen_state.route = screen;
            shell.auth_slot = AuthSessionSlot::SignInTokenStoredInKeychain;
            shell
                .diagnostic_events
                .push(format!("deep_link_sign_in_routed:{launch_mode:?}"));
        }
        DeepLinkResolution::Record { screen, record_id } => {
            shell.screen_state.route = screen;
            shell.screen_state.active_record_id = Some(record_id.clone());
            shell.diagnostic_events.push(format!(
                "deep_link_record_routed:{}:{launch_mode:?}",
                record_id.as_str()
            ));
        }
        DeepLinkResolution::Vitals { screen } => {
            shell.screen_state.route = screen;
            shell.screen_state.active_record_id = None;
            shell
                .diagnostic_events
                .push(format!("deep_link_vitals_routed:{launch_mode:?}"));
        }
    }
}

fn handle_notification(
    payload: &NotificationPayload,
    lifecycle: &mut MobileLifecycleResource,
    shell: &mut MobileAppShellResource,
) {
    let needs_cold_launch = payload.launch_mode == LaunchMode::ColdStart
        && lifecycle.controller.state() == AppLifecycleState::ProcessTerminated;
    if needs_cold_launch {
        let transition_result = lifecycle
            .controller
            .transition(AppLifecycleEvent::ColdLaunch);
        if let Err(err) = transition_result {
            shell.errors.push(MobileRuntimeError::Lifecycle(err));
            return;
        }
    }

    let routed = NotificationRouter::route(payload);
    shell.screen_state.route = routed.screen;
    shell.screen_state.active_record_id = routed.active_record_id;
    shell.diagnostic_events.push(format!(
        "notification_routed:{}:{:?}",
        payload.id.as_str(),
        payload.launch_mode
    ));
}

fn run_render_gate_system(mut lifecycle: ResMut<MobileLifecycleResource>) {
    let outcome = lifecycle.controller.tick_render_frame();
    lifecycle.last_tick_outcome = Some(outcome);
}
