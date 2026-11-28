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
    MobileClientState, MobileTransportError, MobileTransportResource,
};
use ef_mobile_domain::{
    ConnectivityState, MobileAuthAuthority, MobileClientScreen, MobileSessionId,
};
use ef_ui_catalog::{
    ChromeProduct, ComposerAction, ComposerOutcome, ComposerSurface, DesktopPreferences,
    DesktopPrefsStore, DesktopZoom, IdleRedrawGate, Locale, RedrawReason, ThemeMode, UiKey,
    UiLocale, UiTheme, localized_chrome, translate,
};

use crate::config::{LaunchMode, MobileDesktopConfig};
use crate::forms::{
    FormParseError, MobileInputFocus, parse_dob_iso, parse_email, parse_invite_credential,
    replace_surface_text,
};
use crate::worker::{
    LiveServerWorker, LiveWorkerRequest, WorkerOperation, clear_visible_account_state,
};

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
    FocusField(MobileInputFocus),
    FocusNextField,
    FocusPrevField,
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

/// Typed transport mode descriptor for reactive localized status rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MobileModeKind {
    Demo,
    LiveServer { raw_url: String },
}

/// Single authoritative Curie Mobile desktop presentation state.
#[derive(Debug, Resource)]
pub struct MobileDesktopUiState {
    pub mode_kind: MobileModeKind,
    pub mode_label: String,
    pub patient_email: String,
    pub invite_code_display: String,
    pub dob_display: String,
    pub expected_dob: DateOfBirth,
    pub invite_credential: Option<InviteCredential>,
    pub practice_preview_name: String,
    pub last_status: String,
    pub status_override: Option<String>,
    pub next_transfer_seq: u64,
    pub active_focus: MobileInputFocus,
    pub email_field: ComposerSurface,
    pub invite_code_field: ComposerSurface,
    pub dob_field: ComposerSurface,
}

impl MobileDesktopUiState {
    /// Returns the localized mode badge string (`Demo (in-process)` or `Live server (<url>)`).
    #[must_use]
    pub fn localized_mode_label(&self, locale: Locale) -> String {
        match &self.mode_kind {
            MobileModeKind::Demo => translate(locale, UiKey::MobileModeDemo).to_owned(),
            MobileModeKind::LiveServer { raw_url } => {
                format!("{} ({raw_url})", translate(locale, UiKey::MobileModeLive))
            }
        }
    }

    /// Returns the localized status message, falling back to the localized ready banner when no
    /// interactive action override is active.
    #[must_use]
    pub fn localized_status(&self, locale: Locale) -> String {
        self.status_override
            .clone()
            .unwrap_or_else(|| match &self.mode_kind {
                MobileModeKind::Demo => translate(locale, UiKey::MobileStatusReadyDemo).to_owned(),
                MobileModeKind::LiveServer { raw_url } => {
                    format!(
                        "{} ({raw_url})",
                        localized_chrome(locale, ChromeProduct::Mobile).status_ready
                    )
                }
            })
    }

    /// Updates both `last_status` and the active `status_override` shown in the status banner.
    pub fn set_status_message(&mut self, message: impl Into<String>) {
        let msg = message.into();
        self.last_status.clone_from(&msg);
        self.status_override = Some(msg);
    }

    /// Moves focus explicitly to `field`.
    pub fn set_focus(&mut self, field: MobileInputFocus) {
        self.active_focus = field;
    }

    /// Advances focus forward (`Tab`) across the 4-field focus ring.
    pub fn cycle_focus_next(&mut self) {
        self.active_focus = self.active_focus.next();
    }

    /// Moves focus backward (`Shift+Tab`) across the 4-field focus ring.
    pub fn cycle_focus_prev(&mut self) {
        self.active_focus = self.active_focus.prev();
    }

    /// Synchronizes display strings with the authoritative editable field buffers.
    pub fn sync_field_displays(&mut self) {
        self.patient_email = self.email_field.text().to_owned();
        let invite_text = self.invite_code_field.text();
        self.invite_code_display = if invite_text.trim().is_empty() {
            "---".to_owned()
        } else {
            invite_text.to_owned()
        };
        self.dob_display = self.dob_field.text().to_owned();
    }

