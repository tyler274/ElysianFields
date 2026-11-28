//! Native Bevy `0.20.1` Curie Clinic desktop GUI plugin, input handling, and reactive layout.
//!
//! Features:
//! - Top App Bar with 12-locale switcher, WCAG 2.1 AA Dark/Light toggle, 100/150/200% zoom,
//!   Privacy Cover button (`Ctrl/Cmd+Shift+L`), and Shortcuts (`?`) button.
//! - 3-Column responsive workspace (`Node.flex_direction = locale.row_flex_direction()`) for genuine
//!   Hebrew (`he`) and Arabic (`ar`) RTL mirroring.
//! - Multiline clinical composer wired to `ef_ui_catalog::ComposerSurface` (`KeyboardInput`,
//!   `Shift+Enter`, `Enter` submit, `Backspace`, cluster arrows, `Ctrl/Cmd+Z` undo/redo).
//! - Privacy Cover (`ZIndex(100)`) and Shortcuts Help (`ZIndex(90)`) modals.
//! - Native `bevy::a11y::AccessibilityNode` components without duplicating `CurieClinicClientPlugin`'s
//!   `A11yNodeMarker` projection (`0` duplicate AccessKit node IDs).

use std::path::PathBuf;

use bevy::a11y::AccessibilityNode;
use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyCode, KeyboardInput};
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use ef_clinic_client::{
    ActiveWorkspaceView, ClinicAction, ClinicActionMessage, ClinicSessionShell,
};
use ef_core::IdempotencyKey;
use ef_ui_catalog::{
    ChromeProduct, ComposerAction, ComposerSurface, DesktopPreferences, DesktopPrefsStore,
    IdleRedrawGate, Locale, ModifierChord, RedrawReason, ShortcutInput, ShortcutKey,
    ShortcutRouter, ThemeMode, ThemeTokens, UiKey, UiLocale, UiTheme, localized_chrome, translate,
};

use crate::config::ZoomLevel;
use crate::demo::{DesktopCitationCatalog, DesktopTranscriptEntry, DesktopWorkspaceState};

/// Typed interactive button action component (zero string dispatch).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopActionButton {
    CycleLocale,
    ToggleTheme,
    CycleZoom,
    TogglePrivacyCover,
    ToggleShortcuts,
    SelectPatient(usize),
    SelectCitation(usize),
    SubmitPrompt,
    DismissPrivacyCover,
    DismissShortcuts,
}

/// Typed semantic UI slot marker for reactive theme, locale, zoom, and layout updates.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopUiSlot {
    RootContainer,
    TopBar,
    BrandTitle,
    LocaleButton,
    LocaleButtonLabel,
    ThemeButton,
    ThemeButtonLabel,
    ZoomButton,
    ZoomButtonLabel,
    PrivacyButton,
    PrivacyButtonLabel,
    ShortcutsButton,
    ShortcutsButtonLabel,
    WorkspaceColumns,
    RosterPanel,
    RosterHeading,
    ConnectionBadge,
    PatientButton(usize),
    PatientButtonLabel(usize),
    TranscriptPanel,
    TranscriptHeading,
    TranscriptScrollArea,
    TranscriptBody,
    ComposerLabel,
    ComposerBox,
    ComposerText,
    SendButton,
    SendButtonLabel,
    EvidencePanel,
    EvidenceHeading,
    EvidenceScrollArea,
    CitationButton(usize),
    CitationButtonLabel(usize),
    CitationDetailBody,
    StatusBar,
    StatusText,
    PrivacyModalBackdrop,
    PrivacyModalCard,
    PrivacyModalTitle,
    PrivacyModalHint,
    PrivacyDismissButton,
    PrivacyDismissLabel,
    ShortcutsModalBackdrop,
    ShortcutsModalCard,
    ShortcutsModalTitle,
    ShortcutsModalBody,
    ShortcutsDismissButton,
    ShortcutsDismissLabel,
}

/// Typed UI command message processed inside the Bevy desktop update loop.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum DesktopUiEvent {
    CycleLocale,
    SetLocale(Locale),
    ToggleTheme,
    SetTheme(ThemeMode),
    CycleZoom,
    SetZoom(ZoomLevel),
    SelectPatient(usize),
    SelectCitation(usize),
    TypeText(String),
    SubmitPrompt,
    TogglePrivacyCover,
    ToggleShortcuts,
    DismissOverlays,
    ScrollTranscript(i32),
    ScrollEvidence(i32),
}

/// Optional screenshot capture configuration for live GPU window runs.
#[derive(Resource, Debug, Clone, Default)]
pub struct DesktopScreenshotConfig {
    pub path: Option<PathBuf>,
    pub captured: bool,
    pub saved_done: bool,
}

/// Bevy `0.20.1` UI plugin for the native Curie Clinic desktop shell.
pub struct ClinicDesktopUiPlugin {
    pub initial_theme: ThemeMode,
    pub initial_zoom: ZoomLevel,
    pub screenshot_path: Option<PathBuf>,
}

impl ClinicDesktopUiPlugin {
    #[must_use]
    pub const fn new(
        initial_theme: ThemeMode,
        initial_zoom: ZoomLevel,
        screenshot_path: Option<PathBuf>,
    ) -> Self {
        Self {
            initial_theme,
            initial_zoom,
            screenshot_path,
        }
    }
}

impl Plugin for ClinicDesktopUiPlugin {
    fn build(&self, app: &mut App) {
        let initial_ui_theme = UiTheme::new(self.initial_theme, self.initial_zoom.text_scale());
        app.insert_resource(initial_ui_theme)
            .insert_resource(DesktopScreenshotConfig {
                path: self.screenshot_path.clone(),
                captured: false,
                saved_done: false,
            })
            .add_message::<DesktopUiEvent>()
            .add_message::<KeyboardInput>()
            .add_message::<MouseWheel>()
            .add_systems(Startup, spawn_clinic_desktop_ui_system)
            .add_systems(
                Update,
                (
                    handle_desktop_button_presses_system,
                    handle_desktop_keyboard_input_system,
                    handle_desktop_scroll_wheel_system,
                    process_desktop_events_system,
                    sync_desktop_ui_visuals_system,
                )
                    .chain(),
            );

        #[cfg(not(target_arch = "wasm32"))]
        {
            app.add_systems(Update, capture_screenshot_and_exit_system);
        }
    }
}

