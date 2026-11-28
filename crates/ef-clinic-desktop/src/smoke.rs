//! Deterministic headless Bevy desktop smoke verification (`--headless-smoke`).

use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use ef_clinic_client::{
    ClientBootConfig, CurieClinicClientPlugin, SharedBrowserSessionHub, SharedLocaleStorage,
};
use ef_core::TabId;
use ef_ui_catalog::{
    A11yProjection, Locale, ShortcutRouter, TextScale, ThemeMode, ThemeTokens, UiLocale, UiTheme,
    verify_theme_wcag_aa,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::{DesktopLaunchConfig, ZoomLevel};
use crate::demo::{DesktopWorkspaceState, bootstrap_clinic_session, create_transport_for_backend};
use crate::ui::{ClinicDesktopUiPlugin, DesktopUiEvent, DesktopUiSlot};

/// Explicit typed outcome for each individual headless smoke check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SmokeCheckOutcome {
    Passed,
    Failed,
}

impl SmokeCheckOutcome {
    #[must_use]
    pub const fn from_passed(ok: bool) -> Self {
        if ok { Self::Passed } else { Self::Failed }
    }
}

/// Structured JSON report printed by `ef-clinic-desktop --headless-smoke`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeadlessSmokeReport {
    pub app: String,
    pub passed: bool,
    pub connection_mode: String,
    pub initial_locale: String,
    pub dark_wcag_aa_pairs: usize,
    pub light_wcag_aa_pairs: usize,
    pub theme_toggle: SmokeCheckOutcome,
    pub zoom_200: SmokeCheckOutcome,
    pub rtl_hebrew_row_reverse: SmokeCheckOutcome,
    pub rtl_arabic_row_reverse: SmokeCheckOutcome,
    pub locales_cycled: usize,
    pub patients_count: usize,
    pub prompts_submitted: u64,
    pub privacy_cover: SmokeCheckOutcome,
    pub shortcuts_overlay: SmokeCheckOutcome,
    pub a11y_nodes: usize,
    pub duplicate_a11y_nodes: usize,
}

/// Typed failure emitted if any headless desktop invariant regresses.
#[derive(Debug, Error)]
pub enum HeadlessSmokeError {
    #[error("WCAG 2.1 AA verification failed for {mode:?}: {detail}")]
    WcagContrastFailed { mode: ThemeMode, detail: String },
    #[error("desktop smoke invariant failed: {0}")]
    InvariantFailed(String),
}

