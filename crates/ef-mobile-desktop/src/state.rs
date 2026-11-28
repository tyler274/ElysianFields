//! Typed state, actions, and demo transport seeding for `ef-mobile-desktop`.

use std::collections::VecDeque;

use bevy::prelude::*;
use ef_connected_care::{
    ConfidentialityTag, ConnectedCareState, ConnectionId, CreateInvitationRequest, DateOfBirth,
    EnvelopeItem, InviteCredential, PairKey,
};
use ef_core::{IdempotencyKey, PatientId, PhysicianId, PracticeId};
use ef_mobile_client::{
    LiveHttpMobileTransport, LocalMobileTransport, MobileClientCommand, MobileClientCommandQueue,
    MobileClientState, MobileTransportResource,
};
use ef_mobile_domain::ConnectivityState;
use ef_ui_catalog::{
    ChromeProduct, ComposerAction, ComposerOutcome, ComposerSurface, IdleRedrawGate, Locale,
    RedrawReason, ThemeMode, UiLocale, UiTheme, localized_chrome,
};

use crate::config::{LaunchMode, MobileDesktopConfig};

/// Canonical crisis helpline details pinned at the top of Curie Mobile.
pub const CRISIS_LINE_988: &str = "988 Suicide & Crisis Lifeline: Call or text 988 (24/7, free)";
pub const CRISIS_LINE_911: &str = "Emergency: Call 911 immediately";
pub const CRISIS_LINE_SAMHSA: &str = "SAMHSA National Helpline: 1-800-662-4357 (24/7 referral)";

/// Synthetic demo patient date of birth for Connected Care verification.
pub const DEMO_DOB_YEAR: u16 = 1985;
pub const DEMO_DOB_MONTH: u8 = 4;
pub const DEMO_DOB_DAY: u8 = 12;

/// Typed interactive actions dispatched by Bevy UI buttons, keyboard shortcuts, or smoke tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesktopAction {
    CycleLocale,
    SetLocale(Locale),
    ToggleTheme,
    SetTheme(ThemeMode),
    SignIn,
    ToggleConnectivity,
    OfflineSignOut,
    FlushOwedSignOuts,
    VerifyDob,
    ShareConnectedCare,
    ReviewAndSendDoctorNote,
}

/// Queue of typed desktop UI actions processed by the single action owner system.
#[derive(Debug, Default, Resource)]
pub struct DesktopActionQueue {
    pending: VecDeque<DesktopAction>,
}

impl DesktopActionQueue {
    pub fn push(&mut self, action: DesktopAction) {
        self.pending.push_back(action);
    }

    pub fn pop(&mut self) -> Option<DesktopAction> {
        self.pending.pop_front()
    }
}

/// Single authoritative Curie Mobile desktop presentation state.
#[derive(Debug, Resource)]
pub struct MobileDesktopUiState {
    pub mode_label: String,
    pub patient_email: String,
    pub invite_code_display: String,
    pub dob_display: String,
    pub expected_dob: DateOfBirth,
    pub invite_credential: Option<InviteCredential>,
    pub practice_preview_name: String,
    pub last_status: String,
    pub next_transfer_seq: u64,
    pub composer_focused: bool,
}

