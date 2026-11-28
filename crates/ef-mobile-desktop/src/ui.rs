//! Native Bevy `0.20.1` Curie Mobile desktop UI plugin (`420x860` mobile-proportioned layout).

use bevy::a11y::AccessibilityNode;
use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::picking::events::PointerClick;
use bevy::prelude::*;
use bevy::text::FontSource;
use ef_connected_care::{ConnectedCareState, DobVerificationOutcome};
use ef_mobile_client::MobileClientState;
use ef_mobile_domain::{ConnectivityState, MobileClientScreen};
use ef_ui_catalog::{
    A11yNodeMarker, A11ySemanticKey, ChromeProduct, ComposerAction, ComposerOutcome,
    ComposerSurface, IdleRedrawGate, NodeInteractivity, RedrawReason, SrgbColor, ThemeMode,
    ThemeTokens, UiKey, UiLocale, UiTheme, localized_chrome, translate,
};

use crate::forms::{
    ClipboardBuffer, MobileInputFocus, copy_surface_to_clipboard, cut_surface_to_clipboard,
    paste_clipboard_into_surface,
};
use crate::state::{
    DesktopAction, DesktopActionQueue, MobileDesktopUiState, OwedRecoveryBannerState,
    process_desktop_actions_system,
};
use crate::worker::{LiveServerWorker, PendingWorkerState, poll_live_server_worker_system};

/// Product-local action label for the clickable owed sign-out flush control.
pub const FLUSH_OWED_SIGN_OUTS_LABEL: &str = "Flush Owed Sign-Outs";

fn desktop_text_font(size_px: f32) -> TextFont {
    TextFont {
        font: FontSource::sans_serif(),
        ..TextFont::from_font_size(size_px)
    }
}

/// High-contrast WCAG 2.1 AA crimson banner palette for Dark and Light themes.
#[derive(Debug, Clone, Copy)]
pub struct CrisisBannerPalette {
    pub bg: SrgbColor,
    pub border: SrgbColor,
    pub text: SrgbColor,
    pub highlight: SrgbColor,
}

impl CrisisBannerPalette {
    #[must_use]
    pub const fn for_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self {
                bg: SrgbColor::rgb(0x3F, 0x1D, 0x2E),
                border: SrgbColor::rgb(0xBE, 0x12, 0x3C),
                text: SrgbColor::rgb(0xFF, 0xF1, 0xF2),
                highlight: SrgbColor::rgb(0xFD, 0xE6, 0x8A),
            },
            ThemeMode::Light => Self {
                bg: SrgbColor::rgb(0xFF, 0xF1, 0xF2),
                border: SrgbColor::rgb(0x9F, 0x12, 0x39),
                text: SrgbColor::rgb(0x88, 0x13, 0x37),
                highlight: SrgbColor::rgb(0x88, 0x13, 0x37),
            },
        }
    }
}

/// Marker component for the root canvas container.
#[derive(Component)]
pub struct RootCanvasNode;

/// Marker component for every horizontal row that mirrors under Hebrew/Arabic RTL.
#[derive(Component)]
pub struct MirroredRowNode;

/// Marker component for standard card surfaces.
#[derive(Component)]
pub struct CardSurfaceNode;

/// Marker component for the crisis-first banner pinned at the top of the viewport.
#[derive(Component)]
pub struct CrisisBannerNode;

/// Marker component for the Connection & Owed Sign-Out recovery status banner.
#[derive(Component)]
pub struct OwedRecoveryBannerNode;

/// Marker component for the curated note composer visual box.
#[derive(Component)]
pub struct ComposerBoxNode;

/// Marker component tagging one of the 4 interactive form input boxes in the focus ring.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormInputBoxNode {
    pub field: MobileInputFocus,
}

/// Marker component binding a Bevy UI [`Button`] to its typed [`DesktopAction`].
#[derive(Component, Clone)]
pub struct ActionButton {
    pub action: DesktopAction,
    pub style: ButtonStyle,
}

/// Visual prominence for native Bevy UI buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonStyle {
    Primary,
    Secondary,
}

/// Typed slots for reactive text updates without duplicating UI entity trees.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextSlot {
    BrandTitle,
    LocaleBtnLabel,
    ThemeBtnLabel,
    StatusBanner,
    CrisisHeading,
    CrisisBody,
    AccountHeading,
    AccountSummary,
    OwedRecoveryBanner,
    EmailInputLabel,
    EmailInputContent,
    SignInBtnLabel,
    ConnectivityBtnLabel,
    SignOutBtnLabel,
    FlushOwedBtnLabel,
    ConnectedCareHeading,
    ConnectedCareSummary,
    InviteInputLabel,
    InviteInputContent,
    DobInputLabel,
    DobInputContent,
    VerifyDobBtnLabel,
    ShareBtnLabel,
    ForMyDoctorHeading,
    ComposerLabel,
    ComposerContent,
    ReviewSendBtnLabel,
    SentSummary,
}

/// Text color role applied reactively when the WCAG 2.1 AA theme changes.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextTone {
    Primary,
    Secondary,
    AccentLabel,
    CrisisHeading,
    CrisisBody,
    Input,
}

/// Bevy `0.20.1` plugin spawning and driving the Curie Mobile native desktop UI.
pub struct MobileDesktopUiPlugin;

impl Plugin for MobileDesktopUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<KeyboardInput>()
            .init_resource::<DesktopActionQueue>()
            .init_resource::<ClipboardBuffer>()
            .add_systems(Startup, spawn_mobile_desktop_ui)
            .add_systems(
                Update,
                (
                    handle_form_keyboard_input_system,
                    process_desktop_actions_system,
                    poll_live_server_worker_system,
                    sync_desktop_visuals_system,
                )
                    .chain(),
            );
    }
}