fn spawn_clinic_desktop_ui_system(
    mut commands: Commands,
    ui_locale: Res<UiLocale>,
    ui_theme: Res<UiTheme>,
    ws_state: Res<DesktopWorkspaceState>,
) {
    let locale = ui_locale.current;
    let tokens = ui_theme.tokens;
    let scale = ui_theme.text_scale.factor();
    let chrome = localized_chrome(locale, ChromeProduct::Clinic);

    let mut root_a11y = accesskit::Node::new(accesskit::Role::Main);
    root_a11y.set_label(chrome.app_brand.clone());

    commands
        .spawn((
            DesktopUiSlot::RootContainer,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(14.0)),
                row_gap: Val::Px(10.0),
                ..default()
            },
            BackgroundColor(tokens.canvas_bg.to_bevy_color()),
            AccessibilityNode(root_a11y),
        ))
        .with_children(|root| {
            // 1. Top App Bar
            root.spawn((
                DesktopUiSlot::TopBar,
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    padding: UiRect::axes(Val::Px(14.0), Val::Px(10.0)),
                    column_gap: Val::Px(8.0),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(tokens.surface_bg.to_bevy_color()),
                BorderColor::all(tokens.focus_ring.to_bevy_color()),
            ))
            .with_children(|bar| {
                bar.spawn((
                    DesktopUiSlot::BrandTitle,
                    Text::new(chrome.app_brand.clone()),
                    TextFont {
                        font_size: FontSize::Px(20.0 * scale),
                        ..default()
                    },
                    TextColor(tokens.text_primary.to_bevy_color()),
                ));

                bar.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(8.0),
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|controls| {
                    spawn_toolbar_button(
                        controls,
                        DesktopActionButton::CycleLocale,
                        DesktopUiSlot::LocaleButton,
                        DesktopUiSlot::LocaleButtonLabel,
                        format!("{}: {}", chrome.language_label, locale.native_autonym()),
                        tokens,
                        scale,
                    );
                    spawn_toolbar_button(
                        controls,
                        DesktopActionButton::ToggleTheme,
                        DesktopUiSlot::ThemeButton,
                        DesktopUiSlot::ThemeButtonLabel,
                        theme_button_label(locale, ui_theme.mode),
                        tokens,
                        scale,
                    );
                    spawn_toolbar_button(
                        controls,
                        DesktopActionButton::CycleZoom,
                        DesktopUiSlot::ZoomButton,
                        DesktopUiSlot::ZoomButtonLabel,
                        format!(
                            "{}: {}%",
                            translate(locale, UiKey::LabelZoom),
                            ws_state.zoom_percent
                        ),
                        tokens,
                        scale,
                    );
                    spawn_toolbar_button(
                        controls,
                        DesktopActionButton::TogglePrivacyCover,
                        DesktopUiSlot::PrivacyButton,
                        DesktopUiSlot::PrivacyButtonLabel,
                        translate(locale, UiKey::PrivacyCoverButton).to_owned(),
                        tokens,
                        scale,
                    );
                    spawn_toolbar_button(
                        controls,
                        DesktopActionButton::ToggleShortcuts,
                        DesktopUiSlot::ShortcutsButton,
                        DesktopUiSlot::ShortcutsButtonLabel,
                        format!("? {}", translate(locale, UiKey::ShortcutsDialogTitle)),
                        tokens,
                        scale,
                    );
                });
            });

            // 2. 3-Column Workspace (mirrors automatically via `locale.row_flex_direction()` for `he`/`ar`!)
            root.spawn((
                DesktopUiSlot::WorkspaceColumns,
                Node {
                    width: Val::Percent(100.0),
                    flex_grow: 1.0,
                    flex_direction: locale.row_flex_direction(),
                    column_gap: Val::Px(12.0),
                    min_height: Val::Px(0.0),
                    ..default()
                },
            ))
            .with_children(|cols| {
                // Column 1: Patient Roster Rail (`sidebar_edge`)
                cols.spawn((
                    DesktopUiSlot::RosterPanel,
                    Node {
                        width: Val::Percent(24.0),
                        min_width: Val::Px(220.0),
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::all(Val::Px(12.0)),
                        row_gap: Val::Px(8.0),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(tokens.surface_bg.to_bevy_color()),
                    BorderColor::all(tokens.text_secondary.to_bevy_color()),
                ))
                .with_children(|rail| {
                    rail.spawn((
                        DesktopUiSlot::RosterHeading,
                        Text::new(chrome.patient_roster.clone()),
                        TextFont {
                            font_size: FontSize::Px(16.0 * scale),
                            ..default()
                        },
                        TextColor(tokens.text_primary.to_bevy_color()),
                    ));
                    rail.spawn((
                        DesktopUiSlot::ConnectionBadge,
                        Text::new(ws_state.connection_mode.localized_badge(locale)),
                        TextFont {
                            font_size: FontSize::Px(12.0 * scale),
                            ..default()
                        },
                        TextColor(tokens.alert_info_fg.to_bevy_color()),
                    ));

                    for idx in 0..3 {
                        let label = ws_state
                            .patients
                            .get(idx)
                            .map(|p| format!("{}\n{}", p.display_name, p.summary_line))
                            .unwrap_or_default();
                        let selected = idx == ws_state.active_patient_index;
                        let bg = if selected {
                            tokens.nav_selected_bg.to_bevy_color()
                        } else {
                            tokens.input_bg.to_bevy_color()
                        };
                        let fg = if selected {
                            tokens.nav_selected_fg.to_bevy_color()
                        } else {
                            tokens.text_primary.to_bevy_color()
                        };
                        let mut btn_a11y = accesskit::Node::new(accesskit::Role::Button);
                        btn_a11y.set_label(label.clone());
                        rail.spawn((
                            DesktopActionButton::SelectPatient(idx),
                            DesktopUiSlot::PatientButton(idx),
                            Node {
                                width: Val::Percent(100.0),
                                padding: UiRect::all(Val::Px(10.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                display: if idx < ws_state.patients.len() {
                                    Display::Flex
                                } else {
                                    Display::None
                                },
                                ..default()
                            },
                            BackgroundColor(bg),
                            BorderColor::all(tokens.focus_ring.to_bevy_color()),
                            AccessibilityNode(btn_a11y),
                        ))
                        .with_children(|btn| {
                            btn.spawn((
                                DesktopUiSlot::PatientButtonLabel(idx),
                                Text::new(label),
                                TextFont {
                                    font_size: FontSize::Px(13.0 * scale),
                                    ..default()
                                },
                                TextColor(fg),
                            ));
                        });
                    }
                });

                // Column 2: Clinical Transcript & Multiline Composer
                cols.spawn((
                    DesktopUiSlot::TranscriptPanel,
                    Node {
                        flex_grow: 1.0,
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::all(Val::Px(14.0)),
                        row_gap: Val::Px(10.0),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(tokens.surface_bg.to_bevy_color()),
                    BorderColor::all(tokens.text_secondary.to_bevy_color()),
                ))
                .with_children(|center| {
                    center.spawn((
                        DesktopUiSlot::TranscriptHeading,
                        Text::new(chrome.transcript_heading.clone()),
                        TextFont {
                            font_size: FontSize::Px(16.0 * scale),
                            ..default()
                        },
                        TextColor(tokens.text_primary.to_bevy_color()),
                    ));

                    center
                        .spawn((
                            DesktopUiSlot::TranscriptScrollArea,
                            ScrollPosition::default(),
                            Node {
                                width: Val::Percent(100.0),
                                flex_grow: 1.0,
                                min_height: Val::Px(0.0),
                                flex_direction: FlexDirection::Column,
                                padding: UiRect::all(Val::Px(10.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                overflow: Overflow::scroll_y(),
                                ..default()
                            },
                            BackgroundColor(tokens.canvas_bg.to_bevy_color()),
                            BorderColor::all(tokens.text_secondary.to_bevy_color()),
                        ))
                        .with_children(|box_node| {
                            box_node.spawn((
                                DesktopUiSlot::TranscriptBody,
                                Text::new(format_transcript(&ws_state.transcript, locale)),
                                TextFont {
                                    font_size: FontSize::Px(14.0 * scale),
                                    ..default()
                                },
                                TextColor(tokens.text_primary.to_bevy_color()),
                            ));
                        });

                    center.spawn((
                        DesktopUiSlot::ComposerLabel,
                        Text::new(chrome.composer_placeholder.clone()),
                        TextFont {
                            font_size: FontSize::Px(13.0 * scale),
                            ..default()
                        },
                        TextColor(tokens.text_secondary.to_bevy_color()),
                    ));

                    let mut composer_a11y =
                        accesskit::Node::new(accesskit::Role::MultilineTextInput);
                    composer_a11y.set_label(chrome.composer_placeholder.clone());
                    center
                        .spawn((
                            DesktopUiSlot::ComposerBox,
                            Node {
                                width: Val::Percent(100.0),
                                min_height: Val::Px(74.0),
                                padding: UiRect::all(Val::Px(10.0)),
                                border: UiRect::all(Val::Px(2.0)),
                                ..default()
                            },
                            BackgroundColor(tokens.input_bg.to_bevy_color()),
                            BorderColor::all(tokens.focus_ring.to_bevy_color()),
                            AccessibilityNode(composer_a11y),
                        ))
                        .with_children(|input_box| {
                            input_box.spawn((
                                DesktopUiSlot::ComposerText,
                                Text::new(chrome.composer_placeholder.clone()),
                                TextFont {
                                    font_size: FontSize::Px(14.0 * scale),
                                    ..default()
                                },
                                TextColor(tokens.input_fg.to_bevy_color()),
                            ));
                        });

                    let mut send_a11y = accesskit::Node::new(accesskit::Role::Button);
                    send_a11y.set_label(chrome.send.clone());
                    center
                        .spawn((
                            DesktopActionButton::SubmitPrompt,
                            DesktopUiSlot::SendButton,
                            Node {
                                align_self: AlignSelf::FlexEnd,
                                padding: UiRect::axes(Val::Px(16.0), Val::Px(10.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(tokens.accent_fill.to_bevy_color()),
                            BorderColor::all(tokens.focus_ring.to_bevy_color()),
                            AccessibilityNode(send_a11y),
                        ))
                        .with_children(|btn| {
                            btn.spawn((
                                DesktopUiSlot::SendButtonLabel,
                                Text::new(chrome.send.clone()),
                                TextFont {
                                    font_size: FontSize::Px(14.0 * scale),
                                    ..default()
                                },
                                TextColor(tokens.accent_label.to_bevy_color()),
                            ));
                        });
                });

                // Column 3: Cited Evidence Rail (`citation_panel_edge`)
                cols.spawn((
                    DesktopUiSlot::EvidencePanel,
                    Node {
                        width: Val::Percent(27.0),
                        min_width: Val::Px(240.0),
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::all(Val::Px(12.0)),
                        row_gap: Val::Px(8.0),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(tokens.surface_bg.to_bevy_color()),
                    BorderColor::all(tokens.text_secondary.to_bevy_color()),
                ))
                .with_children(|rail| {
                    rail.spawn((
                        DesktopUiSlot::EvidenceHeading,
                        Text::new(chrome.evidence_heading.clone()),
                        TextFont {
                            font_size: FontSize::Px(16.0 * scale),
                            ..default()
                        },
                        TextColor(tokens.text_primary.to_bevy_color()),
                    ));

                    for idx in 0..3 {
                        let card_label = ws_state
                            .citations
                            .get(idx)
                            .map(|c| format!("{} - {}", c.label, c.title))
                            .unwrap_or_default();
                        let mut card_a11y = accesskit::Node::new(accesskit::Role::Button);
                        card_a11y.set_label(card_label.clone());
                        rail.spawn((
                            DesktopActionButton::SelectCitation(idx),
                            DesktopUiSlot::CitationButton(idx),
                            Node {
                                width: Val::Percent(100.0),
                                padding: UiRect::all(Val::Px(8.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                display: if idx < ws_state.citations.len() {
                                    Display::Flex
                                } else {
                                    Display::None
                                },
                                ..default()
                            },
                            BackgroundColor(tokens.input_bg.to_bevy_color()),
                            BorderColor::all(tokens.link_citation.to_bevy_color()),
                            AccessibilityNode(card_a11y),
                        ))
                        .with_children(|btn| {
                            btn.spawn((
                                DesktopUiSlot::CitationButtonLabel(idx),
                                Text::new(card_label),
                                TextFont {
                                    font_size: FontSize::Px(13.0 * scale),
                                    ..default()
                                },
                                TextColor(tokens.link_citation.to_bevy_color()),
                            ));
                        });
                    }

                    let detail_text = ws_state
                        .selected_citation()
                        .map(|c| format!("{}\n\n{}\n({})", c.title, c.excerpt, c.provenance))
                        .unwrap_or_else(|| {
                            translate(locale, UiKey::ClinicCitationSelectPrompt).to_owned()
                        });
                    rail.spawn((
                        DesktopUiSlot::EvidenceScrollArea,
                        ScrollPosition::default(),
                        Node {
                            width: Val::Percent(100.0),
                            flex_grow: 1.0,
                            min_height: Val::Px(0.0),
                            flex_direction: FlexDirection::Column,
                            overflow: Overflow::scroll_y(),
                            ..default()
                        },
                    ))
                    .with_children(|scroll_box| {
                        scroll_box.spawn((
                            DesktopUiSlot::CitationDetailBody,
                            Text::new(detail_text),
                            TextFont {
                                font_size: FontSize::Px(13.0 * scale),
                                ..default()
                            },
                            TextColor(tokens.text_secondary.to_bevy_color()),
                        ));
                    });
                });
            });

            // 3. Status Bar
            root.spawn((
                DesktopUiSlot::StatusBar,
                Node {
                    width: Val::Percent(100.0),
                    padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(tokens.alert_info_bg.to_bevy_color()),
                BorderColor::all(tokens.focus_ring.to_bevy_color()),
            ))
            .with_children(|bar| {
                bar.spawn((
                    DesktopUiSlot::StatusText,
                    Text::new(ws_state.status_message.clone()),
                    TextFont {
                        font_size: FontSize::Px(13.0 * scale),
                        ..default()
                    },
                    TextColor(tokens.alert_info_fg.to_bevy_color()),
                ));
            });

            // 4. Privacy Cover Modal (`ZIndex(100)`) — opaque veil concealing PHI
            let mut privacy_a11y = accesskit::Node::new(accesskit::Role::Dialog);
            privacy_a11y.set_label(translate(locale, UiKey::PrivacyCoverTitle));
            privacy_a11y.set_modal();
            root.spawn((
                DesktopUiSlot::PrivacyModalBackdrop,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    top: Val::Px(0.0),
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    display: Display::None,
                    ..default()
                },
                ZIndex(100),
                BackgroundColor(tokens.privacy_veil_bg.to_bevy_color()),
                AccessibilityNode(privacy_a11y),
            ))
            .with_children(|backdrop| {
                backdrop
                    .spawn((
                        DesktopUiSlot::PrivacyModalCard,
                        Node {
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::Center,
                            padding: UiRect::all(Val::Px(28.0)),
                            row_gap: Val::Px(14.0),
                            border: UiRect::all(Val::Px(2.0)),
                            ..default()
                        },
                        BackgroundColor(tokens.modal_bg.to_bevy_color()),
                        BorderColor::all(tokens.focus_ring.to_bevy_color()),
                    ))
                    .with_children(|card| {
                        card.spawn((
                            DesktopUiSlot::PrivacyModalTitle,
                            Text::new(translate(locale, UiKey::PrivacyCoverTitle)),
                            TextFont {
                                font_size: FontSize::Px(22.0 * scale),
                                ..default()
                            },
                            TextColor(tokens.text_primary.to_bevy_color()),
                        ));
                        card.spawn((
                            DesktopUiSlot::PrivacyModalHint,
                            Text::new(translate(locale, UiKey::PrivacyCoverHint)),
                            TextFont {
                                font_size: FontSize::Px(14.0 * scale),
                                ..default()
                            },
                            TextColor(tokens.text_secondary.to_bevy_color()),
                        ));
                        let mut dismiss_a11y = accesskit::Node::new(accesskit::Role::Button);
                        dismiss_a11y.set_label(translate(locale, UiKey::PrivacyCoverDismiss));
                        card.spawn((
                            DesktopActionButton::DismissPrivacyCover,
                            DesktopUiSlot::PrivacyDismissButton,
                            Node {
                                padding: UiRect::axes(Val::Px(18.0), Val::Px(10.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(tokens.accent_fill.to_bevy_color()),
                            BorderColor::all(tokens.focus_ring.to_bevy_color()),
                            AccessibilityNode(dismiss_a11y),
                        ))
                        .with_children(|btn| {
                            btn.spawn((
                                DesktopUiSlot::PrivacyDismissLabel,
                                Text::new(translate(locale, UiKey::PrivacyCoverDismiss)),
                                TextFont {
                                    font_size: FontSize::Px(14.0 * scale),
                                    ..default()
                                },
                                TextColor(tokens.accent_label.to_bevy_color()),
                            ));
                        });
                    });
            });

            // 5. Shortcuts Help Modal (`ZIndex(90)`)
            let mut shortcuts_a11y = accesskit::Node::new(accesskit::Role::Dialog);
            shortcuts_a11y.set_label(translate(locale, UiKey::ShortcutsDialogTitle));
            shortcuts_a11y.set_modal();
            root.spawn((
                DesktopUiSlot::ShortcutsModalBackdrop,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    top: Val::Px(0.0),
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    display: Display::None,
                    ..default()
                },
                ZIndex(90),
                BackgroundColor(Color::srgba(0.02, 0.04, 0.09, 0.85)),
                AccessibilityNode(shortcuts_a11y),
            ))
            .with_children(|backdrop| {
                backdrop
                    .spawn((
                        DesktopUiSlot::ShortcutsModalCard,
                        Node {
                            flex_direction: FlexDirection::Column,
                            padding: UiRect::all(Val::Px(24.0)),
                            row_gap: Val::Px(12.0),
                            border: UiRect::all(Val::Px(2.0)),
                            ..default()
                        },
                        BackgroundColor(tokens.modal_bg.to_bevy_color()),
                        BorderColor::all(tokens.focus_ring.to_bevy_color()),
                    ))
                    .with_children(|card| {
                        card.spawn((
                            DesktopUiSlot::ShortcutsModalTitle,
                            Text::new(translate(locale, UiKey::ShortcutsDialogTitle)),
                            TextFont {
                                font_size: FontSize::Px(20.0 * scale),
                                ..default()
                            },
                            TextColor(tokens.text_primary.to_bevy_color()),
                        ));
                        card.spawn((
                            DesktopUiSlot::ShortcutsModalBody,
                            Text::new(translate(locale, UiKey::ClinicShortcutsBody)),
                            TextFont {
                                font_size: FontSize::Px(14.0 * scale),
                                ..default()
                            },
                            TextColor(tokens.text_secondary.to_bevy_color()),
                        ));
                        let mut close_a11y = accesskit::Node::new(accesskit::Role::Button);
                        close_a11y.set_label(translate(locale, UiKey::ActionClose));
                        card.spawn((
                            DesktopActionButton::DismissShortcuts,
                            DesktopUiSlot::ShortcutsDismissButton,
                            Node {
                                align_self: AlignSelf::FlexEnd,
                                padding: UiRect::axes(Val::Px(14.0), Val::Px(8.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(tokens.accent_fill.to_bevy_color()),
                            BorderColor::all(tokens.focus_ring.to_bevy_color()),
                            AccessibilityNode(close_a11y),
                        ))
                        .with_children(|btn| {
                            btn.spawn((
                                DesktopUiSlot::ShortcutsDismissLabel,
                                Text::new(translate(locale, UiKey::ActionClose)),
                                TextFont {
                                    font_size: FontSize::Px(14.0 * scale),
                                    ..default()
                                },
                                TextColor(tokens.accent_label.to_bevy_color()),
                            ));
                        });
                    });
            });
        });
}

fn spawn_toolbar_button(
    parent: &mut ChildSpawnerCommands,
    action: DesktopActionButton,
    button_slot: DesktopUiSlot,
    label_slot: DesktopUiSlot,
    text: String,
    tokens: ThemeTokens,
    scale: f32,
) {
    let mut btn_a11y = accesskit::Node::new(accesskit::Role::Button);
    btn_a11y.set_label(text.clone());
    parent
        .spawn((
            action,
            button_slot,
            Node {
                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(tokens.input_bg.to_bevy_color()),
            BorderColor::all(tokens.focus_ring.to_bevy_color()),
            AccessibilityNode(btn_a11y),
        ))
        .with_children(|btn| {
            btn.spawn((
                label_slot,
                Text::new(text),
                TextFont {
                    font_size: FontSize::Px(13.0 * scale),
                    ..default()
                },
                TextColor(tokens.text_primary.to_bevy_color()),
            ));
        });
}

fn handle_desktop_button_presses_system(
    pressed: Query<&DesktopActionButton, Added<bevy::ui::Pressed>>,
    mut events: MessageWriter<DesktopUiEvent>,
) {
    for button in &pressed {
        let evt = match *button {
            DesktopActionButton::CycleLocale => DesktopUiEvent::CycleLocale,
            DesktopActionButton::ToggleTheme => DesktopUiEvent::ToggleTheme,
            DesktopActionButton::CycleZoom => DesktopUiEvent::CycleZoom,
            DesktopActionButton::TogglePrivacyCover => DesktopUiEvent::TogglePrivacyCover,
            DesktopActionButton::ToggleShortcuts => DesktopUiEvent::ToggleShortcuts,
            DesktopActionButton::SelectPatient(idx) => DesktopUiEvent::SelectPatient(idx),
            DesktopActionButton::SelectCitation(idx) => DesktopUiEvent::SelectCitation(idx),
            DesktopActionButton::SubmitPrompt => DesktopUiEvent::SubmitPrompt,
            DesktopActionButton::DismissPrivacyCover | DesktopActionButton::DismissShortcuts => {
                DesktopUiEvent::DismissOverlays
            }
        };
        events.write(evt);
    }
}

fn handle_desktop_keyboard_input_system(
    mut key_events: MessageReader<KeyboardInput>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    router: Res<ShortcutRouter>,
    composer: Res<ComposerSurface>,
    mut clinic_actions: MessageWriter<ClinicActionMessage>,
    mut desktop_events: MessageWriter<DesktopUiEvent>,
) {
    let (primary_down, shift_down) = match keys {
        Some(ref input) => (
            input.any_pressed([
                KeyCode::ControlLeft,
                KeyCode::ControlRight,
                KeyCode::SuperLeft,
                KeyCode::SuperRight,
            ]),
            input.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]),
        ),
        None => (false, false),
    };

    for event in key_events.read() {
        if event.state != ButtonState::Pressed {
            continue;
        }

        // Global Cmd/Ctrl+Shift+L privacy cover toggle via physical KeyCode::KeyL.
        if primary_down && shift_down && event.key_code == KeyCode::KeyL {
            desktop_events.write(DesktopUiEvent::TogglePrivacyCover);
            continue;
        }

        if event.key_code == KeyCode::Escape {
            desktop_events.write(DesktopUiEvent::DismissOverlays);
            continue;
        }

        // Ignore composer edits while privacy cover conceals PHI.
        if router.is_privacy_cover_active() {
            continue;
        }

        if primary_down && event.key_code == KeyCode::KeyZ {
            let action = if shift_down {
                ComposerAction::Redo
            } else {
                ComposerAction::Undo
            };
            clinic_actions.write(ClinicActionMessage(ClinicAction::ComposerEdit(action)));
            continue;
        }

        match event.key_code {
            KeyCode::Enter => {
                if shift_down {
                    clinic_actions.write(ClinicActionMessage(ClinicAction::ComposerEdit(
                        ComposerAction::ShiftEnterNewline,
                    )));
                } else {
                    desktop_events.write(DesktopUiEvent::SubmitPrompt);
                }
            }
            KeyCode::Backspace => {
                clinic_actions.write(ClinicActionMessage(ClinicAction::ComposerEdit(
                    ComposerAction::Backspace,
                )));
            }
            KeyCode::ArrowLeft => {
                clinic_actions.write(ClinicActionMessage(ClinicAction::ComposerEdit(
                    ComposerAction::MovePrevCluster {
                        extend_selection: shift_down,
                    },
                )));
            }
            KeyCode::ArrowRight => {
                clinic_actions.write(ClinicActionMessage(ClinicAction::ComposerEdit(
                    ComposerAction::MoveNextCluster {
                        extend_selection: shift_down,
                    },
                )));
            }
            KeyCode::PageUp => {
                if shift_down {
                    desktop_events.write(DesktopUiEvent::ScrollEvidence(-120));
                } else {
                    desktop_events.write(DesktopUiEvent::ScrollTranscript(-120));
                }
            }
            KeyCode::PageDown => {
                if shift_down {
                    desktop_events.write(DesktopUiEvent::ScrollEvidence(120));
                } else {
                    desktop_events.write(DesktopUiEvent::ScrollTranscript(120));
                }
            }
            _ => {
                if !primary_down {
                    if let Key::Character(ref ch) = event.logical_key
                        && ch.as_str() == "?"
                        && composer.text().is_empty()
                    {
                        desktop_events.write(DesktopUiEvent::ToggleShortcuts);
                        continue;
                    }
                    if let Some(ref text) = event.text {
                        let printable: String = text.chars().filter(|c| !c.is_control()).collect();
                        if !printable.is_empty() {
                            desktop_events.write(DesktopUiEvent::TypeText(printable));
                        }
                    }
                }
            }
        }
    }
}

fn handle_desktop_scroll_wheel_system(
    mut wheel_events: MessageReader<MouseWheel>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    router: Res<ShortcutRouter>,
    mut desktop_events: MessageWriter<DesktopUiEvent>,
) {
    if router.is_privacy_cover_active() {
        return;
    }
    let shift_down = keys
        .as_ref()
        .is_some_and(|input| input.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]));
    for wheel in wheel_events.read() {
        let delta_px = match wheel.unit {
            MouseScrollUnit::Line => (-wheel.y * 32.0).round() as i32,
            MouseScrollUnit::Pixel => (-wheel.y).round() as i32,
        };
        if delta_px == 0 {
            continue;
        }
        if shift_down {
            desktop_events.write(DesktopUiEvent::ScrollEvidence(delta_px));
        } else {
            desktop_events.write(DesktopUiEvent::ScrollTranscript(delta_px));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn process_desktop_events_system(
    mut events: MessageReader<DesktopUiEvent>,
    mut clinic_actions: MessageWriter<ClinicActionMessage>,
    mut ws_state: ResMut<DesktopWorkspaceState>,
    citation_catalog: Res<DesktopCitationCatalog>,
    mut ui_locale: ResMut<UiLocale>,
    mut ui_theme: ResMut<UiTheme>,
    mut composer: ResMut<ComposerSurface>,
    router: Res<ShortcutRouter>,
    session: Res<ClinicSessionShell>,
    active_workspace: Res<ActiveWorkspaceView>,
    mut gate: ResMut<IdleRedrawGate>,
    prefs_store: Option<Res<DesktopPrefsStore>>,
    mut scroll_areas: Query<(&DesktopUiSlot, &mut ScrollPosition)>,
) {
    let persist_prefs = |locale: Locale, theme: ThemeMode, zoom_percent: u16| {
        if let Some(ref store) = prefs_store
            && store.is_enabled()
        {
            let zoom = ZoomLevel::parse(&zoom_percent.to_string())
                .unwrap_or(ZoomLevel::Pct100)
                .to_desktop_zoom();
            let _ = store.save(&DesktopPreferences {
                theme,
                locale,
                zoom,
            });
        }
    };

    for evt in events.read() {
        match evt.clone() {
            DesktopUiEvent::CycleLocale => {
                let idx = Locale::ALL
                    .iter()
                    .position(|&loc| loc == ui_locale.current)
                    .unwrap_or(0);
                let next = Locale::ALL[(idx + 1) % Locale::ALL.len()];
                ui_locale.current = next;
                clinic_actions.write(ClinicActionMessage(ClinicAction::SwitchLocale(next)));
                ws_state.status_override = None;
                persist_prefs(next, ui_theme.mode, ws_state.zoom_percent);
                gate.request_redraw(RedrawReason::LocaleOrThemeChange);
            }
            DesktopUiEvent::SetLocale(next) => {
                ui_locale.current = next;
                clinic_actions.write(ClinicActionMessage(ClinicAction::SwitchLocale(next)));
                ws_state.status_override = None;
                persist_prefs(next, ui_theme.mode, ws_state.zoom_percent);
                gate.request_redraw(RedrawReason::LocaleOrThemeChange);
            }
            DesktopUiEvent::ToggleTheme => {
                let next_mode = match ui_theme.mode {
                    ThemeMode::Dark => ThemeMode::Light,
                    ThemeMode::Light => ThemeMode::Dark,
                };
                ui_theme.set_mode(next_mode);
                ws_state.status_override = None;
                persist_prefs(ui_locale.current, next_mode, ws_state.zoom_percent);
                gate.request_redraw(RedrawReason::LocaleOrThemeChange);
            }
            DesktopUiEvent::SetTheme(mode) => {
                ui_theme.set_mode(mode);
                ws_state.status_override = None;
                persist_prefs(ui_locale.current, mode, ws_state.zoom_percent);
                gate.request_redraw(RedrawReason::LocaleOrThemeChange);
            }
            DesktopUiEvent::CycleZoom => {
                let current = ZoomLevel::parse(&ws_state.zoom_percent.to_string())
                    .unwrap_or(ZoomLevel::Pct100);
                let next = current.cycle_next();
                ws_state.zoom_percent = next.percent();
                ui_theme.text_scale = next.text_scale();
                ws_state.status_override = None;
                persist_prefs(ui_locale.current, ui_theme.mode, next.percent());
                gate.request_redraw(RedrawReason::LocaleOrThemeChange);
            }
            DesktopUiEvent::SetZoom(next) => {
                ws_state.zoom_percent = next.percent();
                ui_theme.text_scale = next.text_scale();
                ws_state.status_override = None;
                persist_prefs(ui_locale.current, ui_theme.mode, next.percent());
                gate.request_redraw(RedrawReason::LocaleOrThemeChange);
            }
            DesktopUiEvent::SelectPatient(idx) => {
                if let Some(patient) = ws_state.patients.get(idx).cloned() {
                    ws_state.active_patient_index = idx;
                    ws_state.citations = citation_catalog.cards_for(&patient.workspace_id);
                    ws_state.selected_citation_index = 0;
                    ws_state.transcript = vec![DesktopTranscriptEntry {
                        speaker: "Curie Summary".to_owned(),
                        body: format!("{} - {}", patient.display_name, patient.summary_line),
                    }];
                    ws_state.status_message = format!("Opened workspace: {}", patient.display_name);
                    clinic_actions.write(ClinicActionMessage(ClinicAction::OpenPatientWorkspace {
                        workspace_id: patient.workspace_id,
                        chat_id: None,
                    }));
                    gate.request_redraw(RedrawReason::FocusTransition);
                }
            }
            DesktopUiEvent::SelectCitation(idx) => {
                if idx < ws_state.citations.len() {
                    ws_state.selected_citation_index = idx;
                    if let Some(card) = ws_state.citations.get(idx) {
                        ws_state.status_message = format!("Inspecting citation {}", card.label);
                    }
                    gate.request_redraw(RedrawReason::UserInput);
                }
            }
            DesktopUiEvent::TypeText(text) => {
                let _ = composer.apply(ComposerAction::InsertText(text));
                gate.request_redraw(RedrawReason::UserInput);
            }
            DesktopUiEvent::SubmitPrompt => {
                let prompt = composer.text().trim().to_owned();
                if prompt.is_empty() {
                    continue;
                }
                ws_state.prompts_submitted = ws_state.prompts_submitted.saturating_add(1);
                ws_state.transcript.push(DesktopTranscriptEntry {
                    speaker: "Clinician".to_owned(),
                    body: prompt.clone(),
                });

                let cited_summary = ws_state.selected_citation().map_or_else(
                    || "Reviewed active patient workspace record.".to_owned(),
                    |c| format!("Based on {}: {}", c.label, c.excerpt),
                );
                ws_state.transcript.push(DesktopTranscriptEntry {
                    speaker: "Curie".to_owned(),
                    body: cited_summary,
                });
                ws_state.status_message = format!(
                    "Submitted prompt #{} ({} chars)",
                    ws_state.prompts_submitted,
                    prompt.chars().count()
                );

                // If authenticated with an active server workspace, dispatch through CurieClinicClientPlugin too.
                if session.view_state.shows_protected_content()
                    && active_workspace.open_workspace.is_some()
                    && let Ok(key) = IdempotencyKey::new(format!(
                        "desktop-prompt-{}",
                        ws_state.prompts_submitted
                    ))
                {
                    clinic_actions.write(ClinicActionMessage(ClinicAction::SubmitMessage {
                        idempotency_key: key,
                        delivery: None,
                    }));
                } else {
                    let _ = composer.apply(ComposerAction::SelectAll);
                    let _ = composer.apply(ComposerAction::Backspace);
                }
                gate.request_redraw(RedrawReason::StreamDelta);
            }
            DesktopUiEvent::TogglePrivacyCover => {
                ws_state.shortcuts_modal_open = false;
                clinic_actions.write(ClinicActionMessage(ClinicAction::DispatchShortcut(
                    ShortcutInput::new(ShortcutKey::Char('l'), ModifierChord::PrimaryShift),
                )));
                ws_state.status_message = if router.is_privacy_cover_active() {
                    "Privacy cover dismissed.".to_owned()
                } else {
                    "Privacy cover active (PHI concealed).".to_owned()
                };
                gate.request_redraw(RedrawReason::OverlayTransition);
            }
            DesktopUiEvent::ToggleShortcuts => {
                if !router.is_privacy_cover_active() {
                    ws_state.shortcuts_modal_open = !ws_state.shortcuts_modal_open;
                    ws_state.status_message = if ws_state.shortcuts_modal_open {
                        "Shortcuts dialog opened.".to_owned()
                    } else {
                        "Shortcuts dialog closed.".to_owned()
                    };
                    gate.request_redraw(RedrawReason::OverlayTransition);
                }
            }
            DesktopUiEvent::DismissOverlays => {
                if router.is_privacy_cover_active() {
                    clinic_actions.write(ClinicActionMessage(ClinicAction::DispatchShortcut(
                        ShortcutInput::new(ShortcutKey::Char('l'), ModifierChord::PrimaryShift),
                    )));
                    ws_state.status_override = None;
                } else if ws_state.shortcuts_modal_open {
                    ws_state.shortcuts_modal_open = false;
                    ws_state.status_override = None;
                }
                gate.request_redraw(RedrawReason::OverlayTransition);
            }
            DesktopUiEvent::ScrollTranscript(delta_px) => {
                for (slot, mut pos) in &mut scroll_areas {
                    if *slot == DesktopUiSlot::TranscriptScrollArea {
                        pos.y = (pos.y + delta_px as f32).max(0.0);
                    }
                }
                gate.request_redraw(RedrawReason::UserInput);
            }
            DesktopUiEvent::ScrollEvidence(delta_px) => {
                for (slot, mut pos) in &mut scroll_areas {
                    if *slot == DesktopUiSlot::EvidenceScrollArea {
                        pos.y = (pos.y + delta_px as f32).max(0.0);
                    }
                }
                gate.request_redraw(RedrawReason::UserInput);
            }
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sync_desktop_ui_visuals_system(
    ui_locale: Res<UiLocale>,
    ui_theme: Res<UiTheme>,
    composer: Res<ComposerSurface>,
    router: Res<ShortcutRouter>,
    mut ws_state: ResMut<DesktopWorkspaceState>,
    mut nodes: Query<(
        &DesktopUiSlot,
        &mut Node,
        Option<&mut BackgroundColor>,
        Option<&mut BorderColor>,
        Option<&mut Text>,
        Option<&mut TextFont>,
        Option<&mut TextColor>,
        Option<&mut AccessibilityNode>,
    )>,
) {
    ws_state.frames_rendered_ui = ws_state.frames_rendered_ui.saturating_add(1);

    let locale = ui_locale.current;
    let tokens = ui_theme.tokens;
    let scale = ui_theme.text_scale.factor();
    let chrome = localized_chrome(locale, ChromeProduct::Clinic);
    let privacy_active = router.is_privacy_cover_active();
    let shortcuts_open = ws_state.shortcuts_modal_open && !privacy_active;

    let composer_display = if composer.display_text().is_empty() {
        chrome.composer_placeholder.clone()
    } else {
        composer.display_text()
    };

    let citation_detail = ws_state
        .selected_citation()
        .map(|c| format!("{}\n\n{}\n({})", c.title, c.excerpt, c.provenance))
        .unwrap_or_else(|| translate(locale, UiKey::ClinicCitationSelectPrompt).to_owned());

    let zoom_label = format!(
        "{}: {}%",
        translate(locale, UiKey::LabelZoom),
        ws_state.zoom_percent
    );
    let privacy_label = translate(
        locale,
        if privacy_active {
            UiKey::PrivacyCoverTitle
        } else {
            UiKey::PrivacyCoverButton
        },
    );
    let status_text = ws_state.status_override.clone().unwrap_or_else(|| {
        format!(
            "{} ({} | {} | {})",
            translate(locale, UiKey::ClinicStatusReady),
            locale.native_autonym(),
            theme_button_label(locale, ui_theme.mode),
            zoom_label
        )
    });
    ws_state.status_message = status_text.clone();

    for (slot, mut node, bg, border, text, font, text_color, a11y) in &mut nodes {
        match *slot {
            DesktopUiSlot::RootContainer => {
                set_bg(bg, tokens.canvas_bg.to_bevy_color());
                set_a11y_label(a11y, &chrome.app_brand);
            }
            DesktopUiSlot::TopBar => {
                set_bg(bg, tokens.surface_bg.to_bevy_color());
                set_border(border, tokens.focus_ring.to_bevy_color());
            }
            DesktopUiSlot::BrandTitle => {
                set_text(text, &chrome.app_brand);
                set_font(font, 20.0 * scale);
                set_color(text_color, tokens.text_primary.to_bevy_color());
            }
            DesktopUiSlot::LocaleButton => {
                let label = format!("{}: {}", chrome.language_label, locale.native_autonym());
                set_bg(bg, tokens.input_bg.to_bevy_color());
                set_border(border, tokens.focus_ring.to_bevy_color());
                set_a11y_label(a11y, &label);
            }
            DesktopUiSlot::LocaleButtonLabel => {
                set_text(
                    text,
                    &format!("{}: {}", chrome.language_label, locale.native_autonym()),
                );
                set_font(font, 13.0 * scale);
                set_color(text_color, tokens.text_primary.to_bevy_color());
            }
            DesktopUiSlot::ThemeButton => {
                let label = theme_button_label(locale, ui_theme.mode);
                set_bg(bg, tokens.input_bg.to_bevy_color());
                set_border(border, tokens.focus_ring.to_bevy_color());
                set_a11y_label(a11y, &label);
            }
            DesktopUiSlot::ThemeButtonLabel => {
                set_text(text, &theme_button_label(locale, ui_theme.mode));
                set_font(font, 13.0 * scale);
                set_color(text_color, tokens.text_primary.to_bevy_color());
            }
            DesktopUiSlot::ZoomButton => {
                set_bg(bg, tokens.input_bg.to_bevy_color());
                set_border(border, tokens.focus_ring.to_bevy_color());
                set_a11y_label(a11y, &zoom_label);
            }
            DesktopUiSlot::ZoomButtonLabel => {
                set_text(text, &zoom_label);
                set_font(font, 13.0 * scale);
                set_color(text_color, tokens.text_primary.to_bevy_color());
            }
            DesktopUiSlot::PrivacyButton => {
                set_bg(bg, tokens.input_bg.to_bevy_color());
                set_border(border, tokens.focus_ring.to_bevy_color());
                set_a11y_label(a11y, privacy_label);
            }
            DesktopUiSlot::PrivacyButtonLabel => {
                set_text(text, privacy_label);
                set_font(font, 13.0 * scale);
                set_color(text_color, tokens.text_primary.to_bevy_color());
            }
            DesktopUiSlot::ShortcutsButton => {
                let label = format!("? {}", translate(locale, UiKey::ShortcutsDialogTitle));
                set_bg(bg, tokens.input_bg.to_bevy_color());
                set_border(border, tokens.focus_ring.to_bevy_color());
                set_a11y_label(a11y, &label);
            }
            DesktopUiSlot::ShortcutsButtonLabel => {
                set_text(
                    text,
                    &format!("? {}", translate(locale, UiKey::ShortcutsDialogTitle)),
                );
                set_font(font, 13.0 * scale);
                set_color(text_color, tokens.text_primary.to_bevy_color());
            }
            DesktopUiSlot::WorkspaceColumns => {
                node.flex_direction = locale.row_flex_direction();
            }
            DesktopUiSlot::RosterPanel
            | DesktopUiSlot::TranscriptPanel
            | DesktopUiSlot::EvidencePanel => {
                set_bg(bg, tokens.surface_bg.to_bevy_color());
                set_border(border, tokens.text_secondary.to_bevy_color());
            }
            DesktopUiSlot::RosterHeading => {
                set_text(text, &chrome.patient_roster);
                set_font(font, 16.0 * scale);
                set_color(text_color, tokens.text_primary.to_bevy_color());
            }
            DesktopUiSlot::ConnectionBadge => {
                set_text(text, &ws_state.connection_mode.localized_badge(locale));
                set_font(font, 12.0 * scale);
                set_color(text_color, tokens.alert_info_fg.to_bevy_color());
            }
            DesktopUiSlot::PatientButton(idx) => {
                let visible = idx < ws_state.patients.len();
                node.display = if visible {
                    Display::Flex
                } else {
                    Display::None
                };
                let selected = idx == ws_state.active_patient_index;
                set_bg(
                    bg,
                    if selected {
                        tokens.nav_selected_bg.to_bevy_color()
                    } else {
                        tokens.input_bg.to_bevy_color()
                    },
                );
                set_border(border, tokens.focus_ring.to_bevy_color());
                if let Some(p) = ws_state.patients.get(idx) {
                    set_a11y_label(a11y, &p.display_name);
                }
            }
            DesktopUiSlot::PatientButtonLabel(idx) => {
                if let Some(p) = ws_state.patients.get(idx) {
                    set_text(text, &format!("{}\n{}", p.display_name, p.summary_line));
                }
                set_font(font, 13.0 * scale);
                let selected = idx == ws_state.active_patient_index;
                set_color(
                    text_color,
                    if selected {
                        tokens.nav_selected_fg.to_bevy_color()
                    } else {
                        tokens.text_primary.to_bevy_color()
                    },
                );
            }
            DesktopUiSlot::TranscriptHeading => {
                set_text(text, &chrome.transcript_heading);
                set_font(font, 16.0 * scale);
                set_color(text_color, tokens.text_primary.to_bevy_color());
            }
            DesktopUiSlot::TranscriptScrollArea => {
                set_bg(bg, tokens.canvas_bg.to_bevy_color());
                set_border(border, tokens.text_secondary.to_bevy_color());
            }
            DesktopUiSlot::TranscriptBody => {
                set_text(text, &format_transcript(&ws_state.transcript, locale));
                set_font(font, 14.0 * scale);
                set_color(text_color, tokens.text_primary.to_bevy_color());
            }
            DesktopUiSlot::ComposerLabel => {
                set_text(text, &chrome.composer_placeholder);
                set_font(font, 13.0 * scale);
                set_color(text_color, tokens.text_secondary.to_bevy_color());
            }
            DesktopUiSlot::ComposerBox => {
                set_bg(bg, tokens.input_bg.to_bevy_color());
                set_border(border, tokens.focus_ring.to_bevy_color());
                set_a11y_label(a11y, &chrome.composer_placeholder);
            }
            DesktopUiSlot::ComposerText => {
                set_text(text, &composer_display);
                set_font(font, 14.0 * scale);
                set_color(text_color, tokens.input_fg.to_bevy_color());
            }
            DesktopUiSlot::SendButton => {
                set_bg(bg, tokens.accent_fill.to_bevy_color());
                set_border(border, tokens.focus_ring.to_bevy_color());
                set_a11y_label(a11y, &chrome.send);
            }
            DesktopUiSlot::SendButtonLabel => {
                set_text(text, &chrome.send);
                set_font(font, 14.0 * scale);
                set_color(text_color, tokens.accent_label.to_bevy_color());
            }
            DesktopUiSlot::EvidenceHeading => {
                set_text(text, &chrome.evidence_heading);
                set_font(font, 16.0 * scale);
                set_color(text_color, tokens.text_primary.to_bevy_color());
            }
            DesktopUiSlot::EvidenceScrollArea => {}
            DesktopUiSlot::CitationButton(idx) => {
                let visible = idx < ws_state.citations.len();
                node.display = if visible {
                    Display::Flex
                } else {
                    Display::None
                };
                let selected = idx == ws_state.selected_citation_index;
                set_bg(
                    bg,
                    if selected {
                        tokens.nav_selected_bg.to_bevy_color()
                    } else {
                        tokens.input_bg.to_bevy_color()
                    },
                );
                set_border(border, tokens.link_citation.to_bevy_color());
                if let Some(c) = ws_state.citations.get(idx) {
                    set_a11y_label(a11y, &c.label);
                }
            }
            DesktopUiSlot::CitationButtonLabel(idx) => {
                if let Some(c) = ws_state.citations.get(idx) {
                    set_text(text, &format!("{} - {}", c.label, c.title));
                }
                set_font(font, 13.0 * scale);
                set_color(text_color, tokens.link_citation.to_bevy_color());
            }
            DesktopUiSlot::CitationDetailBody => {
                set_text(text, &citation_detail);
                set_font(font, 13.0 * scale);
                set_color(text_color, tokens.text_secondary.to_bevy_color());
            }
            DesktopUiSlot::StatusBar => {
                set_bg(bg, tokens.alert_info_bg.to_bevy_color());
                set_border(border, tokens.focus_ring.to_bevy_color());
            }
            DesktopUiSlot::StatusText => {
                set_text(text, &status_text);
                set_font(font, 13.0 * scale);
                set_color(text_color, tokens.alert_info_fg.to_bevy_color());
            }
            DesktopUiSlot::PrivacyModalBackdrop => {
                node.display = if privacy_active {
                    Display::Flex
                } else {
                    Display::None
                };
                set_bg(bg, tokens.privacy_veil_bg.to_bevy_color());
                set_a11y_label(a11y, translate(locale, UiKey::PrivacyCoverTitle));
            }
            DesktopUiSlot::PrivacyModalCard => {
                set_bg(bg, tokens.modal_bg.to_bevy_color());
                set_border(border, tokens.focus_ring.to_bevy_color());
            }
            DesktopUiSlot::PrivacyModalTitle => {
                set_text(text, translate(locale, UiKey::PrivacyCoverTitle));
                set_font(font, 22.0 * scale);
                set_color(text_color, tokens.text_primary.to_bevy_color());
            }
            DesktopUiSlot::PrivacyModalHint => {
                set_text(text, translate(locale, UiKey::PrivacyCoverHint));
                set_font(font, 14.0 * scale);
                set_color(text_color, tokens.text_secondary.to_bevy_color());
            }
            DesktopUiSlot::PrivacyDismissButton => {
                set_bg(bg, tokens.accent_fill.to_bevy_color());
                set_border(border, tokens.focus_ring.to_bevy_color());
                set_a11y_label(a11y, translate(locale, UiKey::PrivacyCoverDismiss));
            }
            DesktopUiSlot::PrivacyDismissLabel => {
                set_text(text, translate(locale, UiKey::PrivacyCoverDismiss));
                set_font(font, 14.0 * scale);
                set_color(text_color, tokens.accent_label.to_bevy_color());
            }
            DesktopUiSlot::ShortcutsModalBackdrop => {
                node.display = if shortcuts_open {
                    Display::Flex
                } else {
                    Display::None
                };
                set_a11y_label(a11y, translate(locale, UiKey::ShortcutsDialogTitle));
            }
            DesktopUiSlot::ShortcutsModalCard => {
                set_bg(bg, tokens.modal_bg.to_bevy_color());
                set_border(border, tokens.focus_ring.to_bevy_color());
            }
            DesktopUiSlot::ShortcutsModalTitle => {
                set_text(text, translate(locale, UiKey::ShortcutsDialogTitle));
                set_font(font, 20.0 * scale);
                set_color(text_color, tokens.text_primary.to_bevy_color());
            }
            DesktopUiSlot::ShortcutsModalBody => {
                set_text(text, translate(locale, UiKey::ClinicShortcutsBody));
                set_font(font, 14.0 * scale);
                set_color(text_color, tokens.text_secondary.to_bevy_color());
            }
            DesktopUiSlot::ShortcutsDismissButton => {
                set_bg(bg, tokens.accent_fill.to_bevy_color());
                set_border(border, tokens.focus_ring.to_bevy_color());
                set_a11y_label(a11y, translate(locale, UiKey::ActionClose));
            }
            DesktopUiSlot::ShortcutsDismissLabel => {
                set_text(text, translate(locale, UiKey::ActionClose));
                set_font(font, 14.0 * scale);
                set_color(text_color, tokens.accent_label.to_bevy_color());
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn capture_screenshot_and_exit_system(
    mut commands: Commands,
    ws_state: Res<DesktopWorkspaceState>,
    mut screenshot_cfg: ResMut<DesktopScreenshotConfig>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(path) = screenshot_cfg.path.clone() else {
        return;
    };
    if screenshot_cfg.saved_done {
        exit.write(AppExit::Success);
        return;
    }
    if !screenshot_cfg.captured && ws_state.frames_rendered_ui >= 6 {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            let _ = std::fs::create_dir_all(parent);
        }
        screenshot_cfg.captured = true;
        let save_handler = bevy::render::view::screenshot::save_to_disk(path);
        commands
            .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
            .observe(save_handler)
            .observe(
                |_trigger: On<bevy::render::view::screenshot::ScreenshotCaptured>,
                 mut cfg: ResMut<DesktopScreenshotConfig>| {
                    cfg.saved_done = true;
                },
            );
    }
}

fn theme_button_label(locale: Locale, mode: ThemeMode) -> String {
    match mode {
        ThemeMode::Dark => translate(locale, UiKey::LabelThemeDark).to_owned(),
        ThemeMode::Light => translate(locale, UiKey::LabelThemeLight).to_owned(),
    }
}

fn format_transcript(entries: &[DesktopTranscriptEntry], locale: Locale) -> String {
    if entries.is_empty() {
        return translate(locale, UiKey::ClinicTranscriptEmpty).to_owned();
    }
    entries
        .iter()
        .map(|e| format!("[{}] {}", e.speaker, e.body))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn set_bg(bg: Option<Mut<'_, BackgroundColor>>, color: Color) {
    if let Some(mut bg) = bg {
        bg.0 = color;
    }
}

fn set_border(border: Option<Mut<'_, BorderColor>>, color: Color) {
    if let Some(mut border) = border {
        *border = BorderColor::all(color);
    }
}

fn set_text(text: Option<Mut<'_, Text>>, value: &str) {
    if let Some(mut text) = text
        && text.0 != value
    {
        text.0 = value.to_owned();
    }
}

fn set_font(font: Option<Mut<'_, TextFont>>, size: f32) {
    if let Some(mut font) = font {
        #[cfg(not(target_arch = "wasm32"))]
        {
            font.font = bevy::text::FontSource::sans_serif();
        }
        font.font_size = FontSize::Px(size);
    }
}

fn set_color(color: Option<Mut<'_, TextColor>>, next: Color) {
    if let Some(mut color) = color {
        color.0 = next;
    }
}

fn set_a11y_label(a11y: Option<Mut<'_, AccessibilityNode>>, label: &str) {
    if let Some(mut a11y) = a11y {
        a11y.0.set_label(label.to_owned());
    }
}