impl MobileDesktopUiState {
    /// Builds the desktop presentation state and matching [`MobileTransportResource`].
    #[must_use]
    pub fn from_config(config: &MobileDesktopConfig) -> (Self, MobileTransportResource) {
        let expected_dob =
            DateOfBirth::new(DEMO_DOB_YEAR, DEMO_DOB_MONTH, DEMO_DOB_DAY).expect("valid demo DOB");
        let dob_display = format!("{DEMO_DOB_YEAR:04}-{DEMO_DOB_MONTH:02}-{DEMO_DOB_DAY:02}");

        match &config.mode {
            LaunchMode::Demo => {
                let mut local = LocalMobileTransport::new();
                let pair = PairKey::new(
                    PracticeId::new("practice-demo-valley").expect("valid practice id"),
                    PatientId::new("patient-demo-001").expect("valid patient id"),
                );
                let issued = local
                    .connected_care
                    .create_invitation(CreateInvitationRequest {
                        pair,
                        issued_by: PhysicianId::new("phys-demo-rivera").expect("valid phys id"),
                        practice_name: "Synthetic Valley Care Clinic".to_owned(),
                        physician_name: "Dr. Elena Rivera".to_owned(),
                        expected_dob,
                        created_at: "2026-01-12T09:00:00Z".to_owned(),
                    })
                    .expect("seed demo connected care invitation");

                let code_display = issued.short_code.as_str().to_owned();
                let ui_state = Self {
                    mode_label: "Demo (in-process)".to_owned(),
                    patient_email: config.email.clone(),
                    invite_code_display: code_display,
                    dob_display,
                    expected_dob,
                    invite_credential: Some(InviteCredential::ShortCode(issued.short_code)),
                    practice_preview_name: "Synthetic Valley Care Clinic - Dr. Elena Rivera"
                        .to_owned(),
                    last_status: "Ready in standalone demo mode.".to_owned(),
                    next_transfer_seq: 1,
                    composer_focused: true,
                };
                (
                    ui_state,
                    MobileTransportResource {
                        transport: Box::new(local),
                    },
                )
            }
            LaunchMode::LiveServer { addr, raw_url } => {
                let ui_state = Self {
                    mode_label: format!("Live ({raw_url})"),
                    patient_email: config.email.clone(),
                    invite_code_display: "Awaiting practice invite".to_owned(),
                    dob_display,
                    expected_dob,
                    invite_credential: None,
                    practice_preview_name: format!("Connected to {addr}"),
                    last_status: format!("Live mobile server configured at {addr}."),
                    next_transfer_seq: 1,
                    composer_focused: true,
                };
                (
                    ui_state,
                    MobileTransportResource {
                        transport: Box::new(LiveHttpMobileTransport::new(*addr)),
                    },
                )
            }
        }
    }

    /// Configures an active invitation credential (used when live server issues an invitation).
    pub fn set_invite_credential(
        &mut self,
        credential: InviteCredential,
        display_code: String,
        expected_dob: DateOfBirth,
    ) {
        self.invite_code_display = display_code;
        self.dob_display = format!(
            "{:04}-{:02}-{:02}",
            expected_dob.year(),
            expected_dob.month(),
            expected_dob.day()
        );
        self.expected_dob = expected_dob;
        self.invite_credential = Some(credential);
    }

    /// Returns the active [`ConnectionId`] when Connected Care sharing is explicitly active.
    #[must_use]
    pub fn active_connection_id(mobile_state: &MobileClientState) -> Option<ConnectionId> {
        let standing = mobile_state.last_connection_standing.as_ref()?;
        if standing.state == ConnectedCareState::Connected {
            standing.active_connection_id.clone()
        } else {
            None
        }
    }
}

/// Cycles to the next locale in the 12-locale compile-time catalog.
#[must_use]
pub fn next_locale(current: Locale) -> Locale {
    let idx = Locale::ALL
        .iter()
        .position(|loc| *loc == current)
        .unwrap_or(0);
    Locale::ALL[(idx + 1) % Locale::ALL.len()]
}

