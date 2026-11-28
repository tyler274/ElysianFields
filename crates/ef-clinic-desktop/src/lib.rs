//! Curie Clinic native Bevy `0.20.1` desktop application.
//!
//! Runs both interactively on Linux/macOS/Windows via `WinitSettings::desktop_app()`
//! and deterministically in `--headless-smoke` mode for CI qualification.

pub mod clipboard;
pub mod config;
pub mod demo;
pub mod smoke;
pub mod ui;
pub mod worker;
pub mod workflows;

pub use clipboard::{
    ClipboardError, ClipboardPort, DEFAULT_CLIPBOARD_HISTORY_LIMIT, DesktopClipboard,
    InMemoryClipboard, SystemClipboardAdapter,
};
pub use config::{
    DesktopConfigError, DesktopLaunchConfig, DesktopPrefOverrides, ExecutionMode, LaunchBackend,
    PreferenceMode, ZoomLevel, parse_server_socket_addr,
};
pub use demo::{
    ConnectionMode, DemoSeedBundle, DesktopCitationCard, DesktopCitationCatalog, DesktopInputFocus,
    DesktopPatientEntry, DesktopStatusBanner, DesktopTranscriptEntry, DesktopWorkspaceState,
    STREAMING_DEMO_DRAFT, STREAMING_DEMO_STEP, bootstrap_clinic_session, build_demo_seed,
    create_transport_for_backend, fetch_live_whoami,
};
pub use smoke::{HeadlessSmokeError, HeadlessSmokeReport, SmokeCheckOutcome, run_headless_smoke};
pub use ui::{
    ClinicDesktopUiPlugin, DesktopActionButton, DesktopScreenshotConfig, DesktopUiEvent,
    DesktopUiSlot,
};
pub use worker::{
    LivePromptOutcome, LivePromptRequest, LivePromptWorker, LiveReplyCard, LiveReplyOrigin,
    LiveSubmissionState, LiveWorkerCommand, TodayStandingUpdate,
};
pub use workflows::{ClinicWorkflowState, ClinicWorkspaceTab, LiveIssuedInvite, LiveTodaySnapshot};

use thiserror::Error;