/// Runs the real Bevy desktop `App` headlessly across all interactive transitions and returns
/// a verified [`HeadlessSmokeReport`].
pub fn run_headless_smoke(
    config: &DesktopLaunchConfig,
) -> Result<HeadlessSmokeReport, HeadlessSmokeError> {
    let dark_pairs = verify_theme_wcag_aa(ThemeMode::Dark).map_err(|violations| {
        HeadlessSmokeError::WcagContrastFailed {
            mode: ThemeMode::Dark,
            detail: format!("{} violation(s)", violations.len()),
        }
    })?;
    let light_pairs = verify_theme_wcag_aa(ThemeMode::Light).map_err(|violations| {
        HeadlessSmokeError::WcagContrastFailed {
            mode: ThemeMode::Light,
            detail: format!("{} violation(s)", violations.len()),
        }
    })?;

    let (transport, conn_mode, email, token, patients, citation_catalog) =
        create_transport_for_backend(&config.backend);
    let initial_citations = patients
        .first()
        .map(|p| citation_catalog.cards_for(&p.workspace_id))
        .unwrap_or_default();

    let boot_path = format!("/elysianfields/?lang={}", config.locale.bcp47_tag());
    let boot_config = ClientBootConfig::from_location(&boot_path)
        .map_err(|err| HeadlessSmokeError::InvariantFailed(format!("boot path error: {err}")))?;
    let tab_id = TabId::new("desktop-smoke-tab")
        .map_err(|err| HeadlessSmokeError::InvariantFailed(format!("tab id error: {err}")))?;

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .insert_resource(transport)
        .insert_resource(citation_catalog)
        .insert_resource(DesktopWorkspaceState {
            connection_mode: conn_mode.clone(),
            patients,
            active_patient_index: 0,
            citations: initial_citations,
            selected_citation_index: 0,
            transcript: Vec::new(),
            status_message: "Curie Clinic Desktop initialized.".to_owned(),
            shortcuts_modal_open: false,
            zoom_percent: config.zoom.percent(),
            frames_rendered_ui: 0,
            prompts_submitted: 0,
        })
        .add_plugins(CurieClinicClientPlugin::new(
            tab_id,
            boot_config,
            SharedBrowserSessionHub::new(None),
            SharedLocaleStorage::default(),
        ))
        .add_plugins(ClinicDesktopUiPlugin::new(config.theme, config.zoom, None));

    bootstrap_clinic_session(&mut app, email, token);

    // 1. Toggle Dark -> Light and verify root BackgroundColor matches WCAG AA Light token.
    dispatch_ui(&mut app, DesktopUiEvent::SetTheme(ThemeMode::Dark));
    dispatch_ui(&mut app, DesktopUiEvent::ToggleTheme);
    let root_bg_after_toggle = query_root_bg(&mut app)?;
    let theme_toggle_verified = app.world().resource::<UiTheme>().mode == ThemeMode::Light
        && root_bg_after_toggle == ThemeTokens::LIGHT.canvas_bg.to_bevy_color();
    if !theme_toggle_verified {
        return Err(HeadlessSmokeError::InvariantFailed(
            "Dark -> Light theme toggle did not update root canvas BackgroundColor".to_owned(),
        ));
    }

    // 2. Zoom to 200% (`TextScale::DOUBLE`).
    dispatch_ui(&mut app, DesktopUiEvent::SetZoom(ZoomLevel::Pct200));
    let zoom_200_verified = app.world().resource::<UiTheme>().text_scale == TextScale::DOUBLE;
    if !zoom_200_verified {
        return Err(HeadlessSmokeError::InvariantFailed(
            "200% zoom did not update UiTheme text_scale to TextScale::DOUBLE".to_owned(),
        ));
    }

    // 3. Cycle all 7 primary scripts (including Hebrew & Arabic RTL) verifying RowReverse mirroring.
    let mut rtl_hebrew_row_reverse = false;
    let mut rtl_arabic_row_reverse = false;
    let test_locales = [
        Locale::En,
        Locale::He,
        Locale::Ar,
        Locale::ZhHans,
        Locale::Ja,
        Locale::Vi,
        Locale::Hi,
    ];
    for loc in test_locales {
        dispatch_ui(&mut app, DesktopUiEvent::SetLocale(loc));
        let cols_dir = query_workspace_flex_direction(&mut app)?;
        if loc == Locale::He {
            rtl_hebrew_row_reverse = cols_dir == FlexDirection::RowReverse;
        } else if loc == Locale::Ar {
            rtl_arabic_row_reverse = cols_dir == FlexDirection::RowReverse;
        } else if cols_dir != FlexDirection::Row {
            return Err(HeadlessSmokeError::InvariantFailed(format!(
                "LTR locale {loc:?} unexpectedly mirrored to {cols_dir:?}"
            )));
        }
    }
    if !rtl_hebrew_row_reverse || !rtl_arabic_row_reverse {
        return Err(HeadlessSmokeError::InvariantFailed(format!(
            "RTL mirroring failed: he={rtl_hebrew_row_reverse}, ar={rtl_arabic_row_reverse}"
        )));
    }
    dispatch_ui(&mut app, DesktopUiEvent::SetLocale(config.locale));

    // 4. Select second patient (when available), type into ComposerSurface, and submit prompt.
    let patients_count = app
        .world()
        .resource::<DesktopWorkspaceState>()
        .patients
        .len();
    if patients_count > 1 {
        dispatch_ui(&mut app, DesktopUiEvent::SelectPatient(1));
        dispatch_ui(&mut app, DesktopUiEvent::SelectPatient(0));
    }
    dispatch_ui(
        &mut app,
        DesktopUiEvent::TypeText("Summarize HbA1c 6.8% and active medications.".to_owned()),
    );
    dispatch_ui(&mut app, DesktopUiEvent::SubmitPrompt);
    let prompts_submitted = app
        .world()
        .resource::<DesktopWorkspaceState>()
        .prompts_submitted;
    if prompts_submitted == 0 {
        return Err(HeadlessSmokeError::InvariantFailed(
            "composer prompt submission did not increment prompts_submitted".to_owned(),
        ));
    }

    // 5. Privacy Cover modal on -> verify Display::Flex & router active -> dismiss -> Display::None.
    dispatch_ui(&mut app, DesktopUiEvent::TogglePrivacyCover);
    let privacy_open = app
        .world()
        .resource::<ShortcutRouter>()
        .is_privacy_cover_active()
        && query_slot_display(&mut app, DesktopUiSlot::PrivacyModalBackdrop)? == Display::Flex;
    dispatch_ui(&mut app, DesktopUiEvent::DismissOverlays);
    let privacy_closed = !app
        .world()
        .resource::<ShortcutRouter>()
        .is_privacy_cover_active()
        && query_slot_display(&mut app, DesktopUiSlot::PrivacyModalBackdrop)? == Display::None;
    let privacy_cover_verified = privacy_open && privacy_closed;
    if !privacy_cover_verified {
        return Err(HeadlessSmokeError::InvariantFailed(format!(
            "privacy cover cycle failed (open={privacy_open}, closed={privacy_closed})"
        )));
    }

    // 6. Shortcuts Help modal open -> close.
    dispatch_ui(&mut app, DesktopUiEvent::ToggleShortcuts);
    let shortcuts_open =
        query_slot_display(&mut app, DesktopUiSlot::ShortcutsModalBackdrop)? == Display::Flex;
    dispatch_ui(&mut app, DesktopUiEvent::DismissOverlays);
    let shortcuts_closed =
        query_slot_display(&mut app, DesktopUiSlot::ShortcutsModalBackdrop)? == Display::None;
    let shortcuts_overlay_verified = shortcuts_open && shortcuts_closed;
    if !shortcuts_overlay_verified {
        return Err(HeadlessSmokeError::InvariantFailed(format!(
            "shortcuts overlay cycle failed (open={shortcuts_open}, closed={shortcuts_closed})"
        )));
    }

    // 7. Verify single-owner AccessKit projection has 0 duplicate node IDs.
    let audit = app
        .world()
        .resource::<A11yProjection>()
        .verify_zero_duplicate_nodes()
        .map_err(|errs| {
            HeadlessSmokeError::InvariantFailed(format!(
                "AccessKit projection reported {} error(s): {errs:?}",
                errs.len()
            ))
        })?;

    Ok(HeadlessSmokeReport {
        app: "ef-clinic-desktop".to_owned(),
        passed: true,
        connection_mode: conn_mode.badge_text(),
        initial_locale: app
            .world()
            .resource::<UiLocale>()
            .current
            .bcp47_tag()
            .to_owned(),
        dark_wcag_aa_pairs: dark_pairs.len(),
        light_wcag_aa_pairs: light_pairs.len(),
        theme_toggle: SmokeCheckOutcome::from_passed(theme_toggle_verified),
        zoom_200: SmokeCheckOutcome::from_passed(zoom_200_verified),
        rtl_hebrew_row_reverse: SmokeCheckOutcome::from_passed(rtl_hebrew_row_reverse),
        rtl_arabic_row_reverse: SmokeCheckOutcome::from_passed(rtl_arabic_row_reverse),
        locales_cycled: test_locales.len(),
        patients_count,
        prompts_submitted,
        privacy_cover: SmokeCheckOutcome::from_passed(privacy_cover_verified),
        shortcuts_overlay: SmokeCheckOutcome::from_passed(shortcuts_overlay_verified),
        a11y_nodes: audit.total_nodes,
        duplicate_a11y_nodes: audit.duplicated_nodes,
    })
}

