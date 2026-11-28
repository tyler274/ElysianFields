//! Standalone, product-isolated native Bevy `0.20.1` Curie Mobile desktop application.
//!
//! Strictly isolated from `ef-clinic-*` crates (enforced by `ef-boundaries`) and never
//! imports native OS health bridges directly (all health data flows through `MobileClientTransport`).

#![cfg(not(target_arch = "wasm32"))]

pub mod config;
pub mod forms;
pub mod smoke;
pub mod state;
pub mod ui;
pub mod worker;

use std::path::PathBuf;

use bevy::app::AppExit;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk};
use bevy::window::{Window, WindowPlugin, WindowResolution};
use bevy::winit::WinitSettings;
use ef_mobile_client::CurieMobileClientPlugin;
use ef_ui_catalog::{ChromeProduct, DesktopPrefsStore, UiLocale, UiTheme};

pub use config::{
    ConfigError, DEFAULT_DEMO_EMAIL, LaunchMode, MOBILE_WINDOW_HEIGHT, MOBILE_WINDOW_TITLE,
    MOBILE_WINDOW_WIDTH, MobileDesktopConfig, MobilePrefOverrides, PreferenceMode, RunTarget,
    parse_server_addr,
};
pub use forms::{
    ClipboardBuffer, FormParseError, MobileInputFocus, parse_dob_iso, parse_email,
    parse_invite_credential,
};
pub use smoke::{
    MobileDesktopSmokeReport, SmokeError, build_headless_mobile_desktop_app, run_headless_smoke,
};
pub use state::{
    CRISIS_LINE_911, CRISIS_LINE_988, CRISIS_LINE_SAMHSA, DEMO_DOB_DAY, DEMO_DOB_MONTH,
    DEMO_DOB_YEAR, DesktopAction, DesktopActionQueue, MobileDesktopUiState, MobileModeKind,
    OwedRecoveryBannerState, next_locale,
};
pub use ui::{
    ActionButton, ButtonStyle, CardSurfaceNode, ComposerBoxNode, CrisisBannerNode,
    CrisisBannerPalette, FLUSH_OWED_SIGN_OUTS_LABEL, FormInputBoxNode, MirroredRowNode,
    MobileDesktopUiPlugin, OwedRecoveryBannerNode, RootCanvasNode, TextSlot, TextTone,
};
pub use worker::{
    LiveServerWorker, LiveWorkerRequest, LiveWorkerResponse, PendingWorkerState,
    WorkerDispatchError, WorkerOperation,
};

#[derive(Resource)]
struct ScreenshotCaptureState {
    path: PathBuf,
    frames_waited: u32,
    requested: bool,
    completed: bool,
}

/// Launches the real GPU-windowed Bevy `0.20.1` Curie Mobile desktop application (`420x860`).
pub fn run_windowed_app(config: &MobileDesktopConfig, screenshot_path: Option<PathBuf>) {
    let mut effective_config = config.clone();
    let prefs_store = if effective_config.should_use_prefs() {
        DesktopPrefsStore::for_product(ChromeProduct::Mobile)
    } else {
        DesktopPrefsStore::disabled()
    };
    if prefs_store.is_enabled() {
        match prefs_store.load() {
            Ok(Some(saved)) => effective_config.apply_saved_preferences(saved),
            Ok(None) => {}
            Err(err) => {
                eprintln!("ef-mobile-desktop warning: ignoring unreadable preferences: {err}");
            }
        }
    }

    let (ui_state, transport_res) = MobileDesktopUiState::from_config(&effective_config);

    let mut app = App::new();
    app.add_plugins((
        DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: MOBILE_WINDOW_TITLE.to_owned(),
                resolution: WindowResolution::new(
                    MOBILE_WINDOW_WIDTH as u32,
                    MOBILE_WINDOW_HEIGHT as u32,
                ),
                resizable: true,
                ..default()
            }),
            ..default()
        }),
        CurieMobileClientPlugin,
        MobileDesktopUiPlugin,
    ))
    .insert_resource(prefs_store)
    .insert_resource(transport_res)
    .insert_resource(ui_state)
    .add_systems(Startup, spawn_mobile_camera);

    if let LaunchMode::LiveServer { addr, .. } = effective_config.mode {
        app.insert_resource(LiveServerWorker::spawn(addr));
    }

    app.world_mut().resource_mut::<UiLocale>().current = effective_config.locale;
    app.world_mut()
        .resource_mut::<UiTheme>()
        .set_mode(effective_config.theme);

    if let Some(path) = screenshot_path {
        app.insert_resource(WinitSettings::game())
            .insert_resource(ScreenshotCaptureState {
                path,
                frames_waited: 0,
                requested: false,
                completed: false,
            })
            .add_systems(Update, drive_screenshot_capture_system);
    } else {
        app.insert_resource(WinitSettings::desktop_app());
    }

    app.run();
}

fn spawn_mobile_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}

fn drive_screenshot_capture_system(
    mut commands: Commands,
    mut capture: ResMut<ScreenshotCaptureState>,
    mut exit_writer: MessageWriter<AppExit>,
) {
    if capture.completed {
        exit_writer.write(AppExit::Success);
        return;
    }
    if !capture.requested {
        capture.frames_waited = capture.frames_waited.saturating_add(1);
        if capture.frames_waited >= 12 {
            capture.requested = true;
            let target_path = capture.path.clone();
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(target_path))
                .observe(
                    |_captured: On<ScreenshotCaptured>,
                     mut capture_state: ResMut<ScreenshotCaptureState>| {
                        capture_state.completed = true;
                    },
                );
        }
    }
}