/// Top-level error returned by [`run_desktop_app`].
#[derive(Debug, Error)]
pub enum DesktopLaunchError {
    #[error(transparent)]
    Config(#[from] DesktopConfigError),
    #[error(transparent)]
    Smoke(#[from] HeadlessSmokeError),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("windowed desktop mode is unavailable on wasm32 targets")]
    UnsupportedTarget,
}

/// Entry point executing either `--headless-smoke` or the windowed GPU Bevy `0.20.1` desktop app.
pub fn run_desktop_app(config: DesktopLaunchConfig) -> Result<(), DesktopLaunchError> {
    match &config.mode {
        ExecutionMode::HeadlessSmoke => {
            let report = run_headless_smoke(&config)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        ExecutionMode::Windowed { screenshot_path } => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                run_windowed_gpu_app(&config, screenshot_path.clone())
            }
            #[cfg(target_arch = "wasm32")]
            {
                let _ = screenshot_path;
                Err(DesktopLaunchError::UnsupportedTarget)
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn run_windowed_gpu_app(
    config: &DesktopLaunchConfig,
    screenshot_path: Option<std::path::PathBuf>,
) -> Result<(), DesktopLaunchError> {
    use bevy::prelude::*;
    use bevy::window::{Window, WindowPlugin};
    use bevy::winit::WinitSettings;
    use ef_clinic_client::{
        ClientBootConfig, CurieClinicClientPlugin, SharedBrowserSessionHub, SharedLocaleStorage,
    };
    use ef_core::TabId;
    use ef_ui_catalog::{ChromeProduct, DesktopPrefsStore};

    let mut effective_config = config.clone();
    let prefs_store = if effective_config.should_use_prefs() {
        DesktopPrefsStore::for_product(ChromeProduct::Clinic)
    } else {
        DesktopPrefsStore::disabled()
    };
    if prefs_store.is_enabled() {
        match prefs_store.load() {
            Ok(Some(saved)) => effective_config.apply_saved_preferences(saved),
            Ok(None) => {}
            Err(err) => {
                eprintln!("ef-clinic-desktop warning: ignoring unreadable preferences: {err}");
            }
        }
    }

    let (transport, conn_mode, email, token, patients, citation_catalog) =
        create_transport_for_backend(&effective_config.backend);
    let initial_citations = patients
        .first()
        .map(|p| citation_catalog.cards_for(&p.workspace_id))
        .unwrap_or_default();

    let boot_path = format!(
        "/elysianfields/?lang={}",
        effective_config.locale.bcp47_tag()
    );
    let boot_config = ClientBootConfig::from_location(&boot_path).map_err(|err| {
        DesktopLaunchError::Smoke(HeadlessSmokeError::InvariantFailed(format!(
            "boot path error: {err}"
        )))
    })?;
    let tab_id = TabId::new("desktop-window-tab").map_err(|err| {
        DesktopLaunchError::Smoke(HeadlessSmokeError::InvariantFailed(format!(
            "tab id error: {err}"
        )))
    })?;

    let initial_transcript = patients
        .first()
        .map(|p| {
            vec![DesktopTranscriptEntry {
                speaker: "Curie Summary".to_owned(),
                body: format!("{} - {}", p.display_name, p.summary_line),
            }]
        })
        .unwrap_or_default();

    let winit_settings = if screenshot_path.is_some() {
        WinitSettings::continuous()
    } else {
        WinitSettings::desktop_app()
    };

    let live_worker = match &effective_config.backend {
        LaunchBackend::Server { socket_addr, .. } => {
            LivePromptWorker::spawn(*socket_addr, std::time::Duration::from_secs(10))
        }
        LaunchBackend::Demo => LivePromptWorker::disabled(),
    };
    let clipboard = if screenshot_path.is_some() {
        DesktopClipboard::in_memory()
    } else {
        DesktopClipboard::system()
    };

    let mut ws_state = DesktopWorkspaceState {
        connection_mode: conn_mode,
        patients,
        active_patient_index: 0,
        citations: initial_citations,
        selected_citation_index: 0,
        transcript: initial_transcript,
        streaming_draft: None,
        last_failed_prompt: None,
        status_message: String::new(),
        status_override: None,
        status_banner: DesktopStatusBanner::DefaultReady,
        input_focus: DesktopInputFocus::Composer,
        patient_search_query: String::new(),
        search_highlight_index: 0,
        live_submission: LiveSubmissionState::Idle,
        next_live_request_id: 1,
        shortcuts_modal_open: false,
        zoom_percent: effective_config.zoom.percent(),
        frames_rendered_ui: 0,
        prompts_submitted: 0,
    };
    ws_state.apply_preview_config(&effective_config);

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Elysian Fields — Curie Clinic".to_owned(),
            resolution: (1280, 820).into(),
            ..default()
        }),
        ..default()
    }))
    .insert_resource(winit_settings)
    .insert_resource(prefs_store)
    .insert_resource(live_worker)
    .insert_resource(clipboard)
    .insert_resource(transport)
    .insert_resource(citation_catalog)
    .insert_resource(ws_state)
    .insert_resource(ClinicWorkflowState::seeded_demo(
        effective_config.initial_view,
    ))
    .add_plugins(CurieClinicClientPlugin::new(
        tab_id,
        boot_config,
        SharedBrowserSessionHub::new(None),
        SharedLocaleStorage::default(),
    ))
    .add_plugins(ClinicDesktopUiPlugin::new(
        effective_config.theme,
        effective_config.zoom,
        screenshot_path,
    ))
    .add_systems(Startup, spawn_desktop_camera);