fn spawn_mobile_desktop_ui(
    mut commands: Commands,
    ui_locale: Res<UiLocale>,
    ui_theme: Res<UiTheme>,
    desktop: Res<MobileDesktopUiState>,
) {
    let tokens = ui_theme.tokens;
    let crisis = CrisisBannerPalette::for_mode(ui_theme.mode);
    let chrome = localized_chrome(ui_locale.current, ChromeProduct::Mobile);
    let row_dir = ui_locale.current.row_flex_direction();

    commands
        .spawn((
            RootCanvasNode,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(10.0),
                padding: UiRect::all(Val::Px(12.0)),
                overflow: Overflow::scroll_y(),
                ..default()
            },
            BackgroundColor(tokens.canvas_bg.to_bevy_color()),
        ))
        .with_children(|root| {
            // 1. Header Bar: Brand title + Locale cycler + Light/Dark theme toggle.
            root.spawn((
                MirroredRowNode,
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: row_dir,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::SpaceBetween,
                    column_gap: Val::Px(6.0),
                    row_gap: Val::Px(6.0),
                    flex_wrap: FlexWrap::Wrap,
                    ..default()
                },
            ))
            .with_children(|header| {
                header.spawn((
                    Text::new(chrome.app_brand.clone()),
                    desktop_text_font(18.0),
                    TextColor(tokens.text_primary.to_bevy_color()),
                    TextSlot::BrandTitle,
                    TextTone::Primary,
                ));

                header
                    .spawn((
                        MirroredRowNode,
                        Node {
                            flex_direction: row_dir,
                            column_gap: Val::Px(6.0),
                            align_items: AlignItems::Center,
                            ..default()
                        },
                    ))
                    .with_children(|controls| {
                        spawn_action_button(
                            controls,
                            DesktopAction::CycleLocale,
                            ButtonStyle::Secondary,
                            TextSlot::LocaleBtnLabel,
                            ui_locale.current.short_badge(),
                            A11ySemanticKey::MobileMenuButton,
                            10,
                            &tokens,
                        );
                        let theme_label = match ui_theme.mode {
                            ThemeMode::Dark => ui_locale.t(UiKey::LabelThemeLight),
                            ThemeMode::Light => ui_locale.t(UiKey::LabelThemeDark),
                        };
                        spawn_action_button(
                            controls,
                            DesktopAction::ToggleTheme,
                            ButtonStyle::Secondary,
                            TextSlot::ThemeBtnLabel,
                            theme_label,
                            A11ySemanticKey::ResumeButton,
                            11,
                            &tokens,
                        );
                    });
            });

            // Status strip
            let locale = ui_locale.current;
            root.spawn((
                CardSurfaceNode,
                Node {
                    width: Val::Percent(100.0),
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(7.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(tokens.surface_bg.to_bevy_color()),
                BorderColor::all(tokens.focus_ring.to_bevy_color()),
            ))
            .with_children(|strip| {
                strip.spawn((
                    Text::new(format!(
                        "{} | {}",
                        desktop.localized_mode_label(locale),
                        desktop.localized_status(locale)
                    )),
                    desktop_text_font(12.0),
                    TextColor(tokens.text_secondary.to_bevy_color()),
                    TextSlot::StatusBanner,
                    TextTone::Secondary,
                ));
            });

            // 2. Crisis-First Banner pinned visibly at top (988, 911, SAMHSA 1-800-662-4357).
            let crisis_988 = translate(locale, UiKey::MobileCrisisLine988);
            let crisis_911 = translate(locale, UiKey::MobileCrisisLine911);
            let crisis_samhsa = translate(locale, UiKey::MobileCrisisLineSamhsa);
            let mut crisis_a11y = accesskit::Node::new(accesskit::Role::Alert);
            crisis_a11y.set_label(format!(
                "{} - {crisis_988}; {crisis_911}; {crisis_samhsa}",
                chrome.crisis_heading
            ));
            root.spawn((
                CrisisBannerNode,
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(4.0),
                    padding: UiRect::all(Val::Px(10.0)),
                    border: UiRect::all(Val::Px(2.0)),
                    ..default()
                },
                BackgroundColor(crisis.bg.to_bevy_color()),
                BorderColor::all(crisis.border.to_bevy_color()),
                A11yNodeMarker {
                    key: A11ySemanticKey::AssertiveAlertRegion,
                    reading_order: 12,
                    interactivity: NodeInteractivity::Structural,
                },
                AccessibilityNode(crisis_a11y),
            ))
            .with_children(|banner| {
                banner.spawn((
                    Text::new(chrome.crisis_heading.clone()),
                    desktop_text_font(14.0),
                    TextColor(crisis.highlight.to_bevy_color()),
                    TextSlot::CrisisHeading,
                    TextTone::CrisisHeading,
                ));
                banner.spawn((
                    Text::new(format!("- {crisis_988}\n- {crisis_911}\n- {crisis_samhsa}")),
                    desktop_text_font(12.0),
                    TextColor(crisis.text.to_bevy_color()),
                    TextSlot::CrisisBody,
                    TextTone::CrisisBody,
                ));
            });

            // 3. Standalone Account & Offline-Safe Sign-Out Section.
            root.spawn((
                CardSurfaceNode,
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(6.0),
                    padding: UiRect::all(Val::Px(10.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(tokens.surface_bg.to_bevy_color()),
                BorderColor::all(tokens.text_secondary.to_bevy_color()),
            ))
            .with_children(|card| {
                card.spawn((
                    Text::new(chrome.nav_account.clone()),
                    desktop_text_font(14.5),
                    TextColor(tokens.text_primary.to_bevy_color()),
                    TextSlot::AccountHeading,
                    TextTone::Primary,
                ));
                card.spawn((
                    Text::new(format!(
                        "{}: {} | {} | {} | {}: 0",
                        translate(locale, UiKey::MobileLabelEmail),
                        desktop.patient_email,
                        translate(locale, UiKey::MobileStatusSignedOut),
                        translate(locale, UiKey::MobileConnectivityOnline),
                        translate(locale, UiKey::MobileLabelOwedSignOuts),
                    )),
                    desktop_text_font(12.0),
                    TextColor(tokens.text_secondary.to_bevy_color()),
                    TextSlot::AccountSummary,
                    TextTone::Secondary,
                ));
                let initial_recovery_text = OwedRecoveryBannerState::SyncedOnline {
                    worker: PendingWorkerState::Idle,
                }
                .format_banner(locale);
                let mut recovery_a11y = accesskit::Node::new(accesskit::Role::Status);
                recovery_a11y.set_label(initial_recovery_text.clone());
                card.spawn((
                    OwedRecoveryBannerNode,
                    Node {
                        width: Val::Percent(100.0),
                        padding: UiRect::axes(Val::Px(8.0), Val::Px(6.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(tokens.canvas_bg.to_bevy_color()),
                    BorderColor::all(tokens.text_secondary.to_bevy_color()),
                    A11yNodeMarker {
                        key: A11ySemanticKey::PoliteLiveRegion,
                        reading_order: 13,
                        interactivity: NodeInteractivity::Structural,
                    },
                    AccessibilityNode(recovery_a11y),
                ))
                .with_children(|banner| {
                    banner.spawn((
                        Text::new(initial_recovery_text),
                        desktop_text_font(11.5),
                        TextColor(tokens.text_primary.to_bevy_color()),
                        TextSlot::OwedRecoveryBanner,
                        TextTone::Primary,
                    ));
                });
                spawn_form_input_field(
                    card,
                    MobileInputFocus::Email,
                    TextSlot::EmailInputLabel,
                    TextSlot::EmailInputContent,
                    translate(locale, UiKey::MobileLabelEmail),
                    desktop.email_field.text(),
                    desktop.active_focus == MobileInputFocus::Email,
                    34.0,
                    14,
                    &tokens,
                );
                card.spawn((
                    MirroredRowNode,
                    Node {
                        width: Val::Percent(100.0),
                        flex_direction: row_dir,
                        column_gap: Val::Px(6.0),
                        row_gap: Val::Px(6.0),
                        flex_wrap: FlexWrap::Wrap,
                        ..default()
                    },
                ))
                .with_children(|row| {
                    spawn_action_button(
                        row,
                        DesktopAction::SignIn,
                        ButtonStyle::Primary,
                        TextSlot::SignInBtnLabel,
                        &chrome.sign_in,
                        A11ySemanticKey::ListRow(1),
                        15,
                        &tokens,
                    );
                    spawn_action_button(
                        row,
                        DesktopAction::ToggleConnectivity,
                        ButtonStyle::Secondary,
                        TextSlot::ConnectivityBtnLabel,
                        translate(locale, UiKey::MobileActionGoOffline),
                        A11ySemanticKey::ListRow(2),
                        16,
                        &tokens,
                    );
                    spawn_action_button(
                        row,
                        DesktopAction::OfflineSignOut,
                        ButtonStyle::Secondary,
                        TextSlot::SignOutBtnLabel,
                        &chrome.sign_out,
                        A11ySemanticKey::ListRow(3),
                        17,
                        &tokens,
                    );
                    spawn_action_button(
                        row,
                        DesktopAction::FlushOwedSignOuts,
                        ButtonStyle::Secondary,
                        TextSlot::FlushOwedBtnLabel,
                        FLUSH_OWED_SIGN_OUTS_LABEL,
                        A11ySemanticKey::TryAgainButton,
                        18,
                        &tokens,
                    );
                });
            });

            // 4. Connected Care Section (Interactive Invite Code + DOB Verify + Explicit Share).
            root.spawn((
                CardSurfaceNode,
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(6.0),
                    padding: UiRect::all(Val::Px(10.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(tokens.surface_bg.to_bevy_color()),
                BorderColor::all(tokens.text_secondary.to_bevy_color()),
            ))
            .with_children(|card| {
                card.spawn((
                    Text::new(chrome.nav_connected_care.clone()),
                    desktop_text_font(14.5),
                    TextColor(tokens.text_primary.to_bevy_color()),
                    TextSlot::ConnectedCareHeading,
                    TextTone::Primary,
                ));
                card.spawn((
                    Text::new(format!(
                        "{}\n{}: {} | {}: {}\n{} | {}",
                        desktop.practice_preview_name,
                        translate(locale, UiKey::MobileLabelInvite),
                        desktop.invite_code_display,
                        translate(locale, UiKey::MobileLabelDob),
                        desktop.dob_display,
                        translate(locale, UiKey::MobileDobNotVerified),
                        translate(locale, UiKey::MobileSharingNotShared),
                    )),
                    desktop_text_font(12.0),
                    TextColor(tokens.text_secondary.to_bevy_color()),
                    TextSlot::ConnectedCareSummary,
                    TextTone::Secondary,
                ));
                spawn_form_input_field(
                    card,
                    MobileInputFocus::InviteCode,
                    TextSlot::InviteInputLabel,
                    TextSlot::InviteInputContent,
                    translate(locale, UiKey::MobileLabelInvite),
                    desktop.invite_code_field.text(),
                    desktop.active_focus == MobileInputFocus::InviteCode,
                    34.0,
                    19,
                    &tokens,
                );
                spawn_form_input_field(
                    card,
                    MobileInputFocus::DateOfBirth,
                    TextSlot::DobInputLabel,
                    TextSlot::DobInputContent,
                    translate(locale, UiKey::MobileLabelDob),
                    desktop.dob_field.text(),
                    desktop.active_focus == MobileInputFocus::DateOfBirth,
                    34.0,
                    20,
                    &tokens,
                );
                card.spawn((
                    MirroredRowNode,
                    Node {
                        width: Val::Percent(100.0),
                        flex_direction: row_dir,
                        column_gap: Val::Px(6.0),
                        row_gap: Val::Px(6.0),
                        flex_wrap: FlexWrap::Wrap,
                        ..default()
                    },
                ))
                .with_children(|row| {
                    spawn_action_button(
                        row,
                        DesktopAction::VerifyDob,
                        ButtonStyle::Secondary,
                        TextSlot::VerifyDobBtnLabel,
                        &chrome.verify_dob,
                        A11ySemanticKey::ListRow(4),
                        21,
                        &tokens,
                    );
                    spawn_action_button(
                        row,
                        DesktopAction::ShareConnectedCare,
                        ButtonStyle::Primary,
                        TextSlot::ShareBtnLabel,
                        &chrome.share,
                        A11ySemanticKey::ListRow(5),
                        22,
                        &tokens,
                    );
                });
            });

            // 5. For My Doctor Section (Curated Note ComposerSurface + Review & Send).
            root.spawn((
                CardSurfaceNode,
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(6.0),
                    padding: UiRect::all(Val::Px(10.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(tokens.surface_bg.to_bevy_color()),
                BorderColor::all(tokens.text_secondary.to_bevy_color()),
            ))
            .with_children(|card| {
                card.spawn((
                    Text::new(chrome.nav_for_my_doctor.clone()),
                    desktop_text_font(14.5),
                    TextColor(tokens.text_primary.to_bevy_color()),
                    TextSlot::ForMyDoctorHeading,
                    TextTone::Primary,
                ));
                spawn_form_input_field(
                    card,
                    MobileInputFocus::CuratedNote,
                    TextSlot::ComposerLabel,
                    TextSlot::ComposerContent,
                    translate(locale, UiKey::MobileCuratedNoteLabel),
                    translate(locale, UiKey::MobileCuratedNotePlaceholder),
                    desktop.active_focus == MobileInputFocus::CuratedNote,
                    64.0,
                    23,
                    &tokens,
                );
                card.spawn((
                    MirroredRowNode,
                    Node {
                        width: Val::Percent(100.0),
                        flex_direction: row_dir,
                        column_gap: Val::Px(6.0),
                        ..default()
                    },
                ))
                .with_children(|row| {
                    spawn_action_button(
                        row,
                        DesktopAction::ReviewAndSendDoctorNote,
                        ButtonStyle::Primary,
                        TextSlot::ReviewSendBtnLabel,
                        &chrome.review_and_send,
                        A11ySemanticKey::CitationChip(0),
                        24,
                        &tokens,
                    );
                });
                card.spawn((
                    Text::new(format!(
                        "{}: 0",
                        translate(locale, UiKey::MobileLabelSentUpdates)
                    )),
                    desktop_text_font(11.5),
                    TextColor(tokens.text_secondary.to_bevy_color()),
                    TextSlot::SentSummary,
                    TextTone::Secondary,
                ));
            });
        });
}

#[allow(clippy::too_many_arguments)]
fn spawn_form_input_field(
    parent: &mut ChildSpawnerCommands,
    field: MobileInputFocus,
    label_slot: TextSlot,
    content_slot: TextSlot,
    label: &str,
    initial_text: &str,
    is_focused: bool,
    min_height_px: f32,
    reading_order: usize,
    tokens: &ThemeTokens,
) {
    parent.spawn((
        Text::new(label.to_owned()),
        desktop_text_font(11.5),
        TextColor(tokens.text_secondary.to_bevy_color()),
        label_slot,
        TextTone::Secondary,
    ));

    let (border_px, border_color) = if is_focused {
        (3.0, tokens.focus_ring.to_bevy_color())
    } else {
        (1.0, tokens.text_secondary.to_bevy_color())
    };

    let role = if field.is_multiline() {
        accesskit::Role::MultilineTextInput
    } else {
        accesskit::Role::TextInput
    };
    let mut a11y_node = accesskit::Node::new(role);
    a11y_node.set_label(label.to_owned());
    a11y_node.set_value(initial_text.to_owned());

    let mut entity_cmd = parent.spawn((
        FormInputBoxNode { field },
        Pickable::default(),
        Node {
            width: Val::Percent(100.0),
            min_height: Val::Px(min_height_px),
            padding: UiRect::all(Val::Px(8.0)),
            border: UiRect::all(Val::Px(border_px)),
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(tokens.input_bg.to_bevy_color()),
        BorderColor::all(border_color),
        A11yNodeMarker {
            key: field.a11y_key(),
            reading_order,
            interactivity: NodeInteractivity::Interactive,
        },
        AccessibilityNode(a11y_node),
    ));

    if field == MobileInputFocus::CuratedNote {
        entity_cmd.insert(ComposerBoxNode);
    }

    entity_cmd
        .observe(
            move |_click: On<PointerClick>,
                  mut desktop: ResMut<MobileDesktopUiState>,
                  mut gate: ResMut<IdleRedrawGate>| {
                desktop.set_focus(field);
                gate.request_redraw(RedrawReason::UserInput);
            },
        )
        .with_children(|box_node| {
            box_node.spawn((
                Text::new(initial_text.to_owned()),
                desktop_text_font(12.5),
                TextColor(tokens.input_fg.to_bevy_color()),
                content_slot,
                TextTone::Input,
            ));
        });
}

#[allow(clippy::too_many_arguments)]
fn spawn_action_button(
    parent: &mut ChildSpawnerCommands,
    action: DesktopAction,
    style: ButtonStyle,
    slot: TextSlot,
    label: &str,
    a11y_key: A11ySemanticKey,
    reading_order: usize,
    tokens: &ThemeTokens,
) {
    let (bg, border, text_color, tone) = match style {
        ButtonStyle::Primary => (
            tokens.accent_fill.to_bevy_color(),
            tokens.accent_fill.to_bevy_color(),
            tokens.accent_label.to_bevy_color(),
            TextTone::AccentLabel,
        ),
        ButtonStyle::Secondary => (
            tokens.canvas_bg.to_bevy_color(),
            tokens.text_secondary.to_bevy_color(),
            tokens.text_primary.to_bevy_color(),
            TextTone::Primary,
        ),
    };

    let mut a11y_node = accesskit::Node::new(accesskit::Role::Button);
    a11y_node.set_label(label.to_owned());

    let click_action = action.clone();
    parent
        .spawn((
            ActionButton { action, style },
            Pickable::default(),
            Node {
                min_height: Val::Px(40.0),
                padding: UiRect::axes(Val::Px(10.0), Val::Px(7.0)),
                border: UiRect::all(Val::Px(1.0)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(bg),
            BorderColor::all(border),
            A11yNodeMarker {
                key: a11y_key,
                reading_order,
                interactivity: NodeInteractivity::Interactive,
            },
            AccessibilityNode(a11y_node),
        ))
        .observe(
            move |_click: On<PointerClick>, mut actions: ResMut<DesktopActionQueue>| {
                actions.push(click_action.clone());
            },
        )
        .with_children(|btn| {
            btn.spawn((
                Text::new(label.to_owned()),
                desktop_text_font(12.5),
                TextColor(text_color),
                slot,
                tone,
            ));
        });
}

fn active_surface_mut<'a>(
    focus: MobileInputFocus,
    desktop: &'a mut MobileDesktopUiState,
    composer: &'a mut ComposerSurface,
) -> &'a mut ComposerSurface {
    match focus {
        MobileInputFocus::Email => &mut desktop.email_field,
        MobileInputFocus::InviteCode => &mut desktop.invite_code_field,
        MobileInputFocus::DateOfBirth => &mut desktop.dob_field,
        MobileInputFocus::CuratedNote => composer,
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_form_keyboard_input_system(
    mut key_messages: MessageReader<KeyboardInput>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mut desktop: ResMut<MobileDesktopUiState>,
    mut composer: ResMut<ComposerSurface>,
    mut clipboard: ResMut<ClipboardBuffer>,
    mut actions: ResMut<DesktopActionQueue>,
    mut gate: ResMut<IdleRedrawGate>,
) {
    let shift = keys
        .as_ref()
        .is_some_and(|k| k.pressed(KeyCode::ShiftLeft) || k.pressed(KeyCode::ShiftRight));
    let primary_mod = keys.as_ref().is_some_and(|k| {
        k.pressed(KeyCode::ControlLeft)
            || k.pressed(KeyCode::ControlRight)
            || k.pressed(KeyCode::SuperLeft)
            || k.pressed(KeyCode::SuperRight)
    });

    let mut form_fields_modified = false;

    for ev in key_messages.read() {
        if ev.state != ButtonState::Pressed {
            continue;
        }

        if matches!(ev.logical_key, Key::Tab) || ev.key_code == KeyCode::Tab {
            if shift {
                desktop.cycle_focus_prev();
            } else {
                desktop.cycle_focus_next();
            }
            gate.request_redraw(RedrawReason::UserInput);
            continue;
        }

        let focus = desktop.active_focus;

        if primary_mod {
            match ev.key_code {
                KeyCode::KeyZ => {
                    let surface = active_surface_mut(focus, &mut desktop, &mut composer);
                    let outcome = if shift {
                        surface.apply(ComposerAction::Redo)
                    } else {
                        surface.apply(ComposerAction::Undo)
                    };
                    if outcome != ComposerOutcome::NoOp {
                        if !focus.is_multiline() {
                            form_fields_modified = true;
                        }
                        gate.request_redraw(RedrawReason::UserInput);
                    }
                    continue;
                }
                KeyCode::KeyA => {
                    let surface = active_surface_mut(focus, &mut desktop, &mut composer);
                    let _ = surface.apply(ComposerAction::SelectAll);
                    gate.request_redraw(RedrawReason::UserInput);
                    continue;
                }
                KeyCode::KeyC => {
                    let surface = active_surface_mut(focus, &mut desktop, &mut composer);
                    let _ = copy_surface_to_clipboard(surface, &mut clipboard);
                    continue;
                }
                KeyCode::KeyX => {
                    let surface = active_surface_mut(focus, &mut desktop, &mut composer);
                    if cut_surface_to_clipboard(surface, &mut clipboard) {
                        if !focus.is_multiline() {
                            form_fields_modified = true;
                        }
                        gate.request_redraw(RedrawReason::UserInput);
                    }
                    continue;
                }
                KeyCode::KeyV => {
                    let surface = active_surface_mut(focus, &mut desktop, &mut composer);
                    let outcome =
                        paste_clipboard_into_surface(surface, &clipboard, !focus.is_multiline());
                    if outcome != ComposerOutcome::NoOp {
                        if !focus.is_multiline() {
                            form_fields_modified = true;
                        }
                        gate.request_redraw(RedrawReason::UserInput);
                    }
                    continue;
                }
                _ => {}
            }
        }

        match &ev.logical_key {
            Key::Enter => match focus {
                MobileInputFocus::Email => {
                    desktop.sync_field_displays();
                    actions.push(DesktopAction::SignIn);
                }
                MobileInputFocus::InviteCode | MobileInputFocus::DateOfBirth => {
                    desktop.sync_field_displays();
                    actions.push(DesktopAction::VerifyDob);
                }
                MobileInputFocus::CuratedNote => {
                    if shift {
                        let _ = composer.apply(ComposerAction::ShiftEnterNewline);
                        gate.request_redraw(RedrawReason::UserInput);
                    } else {
                        actions.push(DesktopAction::ReviewAndSendDoctorNote);
                    }
                }
            },
            Key::Backspace => {
                let surface = active_surface_mut(focus, &mut desktop, &mut composer);
                let _ = surface.apply(ComposerAction::Backspace);
                if !focus.is_multiline() {
                    form_fields_modified = true;
                }
                gate.request_redraw(RedrawReason::UserInput);
            }
            Key::Delete => {
                let surface = active_surface_mut(focus, &mut desktop, &mut composer);
                let _ = surface.apply(ComposerAction::DeleteForward);
                if !focus.is_multiline() {
                    form_fields_modified = true;
                }
                gate.request_redraw(RedrawReason::UserInput);
            }
            Key::ArrowLeft => {
                let surface = active_surface_mut(focus, &mut desktop, &mut composer);
                let _ = surface.apply(ComposerAction::MovePrevCluster {
                    extend_selection: shift,
                });
                gate.request_redraw(RedrawReason::UserInput);
            }
            Key::ArrowRight => {
                let surface = active_surface_mut(focus, &mut desktop, &mut composer);
                let _ = surface.apply(ComposerAction::MoveNextCluster {
                    extend_selection: shift,
                });
                gate.request_redraw(RedrawReason::UserInput);
            }
            Key::Home => {
                let surface = active_surface_mut(focus, &mut desktop, &mut composer);
                let _ = surface.apply(ComposerAction::MoveStart {
                    extend_selection: shift,
                });
                gate.request_redraw(RedrawReason::UserInput);
            }
            Key::End => {
                let surface = active_surface_mut(focus, &mut desktop, &mut composer);
                let _ = surface.apply(ComposerAction::MoveEnd {
                    extend_selection: shift,
                });
                gate.request_redraw(RedrawReason::UserInput);
            }
            Key::Space => {
                let surface = active_surface_mut(focus, &mut desktop, &mut composer);
                let _ = surface.apply(ComposerAction::InsertText(" ".to_owned()));
                if !focus.is_multiline() {
                    form_fields_modified = true;
                }
                gate.request_redraw(RedrawReason::UserInput);
            }
            Key::Character(text)
                if !primary_mod && !text.is_empty() && !text.chars().all(char::is_control) =>
            {
                let surface = active_surface_mut(focus, &mut desktop, &mut composer);
                let _ = surface.apply(ComposerAction::InsertText(text.to_string()));
                if !focus.is_multiline() {
                    form_fields_modified = true;
                }
                gate.request_redraw(RedrawReason::UserInput);
            }
            _ => {}
        }
    }

    if form_fields_modified {
        desktop.sync_field_displays();
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sync_desktop_visuals_system(
    ui_locale: Res<UiLocale>,
    ui_theme: Res<UiTheme>,
    desktop: Res<MobileDesktopUiState>,
    mobile_state: Res<MobileClientState>,
    live_worker: Option<Res<LiveServerWorker>>,
    composer: Res<ComposerSurface>,
    mut root_q: Query<&mut BackgroundColor, With<RootCanvasNode>>,
    mut card_q: Query<
        (&mut BackgroundColor, &mut BorderColor),
        (
            With<CardSurfaceNode>,
            Without<RootCanvasNode>,
            Without<CrisisBannerNode>,
            Without<OwedRecoveryBannerNode>,
            Without<FormInputBoxNode>,
            Without<ActionButton>,
        ),
    >,
    mut crisis_q: Query<
        (&mut BackgroundColor, &mut BorderColor),
        (
            With<CrisisBannerNode>,
            Without<RootCanvasNode>,
            Without<CardSurfaceNode>,
            Without<OwedRecoveryBannerNode>,
            Without<FormInputBoxNode>,
            Without<ActionButton>,
        ),
    >,
    mut recovery_banner_q: Query<
        (&mut Node, &mut BackgroundColor, &mut BorderColor),
        (
            With<OwedRecoveryBannerNode>,
            Without<RootCanvasNode>,
            Without<CardSurfaceNode>,
            Without<CrisisBannerNode>,
            Without<FormInputBoxNode>,
            Without<ActionButton>,
            Without<MirroredRowNode>,
        ),
    >,
    mut input_box_q: Query<
        (
            &FormInputBoxNode,
            &mut Node,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        (
            Without<RootCanvasNode>,
            Without<CardSurfaceNode>,
            Without<CrisisBannerNode>,
            Without<OwedRecoveryBannerNode>,
            Without<ActionButton>,
            Without<MirroredRowNode>,
        ),
    >,
    mut btn_q: Query<
        (&ActionButton, &mut BackgroundColor, &mut BorderColor),
        (
            Without<RootCanvasNode>,
            Without<CardSurfaceNode>,
            Without<CrisisBannerNode>,
            Without<OwedRecoveryBannerNode>,
            Without<FormInputBoxNode>,
        ),
    >,
    mut rows_q: Query<
        &mut Node,
        (
            With<MirroredRowNode>,
            Without<FormInputBoxNode>,
            Without<OwedRecoveryBannerNode>,
        ),
    >,
    mut text_q: Query<(&TextSlot, &TextTone, &mut Text, &mut TextColor)>,
) {
    let tokens = ui_theme.tokens;
    let crisis = CrisisBannerPalette::for_mode(ui_theme.mode);
    let chrome = localized_chrome(ui_locale.current, ChromeProduct::Mobile);
    let row_dir = ui_locale.current.row_flex_direction();
    let owed_count = mobile_state.auth_device.owed_sign_outs().len();
    let worker_state = live_worker
        .as_deref()
        .map_or(PendingWorkerState::Idle, LiveServerWorker::pending_state);
    let recovery_state = OwedRecoveryBannerState::from_runtime(
        mobile_state.auth_device.connectivity(),
        owed_count,
        worker_state,
    );

    for mut bg in &mut root_q {
        *bg = BackgroundColor(tokens.canvas_bg.to_bevy_color());
    }
    for (mut bg, mut border) in &mut card_q {
        *bg = BackgroundColor(tokens.surface_bg.to_bevy_color());
        *border = BorderColor::all(tokens.text_secondary.to_bevy_color());
    }
    for (mut bg, mut border) in &mut crisis_q {
        *bg = BackgroundColor(crisis.bg.to_bevy_color());
        *border = BorderColor::all(crisis.border.to_bevy_color());
    }
    for (mut node, mut bg, mut border) in &mut recovery_banner_q {
        match &recovery_state {
            OwedRecoveryBannerState::SyncedOnline { .. } => {
                node.border = UiRect::all(Val::Px(1.0));
                *bg = BackgroundColor(tokens.canvas_bg.to_bevy_color());
                *border = BorderColor::all(tokens.text_secondary.to_bevy_color());
            }
            OwedRecoveryBannerState::OfflineHolding { .. } => {
                node.border = UiRect::all(Val::Px(2.0));
                *bg = BackgroundColor(tokens.surface_bg.to_bevy_color());
                *border = BorderColor::all(crisis.border.to_bevy_color());
            }
            OwedRecoveryBannerState::OnlineFlushPending { .. } => {
                node.border = UiRect::all(Val::Px(2.0));
                *bg = BackgroundColor(tokens.surface_bg.to_bevy_color());
                *border = BorderColor::all(tokens.focus_ring.to_bevy_color());
            }
        }
    }
    for (input_box, mut node, mut bg, mut border) in &mut input_box_q {
        *bg = BackgroundColor(tokens.input_bg.to_bevy_color());
        if input_box.field == desktop.active_focus {
            node.border = UiRect::all(Val::Px(3.0));
            *border = BorderColor::all(tokens.focus_ring.to_bevy_color());
        } else {
            node.border = UiRect::all(Val::Px(1.0));
            *border = BorderColor::all(tokens.text_secondary.to_bevy_color());
        }
    }
    for (btn, mut bg, mut border) in &mut btn_q {
        let prominent_flush = btn.action == DesktopAction::FlushOwedSignOuts
            && recovery_state.flush_action_prominent();
        if prominent_flush {
            *bg = BackgroundColor(tokens.accent_fill.to_bevy_color());
            *border = BorderColor::all(tokens.focus_ring.to_bevy_color());
            continue;
        }
        match btn.style {
            ButtonStyle::Primary => {
                *bg = BackgroundColor(tokens.accent_fill.to_bevy_color());
                *border = BorderColor::all(tokens.accent_fill.to_bevy_color());
            }
            ButtonStyle::Secondary => {
                *bg = BackgroundColor(tokens.canvas_bg.to_bevy_color());
                *border = BorderColor::all(tokens.text_secondary.to_bevy_color());
            }
        }
    }
    for mut node in &mut rows_q {
        node.flex_direction = row_dir;
    }

    let locale = ui_locale.current;
    let connectivity_label = match mobile_state.auth_device.connectivity() {
        ConnectivityState::Online => translate(locale, UiKey::MobileConnectivityOnline),
        ConnectivityState::Offline => translate(locale, UiKey::MobileConnectivityOffline),
    };
    let toggle_connectivity_label = match mobile_state.auth_device.connectivity() {
        ConnectivityState::Online => translate(locale, UiKey::MobileActionGoOffline),
        ConnectivityState::Offline => translate(locale, UiKey::MobileActionGoOnline),
    };
    let screen_summary = match mobile_state.auth_device.screen() {
        MobileClientScreen::EnterYourEmail => {
            translate(locale, UiKey::MobileStatusSignedOut).to_owned()
        }
        MobileClientScreen::CheckYourEmail { pending_email } => {
            format!(
                "{} ({})",
                translate(locale, UiKey::MobileStatusCheckEmail),
                pending_email.as_str()
            )
        }
        MobileClientScreen::SignedIn { account_id, .. } => {
            format!(
                "{} ({})",
                translate(locale, UiKey::MobileStatusSignedIn),
                account_id.as_str()
            )
        }
        MobileClientScreen::SessionEnded => {
            translate(locale, UiKey::MobileStatusSessionEnded).to_owned()
        }
    };
    let owed_count = mobile_state.auth_device.owed_sign_outs().len();

    let dob_summary = match &mobile_state.last_dob_outcome {
        None => translate(locale, UiKey::MobileDobNotVerified).to_owned(),
        Some(DobVerificationOutcome::Verified { .. }) => {
            translate(locale, UiKey::MobileDobVerified).to_owned()
        }
        Some(DobVerificationOutcome::Mismatch { remaining_attempts }) => {
            format!(
                "{} ({remaining_attempts})",
                translate(locale, UiKey::MobileDobMismatch)
            )
        }
        Some(DobVerificationOutcome::LockedOut) => {
            translate(locale, UiKey::MobileDobLockedOut).to_owned()
        }
    };
    let standing_summary = match &mobile_state.last_connection_standing {
        None => translate(locale, UiKey::MobileSharingNotShared).to_owned(),
        Some(standing) => match standing.state {
            ConnectedCareState::Connected => {
                let conn_id = standing
                    .active_connection_id
                    .as_ref()
                    .map_or("", |id| id.as_str());
                if conn_id.is_empty() {
                    translate(locale, UiKey::MobileSharingShared).to_owned()
                } else {
                    format!(
                        "{} ({conn_id})",
                        translate(locale, UiKey::MobileSharingShared)
                    )
                }
            }
            ConnectedCareState::NotInvited => {
                translate(locale, UiKey::MobileSharingNotInvited).to_owned()
            }
            ConnectedCareState::Invited => {
                translate(locale, UiKey::MobileSharingInvited).to_owned()
            }
            ConnectedCareState::InvitedLocked => {
                translate(locale, UiKey::MobileSharingLocked).to_owned()
            }
            ConnectedCareState::Ended => {
                translate(locale, UiKey::MobileSharingDisconnected).to_owned()
            }
        },
    };

    let email_input_display = desktop.email_field.display_text();
    let invite_input_display = desktop.invite_code_field.display_text();
    let dob_input_display = desktop.dob_field.display_text();

    let composer_display = if composer.display_text().is_empty() {
        translate(locale, UiKey::MobileCuratedNotePlaceholder).to_owned()
    } else {
        composer.display_text()
    };

    let sent_label = translate(locale, UiKey::MobileLabelSentUpdates);
    let sent_summary = if let Some(last) = mobile_state.sent_envelopes.last() {
        let preview = last
            .items
            .first()
            .map_or("-", |item| item.verbatim_payload.as_str());
        format!(
            "{sent_label}: {} | {}: {preview}",
            mobile_state.sent_envelopes.len(),
            translate(locale, UiKey::MobileLabelLatest),
        )
    } else {
        format!("{sent_label}: 0")
    };

    let theme_btn_text = match ui_theme.mode {
        ThemeMode::Dark => ui_locale.t(UiKey::LabelThemeLight).to_owned(),
        ThemeMode::Light => ui_locale.t(UiKey::LabelThemeDark).to_owned(),
    };

    let crisis_988 = translate(locale, UiKey::MobileCrisisLine988);
    let crisis_911 = translate(locale, UiKey::MobileCrisisLine911);
    let crisis_samhsa = translate(locale, UiKey::MobileCrisisLineSamhsa);

    let recovery_banner_text = recovery_state.format_banner(locale);
    let flush_btn_text = if owed_count > 0 {
        format!("{FLUSH_OWED_SIGN_OUTS_LABEL} ({owed_count})")
    } else {
        FLUSH_OWED_SIGN_OUTS_LABEL.to_owned()
    };

    for (slot, tone, mut text, mut color) in &mut text_q {
        let effective_color =
            if *slot == TextSlot::FlushOwedBtnLabel && recovery_state.flush_action_prominent() {
                tokens.accent_label.to_bevy_color()
            } else {
                match tone {
                    TextTone::Primary => tokens.text_primary.to_bevy_color(),
                    TextTone::Secondary => tokens.text_secondary.to_bevy_color(),
                    TextTone::AccentLabel => tokens.accent_label.to_bevy_color(),
                    TextTone::CrisisHeading => crisis.highlight.to_bevy_color(),
                    TextTone::CrisisBody => crisis.text.to_bevy_color(),
                    TextTone::Input => tokens.input_fg.to_bevy_color(),
                }
            };
        *color = TextColor(effective_color);

        let next_text = match slot {
            TextSlot::BrandTitle => chrome.app_brand.clone(),
            TextSlot::LocaleBtnLabel => locale.short_badge().to_owned(),
            TextSlot::ThemeBtnLabel => theme_btn_text.clone(),
            TextSlot::StatusBanner => format!(
                "{} | {} ({})",
                desktop.localized_mode_label(locale),
                desktop.localized_status(locale),
                locale.bcp47_tag()
            ),
            TextSlot::CrisisHeading => chrome.crisis_heading.clone(),
            TextSlot::CrisisBody => {
                format!("- {crisis_988}\n- {crisis_911}\n- {crisis_samhsa}")
            }
            TextSlot::AccountHeading => chrome.nav_account.clone(),
            TextSlot::AccountSummary => format!(
                "{}: {} | {} | {} | {}: {owed_count}",
                translate(locale, UiKey::MobileLabelEmail),
                desktop.patient_email,
                screen_summary,
                connectivity_label,
                translate(locale, UiKey::MobileLabelOwedSignOuts),
            ),
            TextSlot::OwedRecoveryBanner => recovery_banner_text.clone(),
            TextSlot::EmailInputLabel => translate(locale, UiKey::MobileLabelEmail).to_owned(),
            TextSlot::EmailInputContent => email_input_display.clone(),
            TextSlot::SignInBtnLabel => chrome.sign_in.clone(),
            TextSlot::ConnectivityBtnLabel => toggle_connectivity_label.to_owned(),
            TextSlot::SignOutBtnLabel => chrome.sign_out.clone(),
            TextSlot::FlushOwedBtnLabel => flush_btn_text.clone(),
            TextSlot::ConnectedCareHeading => chrome.nav_connected_care.clone(),
            TextSlot::ConnectedCareSummary => format!(
                "{}\n{}: {} | {}: {}\n{} | {}",
                desktop.practice_preview_name,
                translate(locale, UiKey::MobileLabelInvite),
                desktop.invite_code_display,
                translate(locale, UiKey::MobileLabelDob),
                desktop.dob_display,
                dob_summary,
                standing_summary
            ),
            TextSlot::InviteInputLabel => translate(locale, UiKey::MobileLabelInvite).to_owned(),
            TextSlot::InviteInputContent => invite_input_display.clone(),
            TextSlot::DobInputLabel => translate(locale, UiKey::MobileLabelDob).to_owned(),
            TextSlot::DobInputContent => dob_input_display.clone(),
            TextSlot::VerifyDobBtnLabel => chrome.verify_dob.clone(),
            TextSlot::ShareBtnLabel => chrome.share.clone(),
            TextSlot::ForMyDoctorHeading => chrome.nav_for_my_doctor.clone(),
            TextSlot::ComposerLabel => translate(locale, UiKey::MobileCuratedNoteLabel).to_owned(),
            TextSlot::ComposerContent => composer_display.clone(),
            TextSlot::ReviewSendBtnLabel => chrome.review_and_send.clone(),
            TextSlot::SentSummary => sent_summary.clone(),
        };

        if text.0 != next_text {
            text.0 = next_text;
        }
    }
}