fn dispatch_ui(app: &mut App, event: DesktopUiEvent) {
    app.world_mut().write_message(event);
    app.update();
    app.update();
}

fn query_root_bg(app: &mut App) -> Result<Color, HeadlessSmokeError> {
    let mut q = app
        .world_mut()
        .query::<(&DesktopUiSlot, &BackgroundColor)>();
    q.iter(app.world())
        .find(|(slot, _)| **slot == DesktopUiSlot::RootContainer)
        .map(|(_, bg)| bg.0)
        .ok_or_else(|| HeadlessSmokeError::InvariantFailed("RootContainer not found".to_owned()))
}

fn query_workspace_flex_direction(app: &mut App) -> Result<FlexDirection, HeadlessSmokeError> {
    let mut q = app.world_mut().query::<(&DesktopUiSlot, &Node)>();
    q.iter(app.world())
        .find(|(slot, _)| **slot == DesktopUiSlot::WorkspaceColumns)
        .map(|(_, node)| node.flex_direction)
        .ok_or_else(|| HeadlessSmokeError::InvariantFailed("WorkspaceColumns not found".to_owned()))
}

fn query_slot_display(app: &mut App, target: DesktopUiSlot) -> Result<Display, HeadlessSmokeError> {
    let mut q = app.world_mut().query::<(&DesktopUiSlot, &Node)>();
    q.iter(app.world())
        .find(|(slot, _)| **slot == target)
        .map(|(_, node)| node.display)
        .ok_or_else(|| {
            HeadlessSmokeError::InvariantFailed(format!("DesktopUiSlot {target:?} not found"))
        })
}