    // Queue initial sign-in/workspace bootstrap actions so the live window boots populated.
    if let Some(clinician_email) = email {
        app.insert_resource(PendingWindowBootstrap {
            email: clinician_email,
            explicit_token: token,
            stage: WindowBootstrapStage::BootAndRequest,
        });
        app.add_systems(Update, run_pending_window_bootstrap);
    }

    let _ = app.run();
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn_desktop_camera(mut commands: bevy::prelude::Commands) {
    commands.spawn(bevy::prelude::Camera2d);
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowBootstrapStage {
    BootAndRequest,
    VerifyAndOpen,
    SyncRosterAndOpen,
    Done,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(bevy::prelude::Resource)]
struct PendingWindowBootstrap {
    email: String,
    explicit_token: Option<String>,
    stage: WindowBootstrapStage,
}

#[cfg(not(target_arch = "wasm32"))]
fn run_pending_window_bootstrap(
    mut pending: bevy::prelude::ResMut<PendingWindowBootstrap>,
    mut actions: bevy::prelude::MessageWriter<ef_clinic_client::ClinicActionMessage>,
    session: bevy::prelude::Res<ef_clinic_client::ClinicSessionShell>,
    roster: bevy::prelude::Res<ef_clinic_client::TodayRosterView>,
    mut ws_state: bevy::prelude::ResMut<DesktopWorkspaceState>,
) {
    match pending.stage {
        WindowBootstrapStage::BootAndRequest => {
            actions.write(ef_clinic_client::ClinicActionMessage(
                ef_clinic_client::ClinicAction::BootTab,
            ));
            if let Some(tok) = pending.explicit_token.clone() {
                actions.write(ef_clinic_client::ClinicActionMessage(
                    ef_clinic_client::ClinicAction::VerifySignInToken {
                        email: pending.email.clone(),
                        token: tok,
                        entry: ef_clinic_client::SignInEntryMethod::DirectForm,
                    },
                ));
                pending.stage = WindowBootstrapStage::SyncRosterAndOpen;
            } else {
                actions.write(ef_clinic_client::ClinicActionMessage(
                    ef_clinic_client::ClinicAction::SubmitSignInEmail {
                        email: pending.email.clone(),
                    },
                ));
                pending.stage = WindowBootstrapStage::VerifyAndOpen;
            }
        }
        WindowBootstrapStage::VerifyAndOpen => {
            if let ef_clinic_client::SignInFormStatus::LinkRequested {
                proof_token: Some(tok),
                ..
            } = &session.sign_in_form
            {
                actions.write(ef_clinic_client::ClinicActionMessage(
                    ef_clinic_client::ClinicAction::VerifySignInToken {
                        email: pending.email.clone(),
                        token: tok.clone(),
                        entry: ef_clinic_client::SignInEntryMethod::DirectForm,
                    },
                ));
                pending.stage = WindowBootstrapStage::SyncRosterAndOpen;
            } else {
                pending.stage = WindowBootstrapStage::Done;
            }
        }
        WindowBootstrapStage::SyncRosterAndOpen => {
            if ws_state.patients.is_empty()
                && let ef_clinic_domain::workspace::PatientRosterState::Loaded { patients, .. } =
                    &roster.state
            {
                ws_state.patients = patients
                    .iter()
                    .map(|p| DesktopPatientEntry {
                        workspace_id: p.patient_workspace_id.clone(),
                        display_name: p.display_name.clone(),
                        summary_line: format!("Workspace {}", p.patient_workspace_id.as_str()),
                    })
                    .collect();
            }
            if let Some(first) = ws_state.patients.first() {
                actions.write(ef_clinic_client::ClinicActionMessage(
                    ef_clinic_client::ClinicAction::OpenPatientWorkspace {
                        workspace_id: first.workspace_id.clone(),
                        chat_id: None,
                    },
                ));
            }
            pending.stage = WindowBootstrapStage::Done;
        }
        WindowBootstrapStage::Done => {}
    }
}
