//! Headless Bevy `0.20.1` smoke-test runner for `ef-mobile-desktop --headless-smoke`.

use bevy::prelude::*;
use ef_connected_care::{ConnectedCareState, DobVerificationOutcome};
use ef_mobile_client::{CurieMobileClientPlugin, MobileClientState};
use ef_mobile_domain::MobileClientScreen;
use ef_ui_catalog::{
    A11yProjection, CatalogA11ySyncStatus, ComposerAction, ComposerSurface, Locale, ThemeMode,
    UiLocale, UiTheme, verify_theme_wcag_aa,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::{LaunchMode, MobileDesktopConfig};
use crate::state::{
    DesktopAction, DesktopActionQueue, MobileDesktopUiState, OwedRecoveryBannerState,
    queue_owed_demo_preview,
};
use crate::ui::{CrisisBannerNode, MirroredRowNode, MobileDesktopUiPlugin, RootCanvasNode};
use crate::worker::{LiveServerWorker, PendingWorkerState};

/// Structured JSON summary emitted by `ef-mobile-desktop --headless-smoke`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MobileDesktopSmokeReport {
    pub app: String,
    pub mode: String,
    pub locales_verified: Vec<String>,
    pub rtl_mirrored_locales: Vec<String>,
    pub themes_verified: Vec<String>,
    pub crisis_banner_count: usize,
    pub offline_owed_sign_outs_observed: usize,
    pub owed_sign_outs_after_flush: usize,
    pub connected_care_status: String,
    pub sent_doctor_envelopes: usize,
    pub a11y_projected_nodes: usize,
    pub a11y_duplicate_nodes: usize,
}

/// Typed failure returned if any headless smoke invariant fails.
#[derive(Debug, Error)]
pub enum SmokeError {
    #[error("smoke invariant failed: {0}")]
    Invariant(String),
}

/// Builds a headless Bevy `App` hosting [`CurieMobileClientPlugin`] and [`MobileDesktopUiPlugin`].
pub fn build_headless_mobile_desktop_app(config: &MobileDesktopConfig) -> App {
    let (ui_state, transport_res) = MobileDesktopUiState::from_config(config);

    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        CurieMobileClientPlugin,
        MobileDesktopUiPlugin,
    ))
    .insert_resource(transport_res)
    .insert_resource(ui_state);

    if let LaunchMode::LiveServer { addr, .. } = config.mode {
        app.insert_resource(LiveServerWorker::spawn(addr));
    }

    app.world_mut().resource_mut::<UiLocale>().current = config.locale;
    app.world_mut()
        .resource_mut::<UiTheme>()
        .set_mode(config.theme);
    queue_owed_demo_preview(&mut app, config);

    app
}

fn dispatch_action(app: &mut App, action: DesktopAction) {
    app.world_mut()
        .resource_mut::<DesktopActionQueue>()
        .push(action);
    // Two ticks: first translates DesktopAction -> MobileClientCommand; second processes transport + syncs UI/a11y.
    app.update();
    app.update();
}