/// Processes typed [`DesktopAction`]s into [`MobileClientCommandQueue`], [`UiTheme`], and [`UiLocale`].
#[allow(clippy::too_many_arguments)]
pub fn process_desktop_actions_system(
    mut actions: ResMut<DesktopActionQueue>,
    mut desktop: ResMut<MobileDesktopUiState>,
    mut mobile_queue: ResMut<MobileClientCommandQueue>,
    mobile_state: Res<MobileClientState>,
    mut ui_locale: ResMut<UiLocale>,
    mut ui_theme: ResMut<UiTheme>,
    mut composer: ResMut<ComposerSurface>,
    mut gate: ResMut<IdleRedrawGate>,
) {
    while let Some(action) = actions.pop() {
        match action {
            DesktopAction::CycleLocale => {
                let next = next_locale(ui_locale.current);
                ui_locale.current = next;
                mobile_queue.push(MobileClientCommand::SwitchLocale(next));
                let chrome = localized_chrome(next, ChromeProduct::Mobile);
                desktop.last_status = format!(
                    "{} ({} | {})",
                    chrome.status_ready,
                    next.bcp47_tag(),
                    if next.is_rtl() { "RTL" } else { "LTR" }
                );
            }
            DesktopAction::SetLocale(locale) => {
                ui_locale.current = locale;
                mobile_queue.push(MobileClientCommand::SwitchLocale(locale));
                let chrome = localized_chrome(locale, ChromeProduct::Mobile);
                desktop.last_status = format!(
                    "{} ({} | {})",
                    chrome.status_ready,
                    locale.bcp47_tag(),
                    if locale.is_rtl() { "RTL" } else { "LTR" }
                );
            }
            DesktopAction::ToggleTheme => {
                let next_mode = match ui_theme.mode {
                    ThemeMode::Dark => ThemeMode::Light,
                    ThemeMode::Light => ThemeMode::Dark,
                };
                ui_theme.set_mode(next_mode);
                gate.request_redraw(RedrawReason::LocaleOrThemeChange);
                desktop.last_status = format!("Theme switched to {next_mode:?} (WCAG 2.1 AA).");
            }
            DesktopAction::SetTheme(mode) => {
                ui_theme.set_mode(mode);
                gate.request_redraw(RedrawReason::LocaleOrThemeChange);
                desktop.last_status = format!("Theme set to {mode:?} (WCAG 2.1 AA).");
            }
            DesktopAction::SignIn => {
                mobile_queue.push(MobileClientCommand::SignIn {
                    email: desktop.patient_email.clone(),
                });
                desktop.last_status = format!("Signing in as {}...", desktop.patient_email);
            }
            DesktopAction::ToggleConnectivity => {
                let next = match mobile_state.auth_device.connectivity() {
                    ConnectivityState::Online => ConnectivityState::Offline,
                    ConnectivityState::Offline => ConnectivityState::Online,
                };
                mobile_queue.push(MobileClientCommand::SetConnectivity(next));
                desktop.last_status = match next {
                    ConnectivityState::Online => "Connectivity set to Online.".to_owned(),
                    ConnectivityState::Offline => {
                        "Connectivity set to Offline (sign-out queues owed revocation).".to_owned()
                    }
                };
            }
            DesktopAction::OfflineSignOut => {
                mobile_queue.push(MobileClientCommand::SignOut {
                    device_id: Some("desktop-mobile-device".to_owned()),
                });
                desktop.last_status =
                    "Signed out locally (visible health data cleared immediately).".to_owned();
            }
            DesktopAction::FlushOwedSignOuts => {
                mobile_queue.push(MobileClientCommand::FlushOwedSignOuts);
                desktop.last_status = "Flushing queued owed sign-outs.".to_owned();
            }
            DesktopAction::VerifyDob => {
                let Some(credential) = desktop.invite_credential.clone() else {
                    desktop.last_status =
                        "No practice invitation loaded for DOB verification.".to_owned();
                    continue;
                };
                mobile_queue.push(MobileClientCommand::ConnectedCareVerifyDob {
                    credential,
                    dob: desktop.expected_dob,
                });
                desktop.last_status =
                    "Verifying DOB (sharing remains off until you explicitly select Share)."
                        .to_owned();
            }
            DesktopAction::ShareConnectedCare => {
                let Some(credential) = desktop.invite_credential.clone() else {
                    desktop.last_status = "No practice invitation loaded to share.".to_owned();
                    continue;
                };
                mobile_queue.push(MobileClientCommand::ConnectedCareShare { credential });
                desktop.last_status =
                    "Explicit Connected Care sharing consent submitted.".to_owned();
            }
            DesktopAction::ReviewAndSendDoctorNote => {
                let note_text = composer.text().trim().to_owned();
                if note_text.is_empty() {
                    desktop.last_status =
                        "Enter a curated note before selecting Review & Send.".to_owned();
                    continue;
                }
                let Some(connection_id) = MobileDesktopUiState::active_connection_id(&mobile_state)
                else {
                    desktop.last_status =
                        "Complete DOB verification and explicit Share before sending.".to_owned();
                    continue;
                };
                let seq = desktop.next_transfer_seq;
                desktop.next_transfer_seq = desktop.next_transfer_seq.saturating_add(1);
                let Ok(idempotency_key) = IdempotencyKey::new(format!("desktop-note-{seq}")) else {
                    desktop.last_status = "Failed to allocate idempotency key.".to_owned();
                    continue;
                };
                let submitted = match composer.apply(ComposerAction::EnterSubmit) {
                    ComposerOutcome::Submitted(text) => text,
                    ComposerOutcome::Edited
                    | ComposerOutcome::SelectionChanged
                    | ComposerOutcome::NoOp => note_text,
                };
                mobile_queue.push(MobileClientCommand::SendForMyDoctor {
                    idempotency_key,
                    connection_id,
                    items: vec![EnvelopeItem {
                        item_id: format!("note-{seq}"),
                        source_version: seq,
                        confidentiality: ConfidentialityTag::Standard,
                        payload_sha256: format!("desktop-note-sha256-{seq:08x}"),
                        verbatim_payload: submitted,
                        part2_notice: None,
                    }],
                    part2_consent: None,
                });
                desktop.last_status = format!("Curated note #{seq} queued for explicit send.");
            }
        }
    }
}