    /// Sets the interactive email field buffer and keeps display state synchronized.
    pub fn set_email_text(&mut self, email: &str) {
        replace_surface_text(&mut self.email_field, email);
        self.sync_field_displays();
    }

    /// Sets the interactive invite code field buffer and keeps display state synchronized.
    pub fn set_invite_code_text(&mut self, code: &str) {
        replace_surface_text(&mut self.invite_code_field, code);
        self.sync_field_displays();
    }

    /// Sets the interactive DOB (`YYYY-MM-DD`) field buffer and keeps display state synchronized.
    pub fn set_dob_text(&mut self, dob: &str) {
        replace_surface_text(&mut self.dob_field, dob);
        self.sync_field_displays();
    }

    /// Parses and validates the current interactive email input.
    pub fn resolve_email(&self) -> Result<String, FormParseError> {
        parse_email(self.email_field.text())
    }

    /// Parses and validates the current interactive invitation credential input.
    pub fn resolve_invite_credential(&self) -> Result<InviteCredential, FormParseError> {
        parse_invite_credential(
            self.invite_code_field.text(),
            self.invite_credential.as_ref(),
        )
    }

    /// Parses and validates the current interactive `YYYY-MM-DD` DOB input.
    pub fn resolve_dob(&self) -> Result<DateOfBirth, FormParseError> {
        parse_dob_iso(self.dob_field.text())
    }

    /// Synchronizes text direction on single-line editable fields when the locale changes.
    pub fn set_fields_direction(&mut self, locale: Locale) {
        let dir = locale.direction();
        self.email_field.set_direction(dir);
        self.invite_code_field.set_direction(dir);
        self.dob_field.set_direction(dir);
    }