/// Runs the full headless Bevy verification suite covering all 12 locales, RTL mirroring,
/// Light/Dark WCAG 2.1 AA themes, crisis-first banner, offline sign-out queueing,
/// Connected Care DOB verify & explicit Share, For My Doctor send, and 0 duplicate AccessKit IDs.
pub fn run_headless_smoke(
    config: &MobileDesktopConfig,
) -> Result<MobileDesktopSmokeReport, SmokeError> {
    let mut app = build_headless_mobile_desktop_app(config);
    app.update();
    app.update();

    // 0. Verify deterministic preview CLI flags (`--focus`, `--owed-demo`).
    if app.world().resource::<MobileDesktopUiState>().active_focus != config.initial_focus {
        return Err(SmokeError::Invariant(format!(
            "--focus {:?} did not initialize active_focus",
            config.initial_focus
        )));
    }
    if config.owed_demo && matches!(config.mode, LaunchMode::Demo) {
        let mobile = app.world().resource::<MobileClientState>();
        let banner = OwedRecoveryBannerState::from_runtime(
            mobile.auth_device.connectivity(),
            mobile.auth_device.owed_sign_outs().len(),
            PendingWorkerState::Idle,
        );
        if !matches!(
            banner,
            OwedRecoveryBannerState::OnlineFlushPending { owed_count: 1, .. }
        ) {
            return Err(SmokeError::Invariant(format!(
                "--owed-demo expected OnlineFlushPending(1), got {banner:?}"
            )));
        }
        // Clear seeded demo owed sign-out before running step 4's clean sign-in/offline cycle.
        dispatch_action(&mut app, DesktopAction::FlushOwedSignOuts);
    }

    // 1. Verify Crisis-First Banner is spawned.
    let crisis_banner_count = app
        .world_mut()
        .query_filtered::<Entity, With<CrisisBannerNode>>()
        .iter(app.world())
        .count();
    if crisis_banner_count != 1 {
        return Err(SmokeError::Invariant(format!(
            "expected exactly 1 CrisisBannerNode, found {crisis_banner_count}"
        )));
    }

    // 2. Cycle all 12 locales and verify RTL FlexDirection::RowReverse for Hebrew & Arabic.
    let mut locales_verified = Vec::with_capacity(Locale::ALL.len());
    let mut rtl_mirrored_locales = Vec::new();
    for locale in Locale::ALL {
        dispatch_action(&mut app, DesktopAction::SetLocale(locale));
        let expected_dir = locale.row_flex_direction();
        let row_dirs: Vec<FlexDirection> = app
            .world_mut()
            .query_filtered::<&Node, With<MirroredRowNode>>()
            .iter(app.world())
            .map(|n| n.flex_direction)
            .collect();
        if row_dirs.is_empty() || row_dirs.iter().any(|d| *d != expected_dir) {
            return Err(SmokeError::Invariant(format!(
                "locale {} expected row direction {expected_dir:?}, got {row_dirs:?}",
                locale.bcp47_tag()
            )));
        }
        if locale.is_rtl() {
            rtl_mirrored_locales.push(locale.bcp47_tag().to_owned());
        }
        locales_verified.push(locale.bcp47_tag().to_owned());
    }

    // Restore requested initial locale after verifying all 12.
    dispatch_action(&mut app, DesktopAction::SetLocale(config.locale));

    // 3. Verify BOTH Dark and Light themes meet WCAG 2.1 AA and update root BackgroundColor.
    let mut themes_verified = Vec::new();
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        verify_theme_wcag_aa(mode).map_err(|violations| {
            SmokeError::Invariant(format!(
                "WCAG 2.1 AA theme check failed for {mode:?}: {violations:?}"
            ))
        })?;
        dispatch_action(&mut app, DesktopAction::SetTheme(mode));
        let expected_bg = app
            .world()
            .resource::<UiTheme>()
            .tokens
            .canvas_bg
            .to_bevy_color();
        let root_bg = app
            .world_mut()
            .query_filtered::<&BackgroundColor, With<RootCanvasNode>>()
            .iter(app.world())
            .next()
            .copied();
        if root_bg.map(|b| b.0) != Some(expected_bg) {
            return Err(SmokeError::Invariant(format!(
                "root background did not update for theme {mode:?}"
            )));
        }
        themes_verified.push(format!("{mode:?}"));
    }
    dispatch_action(&mut app, DesktopAction::SetTheme(config.theme));

    // 4. Standalone Account Sign-In -> Offline Sign-Out (owed queue) -> Reconnect & Flush -> Sign back in.
    dispatch_action(&mut app, DesktopAction::SignIn);
    if !matches!(
        app.world()
            .resource::<MobileClientState>()
            .auth_device
            .screen(),
        MobileClientScreen::SignedIn { .. }
    ) {
        return Err(SmokeError::Invariant(
            "expected SignedIn screen after SignIn action".to_owned(),
        ));
    }

    dispatch_action(&mut app, DesktopAction::ToggleConnectivity);
    dispatch_action(&mut app, DesktopAction::OfflineSignOut);
    let offline_owed = app
        .world()
        .resource::<MobileClientState>()
        .auth_device
        .owed_sign_outs()
        .len();
    if offline_owed != 1 {
        return Err(SmokeError::Invariant(format!(
            "expected 1 queued owed sign-out while offline, got {offline_owed}"
        )));
    }

    dispatch_action(&mut app, DesktopAction::ToggleConnectivity);
    dispatch_action(&mut app, DesktopAction::FlushOwedSignOuts);
    let owed_after_flush = app
        .world()
        .resource::<MobileClientState>()
        .auth_device
        .owed_sign_outs()
        .len();
    if owed_after_flush != 0 {
        return Err(SmokeError::Invariant(format!(
            "expected 0 owed sign-outs after online flush, got {owed_after_flush}"
        )));
    }

    // Sign back in for Connected Care & For My Doctor flows.
    dispatch_action(&mut app, DesktopAction::SignIn);

    // 5. Connected Care: Verify DOB (never auto-shares) -> Explicit Share consent -> Active.
    let (connected_care_status, sent_doctor_envelopes) = if app
        .world()
        .resource::<MobileDesktopUiState>()
        .invite_credential
        .is_some()
    {
        dispatch_action(&mut app, DesktopAction::VerifyDob);
        let dob_ok = matches!(
            app.world().resource::<MobileClientState>().last_dob_outcome,
            Some(DobVerificationOutcome::Verified { .. })
        );
        if !dob_ok {
            return Err(SmokeError::Invariant(
                "DOB verification did not return Verified".to_owned(),
            ));
        }
        // Must NOT auto-share before explicit Share button.
        let already_connected = app
            .world()
            .resource::<MobileClientState>()
            .last_connection_standing
            .as_ref()
            .is_some_and(|s| s.state == ConnectedCareState::Connected);
        if already_connected {
            return Err(SmokeError::Invariant(
                "Connected Care must never auto-share before explicit Share action".to_owned(),
            ));
        }

        dispatch_action(&mut app, DesktopAction::ShareConnectedCare);
        let is_active = app
            .world()
            .resource::<MobileClientState>()
            .last_connection_standing
            .as_ref()
            .is_some_and(|s| {
                s.state == ConnectedCareState::Connected && s.active_connection_id.is_some()
            });
        if !is_active {
            return Err(SmokeError::Invariant(
                "Connected Care did not become Connected after explicit Share".to_owned(),
            ));
        }

        // 6. For My Doctor: compose curated note in ComposerSurface -> explicit Review & Send.
        let _ =
            app.world_mut()
                .resource_mut::<ComposerSurface>()
                .apply(ComposerAction::InsertText(
                    "Home BP averaged 122/78 mmHg this week; feeling steady.".to_owned(),
                ));
        dispatch_action(&mut app, DesktopAction::ReviewAndSendDoctorNote);
        let sent = app
            .world()
            .resource::<MobileClientState>()
            .sent_envelopes
            .len();
        if sent != 1 {
            return Err(SmokeError::Invariant(format!(
                "expected 1 sent For My Doctor envelope, got {sent}"
            )));
        }
        ("Active".to_owned(), sent)
    } else {
        ("NoInviteLoaded".to_owned(), 0)
    };

    // 7. Verify native AccessKit projection has 0 duplicate IDs and 0 sync errors.
    app.update();
    let sync_errors = &app.world().resource::<CatalogA11ySyncStatus>().errors;
    if !sync_errors.is_empty() {
        return Err(SmokeError::Invariant(format!(
            "AccessKit projection sync errors: {sync_errors:?}"
        )));
    }
    let audit = app
        .world()
        .resource::<A11yProjection>()
        .verify_zero_duplicate_nodes()
        .map_err(|errs| {
            SmokeError::Invariant(format!("AccessKit duplicate node violations: {errs:?}"))
        })?;

    Ok(MobileDesktopSmokeReport {
        app: "ef-mobile-desktop".to_owned(),
        mode: app
            .world()
            .resource::<MobileDesktopUiState>()
            .mode_label
            .clone(),
        locales_verified,
        rtl_mirrored_locales,
        themes_verified,
        crisis_banner_count,
        offline_owed_sign_outs_observed: offline_owed,
        owed_sign_outs_after_flush: owed_after_flush,
        connected_care_status,
        sent_doctor_envelopes,
        a11y_projected_nodes: audit.total_nodes,
        a11y_duplicate_nodes: audit.duplicated_nodes,
    })
}