    /// Builds the desktop presentation state and matching [`MobileTransportResource`].
    #[must_use]
    pub fn from_config(config: &MobileDesktopConfig) -> (Self, MobileTransportResource) {
        let expected_dob =
            DateOfBirth::new(DEMO_DOB_YEAR, DEMO_DOB_MONTH, DEMO_DOB_DAY).expect("valid demo DOB");
        let dob_display = format!("{DEMO_DOB_YEAR:04}-{DEMO_DOB_MONTH:02}-{DEMO_DOB_DAY:02}");
        let dir = config.locale.direction();

        let mut email_field = ComposerSurface::new(dir);
        replace_surface_text(&mut email_field, &config.email);

        let mut dob_field = ComposerSurface::new(dir);
        replace_surface_text(&mut dob_field, &dob_display);

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
                let mut invite_code_field = ComposerSurface::new(dir);
                replace_surface_text(&mut invite_code_field, &code_display);

                let ui_state = Self {
                    mode_kind: MobileModeKind::Demo,
                    mode_label: translate(config.locale, UiKey::MobileModeDemo).to_owned(),
                    patient_email: config.email.clone(),
                    invite_code_display: code_display,
                    dob_display,
                    expected_dob,
                    invite_credential: Some(InviteCredential::ShortCode(issued.short_code)),
                    practice_preview_name: "Synthetic Valley Care Clinic - Dr. Elena Rivera"
                        .to_owned(),
                    last_status: translate(config.locale, UiKey::MobileStatusReadyDemo).to_owned(),
                    status_override: None,
                    next_transfer_seq: 1,
                    active_focus: MobileInputFocus::CuratedNote,
                    email_field,
                    invite_code_field,
                    dob_field,
                };
                (
                    ui_state,
                    MobileTransportResource {
                        transport: Box::new(local),
                    },
                )
            }
            LaunchMode::LiveServer { addr, raw_url } => {
                let invite_code_field = ComposerSurface::new(dir);
                let ui_state = Self {
                    mode_kind: MobileModeKind::LiveServer {
                        raw_url: raw_url.clone(),
                    },
                    mode_label: format!(
                        "{} ({raw_url})",
                        translate(config.locale, UiKey::MobileModeLive)
                    ),
                    patient_email: config.email.clone(),
                    invite_code_display: "---".to_owned(),
                    dob_display,
                    expected_dob,
                    invite_credential: None,
                    practice_preview_name: format!("{addr}"),
                    last_status: format!(
                        "{} ({addr})",
                        localized_chrome(config.locale, ChromeProduct::Mobile).status_ready
                    ),
                    status_override: None,
                    next_transfer_seq: 1,
                    active_focus: MobileInputFocus::CuratedNote,
                    email_field,
                    invite_code_field,
                    dob_field,
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
        let formatted_dob = format!(
            "{:04}-{:02}-{:02}",
            expected_dob.year(),
            expected_dob.month(),
            expected_dob.day()
        );
        replace_surface_text(&mut self.invite_code_field, &display_code);
        replace_surface_text(&mut self.dob_field, &formatted_dob);
        self.invite_code_display = display_code;
        self.dob_display = formatted_dob;
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

fn active_session_id(mobile_state: &MobileClientState) -> Option<MobileSessionId> {
    match mobile_state.auth_device.screen() {
        MobileClientScreen::SignedIn { session_id, .. } => Some(session_id.clone()),
        MobileClientScreen::EnterYourEmail
        | MobileClientScreen::CheckYourEmail { .. }
        | MobileClientScreen::SessionEnded => None,
    }
}

/// Processes typed [`DesktopAction`]s into [`MobileClientCommandQueue`], [`LiveServerWorker`],
/// [`UiTheme`], and [`UiLocale`].
#[allow(clippy::too_many_arguments)]
pub fn process_desktop_actions_system(
    mut actions: ResMut<DesktopActionQueue>,
    mut desktop: ResMut<MobileDesktopUiState>,
    mut mobile_queue: ResMut<MobileClientCommandQueue>,
    mut mobile_state: ResMut<MobileClientState>,
    mut live_worker: Option<ResMut<LiveServerWorker>>,
    mut ui_locale: ResMut<UiLocale>,
    mut ui_theme: ResMut<UiTheme>,
    mut composer: ResMut<ComposerSurface>,
    mut gate: ResMut<IdleRedrawGate>,
    prefs_store: Option<Res<DesktopPrefsStore>>,
) {
    let persist_prefs = |locale: Locale, theme: ThemeMode| {
        if let Some(ref store) = prefs_store
            && store.is_enabled()
        {
            let _ = store.save(&DesktopPreferences {
                theme,
                locale,
                zoom: DesktopZoom::Pct100,
            });
        }
    };

    while let Some(action) = actions.pop() {
        match action {
            DesktopAction::CycleLocale => {
                let next = next_locale(ui_locale.current);
                ui_locale.current = next;
                desktop.set_fields_direction(next);
                mobile_queue.push(MobileClientCommand::SwitchLocale(next));
                desktop.mode_label = desktop.localized_mode_label(next);
                desktop.status_override = None;
                desktop.last_status = desktop.localized_status(next);
                persist_prefs(next, ui_theme.mode);
            }
            DesktopAction::SetLocale(locale) => {
                ui_locale.current = locale;
                desktop.set_fields_direction(locale);
                mobile_queue.push(MobileClientCommand::SwitchLocale(locale));
                desktop.mode_label = desktop.localized_mode_label(locale);
                desktop.status_override = None;
                desktop.last_status = desktop.localized_status(locale);
                persist_prefs(locale, ui_theme.mode);
            }
            DesktopAction::ToggleTheme => {
                let next_mode = match ui_theme.mode {
                    ThemeMode::Dark => ThemeMode::Light,
                    ThemeMode::Light => ThemeMode::Dark,
                };
                ui_theme.set_mode(next_mode);
                gate.request_redraw(RedrawReason::LocaleOrThemeChange);
                desktop.status_override = None;
                desktop.last_status = desktop.localized_status(ui_locale.current);
                persist_prefs(ui_locale.current, next_mode);
            }
            DesktopAction::SetTheme(mode) => {
                ui_theme.set_mode(mode);
                gate.request_redraw(RedrawReason::LocaleOrThemeChange);
                desktop.status_override = None;
                desktop.last_status = desktop.localized_status(ui_locale.current);
                persist_prefs(ui_locale.current, mode);
            }
            DesktopAction::FocusField(field) => {
                desktop.set_focus(field);
                gate.request_redraw(RedrawReason::UserInput);
            }
            DesktopAction::FocusNextField => {
                desktop.cycle_focus_next();
                gate.request_redraw(RedrawReason::UserInput);
            }
            DesktopAction::FocusPrevField => {
                desktop.cycle_focus_prev();
                gate.request_redraw(RedrawReason::UserInput);
            }
            DesktopAction::SignIn => {
                let email = match desktop.resolve_email() {
                    Ok(email) => email,
                    Err(err) => {
                        desktop.set_status_message(err.to_string());
                        gate.request_redraw(RedrawReason::UserInput);
                        continue;
                    }
                };
                desktop.patient_email.clone_from(&email);

                if let Some(worker) = live_worker.as_deref_mut() {
                    if mobile_state.auth_device.connectivity() == ConnectivityState::Offline {
                        mobile_state
                            .errors
                            .push(MobileTransportError::Offline.to_string());
                        desktop.set_status_message("Cannot sign in while offline.");
                        gate.request_redraw(RedrawReason::UserInput);
                        continue;
                    }
                    if let Err(err) = mobile_state.auth_device.submit_email(&email) {
                        mobile_state.errors.push(err.to_string());
                        desktop.set_status_message(format!("Sign-in failed: {err}"));
                        gate.request_redraw(RedrawReason::UserInput);
                        continue;
                    }
                    let now_secs = mobile_state.clock_secs;
                    match worker.submit(LiveWorkerRequest::SignIn {
                        email: email.clone(),
                        now_secs,
                    }) {
                        Ok(()) => {
                            desktop.set_status_message(format!(
                                "{} ({email})",
                                WorkerOperation::SignIn.in_flight_label()
                            ));
                        }
                        Err(err) => {
                            mobile_state.auth_device.return_to_sign_in();
                            mobile_state.errors.push(err.to_string());
                            desktop.set_status_message(format!("Sign-in dispatch failed: {err}"));
                        }
                    }
                    gate.request_redraw(RedrawReason::UserInput);
                } else {
                    mobile_queue.push(MobileClientCommand::SignIn {
                        email: email.clone(),
                    });
                    desktop.set_status_message(format!("Signing in as {email}..."));
                }
            }
            DesktopAction::ToggleConnectivity => {
                let next = match mobile_state.auth_device.connectivity() {
                    ConnectivityState::Online => ConnectivityState::Offline,
                    ConnectivityState::Offline => ConnectivityState::Online,
                };
                mobile_queue.push(MobileClientCommand::SetConnectivity(next));
                let message = match next {
                    ConnectivityState::Online => "Connectivity set to Online.".to_owned(),
                    ConnectivityState::Offline => {
                        "Connectivity set to Offline (sign-out queues owed revocation).".to_owned()
                    }
                };
                desktop.set_status_message(message);
            }
            DesktopAction::OfflineSignOut => {
                if let Some(worker) = live_worker.as_deref_mut() {
                    let _ = mobile_state
                        .push_registry
                        .revoke_device("desktop-mobile-device");
                    clear_visible_account_state(&mut mobile_state, &mut composer);

                    let mut local_auth_shadow = MobileAuthAuthority::new();
                    let connectivity = mobile_state.auth_device.connectivity();
                    if let Some(revoked_session) =
                        mobile_state.auth_device.sign_out(&mut local_auth_shadow)
                        && connectivity == ConnectivityState::Online
                    {
                        match worker.submit(LiveWorkerRequest::RevokeSession {
                            session_id: revoked_session,
                        }) {
                            Ok(()) => {
                                desktop
                                    .set_status_message(WorkerOperation::SignOut.in_flight_label());
                            }
                            Err(err) => {
                                mobile_state.errors.push(err.to_string());
                                desktop.set_status_message(format!(
                                    "Signed out locally; server revoke failed: {err}"
                                ));
                            }
                        }
                    } else {
                        desktop.set_status_message(
                            "Signed out locally (visible health data cleared immediately).",
                        );
                    }
                    gate.request_redraw(RedrawReason::UserInput);
                } else {
                    mobile_queue.push(MobileClientCommand::SignOut {
                        device_id: Some("desktop-mobile-device".to_owned()),
                    });
                    desktop.set_status_message(
                        "Signed out locally (visible health data cleared immediately).",
                    );
                }
            }
            DesktopAction::FlushOwedSignOuts => {
                if let Some(worker) = live_worker.as_deref_mut() {
                    if mobile_state.auth_device.connectivity() == ConnectivityState::Offline {
                        desktop.set_status_message("Cannot flush owed sign-outs while offline.");
                        gate.request_redraw(RedrawReason::UserInput);
                        continue;
                    }
                    let owed_sessions: Vec<MobileSessionId> = mobile_state
                        .auth_device
                        .owed_sign_outs()
                        .iter()
                        .map(|o| o.session_id.clone())
                        .collect();
                    if owed_sessions.is_empty() {
                        desktop.set_status_message("No owed sign-outs queued.");
                        continue;
                    }
                    match worker.submit(LiveWorkerRequest::FlushOwedSignOuts { owed_sessions }) {
                        Ok(()) => {
                            desktop.set_status_message(
                                WorkerOperation::FlushOwedSignOuts.in_flight_label(),
                            );
                        }
                        Err(err) => {
                            mobile_state.errors.push(err.to_string());
                            desktop.set_status_message(format!(
                                "Flush owed sign-outs dispatch failed: {err}"
                            ));
                        }
                    }
                    gate.request_redraw(RedrawReason::UserInput);
                } else {
                    mobile_queue.push(MobileClientCommand::FlushOwedSignOuts);
                    desktop.set_status_message("Flushing queued owed sign-outs.");
                }
            }
            DesktopAction::VerifyDob => {
                let credential = match desktop.resolve_invite_credential() {
                    Ok(cred) => cred,
                    Err(err) => {
                        desktop.set_status_message(err.to_string());
                        gate.request_redraw(RedrawReason::UserInput);
                        continue;
                    }
                };
                let dob = match desktop.resolve_dob() {
                    Ok(dob) => dob,
                    Err(err) => {
                        desktop.set_status_message(err.to_string());
                        gate.request_redraw(RedrawReason::UserInput);
                        continue;
                    }
                };
                desktop.expected_dob = dob;
                desktop.invite_credential = Some(credential.clone());

                if let Some(worker) = live_worker.as_deref_mut() {
                    let Some(session_id) = active_session_id(&mobile_state) else {
                        desktop.set_status_message("Sign in before verifying date of birth.");
                        gate.request_redraw(RedrawReason::UserInput);
                        continue;
                    };
                    let now_secs = mobile_state.clock_secs;
                    match worker.submit(LiveWorkerRequest::VerifyDob {
                        session_id,
                        credential,
                        dob,
                        now_secs,
                    }) {
                        Ok(()) => {
                            desktop
                                .set_status_message(WorkerOperation::VerifyDob.in_flight_label());
                        }
                        Err(err) => {
                            mobile_state.errors.push(err.to_string());
                            desktop.set_status_message(format!(
                                "DOB verification dispatch failed: {err}"
                            ));
                        }
                    }
                    gate.request_redraw(RedrawReason::UserInput);
                } else {
                    mobile_queue
                        .push(MobileClientCommand::ConnectedCareVerifyDob { credential, dob });
                    desktop.set_status_message(
                        "Verifying DOB (sharing remains off until you explicitly select Share).",
                    );
                }
            }
            DesktopAction::ShareConnectedCare => {
                let credential = match desktop.resolve_invite_credential() {
                    Ok(cred) => cred,
                    Err(err) => {
                        desktop.set_status_message(err.to_string());
                        gate.request_redraw(RedrawReason::UserInput);
                        continue;
                    }
                };
                desktop.invite_credential = Some(credential.clone());

                if let Some(worker) = live_worker.as_deref_mut() {
                    let Some(session_id) = active_session_id(&mobile_state) else {
                        desktop.set_status_message("Sign in before enabling Connected Care share.");
                        gate.request_redraw(RedrawReason::UserInput);
                        continue;
                    };
                    let now_secs = mobile_state.clock_secs;
                    match worker.submit(LiveWorkerRequest::ShareConnectedCare {
                        session_id,
                        credential,
                        now_secs,
                    }) {
                        Ok(()) => {
                            desktop.set_status_message(
                                WorkerOperation::ShareConnectedCare.in_flight_label(),
                            );
                        }
                        Err(err) => {
                            mobile_state.errors.push(err.to_string());
                            desktop.set_status_message(format!(
                                "Connected Care share dispatch failed: {err}"
                            ));
                        }
                    }
                    gate.request_redraw(RedrawReason::UserInput);
                } else {
                    mobile_queue.push(MobileClientCommand::ConnectedCareShare { credential });
                    desktop
                        .set_status_message("Explicit Connected Care sharing consent submitted.");
                }
            }
            DesktopAction::ReviewAndSendDoctorNote => {
                let note_text = composer.text().trim().to_owned();
                if note_text.is_empty() {
                    desktop
                        .set_status_message("Enter a curated note before selecting Review & Send.");
                    continue;
                }
                let Some(connection_id) = MobileDesktopUiState::active_connection_id(&mobile_state)
                else {
                    desktop.set_status_message(
                        "Complete DOB verification and explicit Share before sending.",
                    );
                    continue;
                };
                let seq = desktop.next_transfer_seq;
                desktop.next_transfer_seq = desktop.next_transfer_seq.saturating_add(1);
                let Ok(idempotency_key) = IdempotencyKey::new(format!("desktop-note-{seq}")) else {
                    desktop.set_status_message("Failed to allocate idempotency key.");
                    continue;
                };
                let submitted = match composer.apply(ComposerAction::EnterSubmit) {
                    ComposerOutcome::Submitted(text) => text,
                    ComposerOutcome::Edited
                    | ComposerOutcome::SelectionChanged
                    | ComposerOutcome::NoOp => note_text,
                };
                let items = vec![EnvelopeItem {
                    item_id: format!("note-{seq}"),
                    source_version: seq,
                    confidentiality: ConfidentialityTag::Standard,
                    payload_sha256: format!("desktop-note-sha256-{seq:08x}"),
                    verbatim_payload: submitted,
                    part2_notice: None,
                }];

                if let Some(worker) = live_worker.as_deref_mut() {
                    let Some(session_id) = active_session_id(&mobile_state) else {
                        desktop.set_status_message("Sign in before sending a doctor note.");
                        gate.request_redraw(RedrawReason::UserInput);
                        continue;
                    };
                    let now_secs = mobile_state.clock_secs;
                    match worker.submit(LiveWorkerRequest::SendForMyDoctor {
                        session_id,
                        idempotency_key,
                        connection_id,
                        items,
                        part2_consent: None,
                        now_secs,
                    }) {
                        Ok(()) => {
                            desktop.set_status_message(
                                WorkerOperation::SendForMyDoctor.in_flight_label(),
                            );
                        }
                        Err(err) => {
                            mobile_state.errors.push(err.to_string());
                            desktop.set_status_message(format!("Send note dispatch failed: {err}"));
                        }
                    }
                    gate.request_redraw(RedrawReason::UserInput);
                } else {
                    mobile_queue.push(MobileClientCommand::SendForMyDoctor {
                        idempotency_key,
                        connection_id,
                        items,
                        part2_consent: None,
                    });
                    desktop.set_status_message(format!(
                        "Curated note #{seq} queued for explicit send."
                    ));
                }
            }
        }
    }
}
