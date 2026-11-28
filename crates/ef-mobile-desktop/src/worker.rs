//! Non-blocking background HTTP/1.1 worker thread for `--server <url>` live mode.
//!
//! All [`LiveHttpMobileTransport`] socket operations execute off the Bevy UI thread and
//! deliver typed [`LiveWorkerResponse`] results over `std::sync::mpsc` polled each frame.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use bevy::prelude::*;
use ef_connected_care::{
    ConnectionId, ConnectionStanding, DateOfBirth, DobVerificationOutcome, EnvelopeItem,
    InviteCredential, Part2WrittenConsent, TransferEnvelope,
};
use ef_core::IdempotencyKey;
use ef_mobile_client::{
    AccountDeleteReceipt, LiveHttpMobileTransport, MobileClientState, MobileClientTransport,
    MobileTransportError,
};
use ef_mobile_domain::{
    AccountFencedHealthBatch, HealthSampleEntry, IncomingBatchItem, MobileAuthAuthority,
    MobileSessionId, PatientRecord, PortablePatientExport, VerifiedMobileSession,
};
use ef_ui_catalog::{
    ComposerAction, ComposerSurface, IdleRedrawGate, RedrawReason, UiKey, UiLocale, translate,
};
use thiserror::Error;

use crate::state::MobileDesktopUiState;

/// Typed live-server operation dispatched to the background HTTP worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerOperation {
    SignIn,
    SignOut,
    FlushOwedSignOuts,
    VerifyDob,
    ShareConnectedCare,
    DisconnectConnectedCare,
    SendForMyDoctor,
    SyncHealthSnapshot,
    ExportData,
    DeleteAccount,
}

impl WorkerOperation {
    #[must_use]
    pub const fn in_flight_label(self) -> &'static str {
        match self {
            Self::SignIn => "Signing in over live server...",
            Self::SignOut => "Revoking live server session...",
            Self::FlushOwedSignOuts => "Flushing owed sign-outs to live server...",
            Self::VerifyDob => "Verifying date of birth with live server...",
            Self::ShareConnectedCare => "Submitting Connected Care sharing consent...",
            Self::DisconnectConnectedCare => "Disconnecting Connected Care...",
            Self::SendForMyDoctor => "Delivering curated note to your doctor...",
            Self::SyncHealthSnapshot => "Syncing personal health trends & records...",
            Self::ExportData => "Exporting portable patient data...",
            Self::DeleteAccount => "Permanently deleting mobile account...",
        }
    }
}

/// Explicit typed in-flight status of the background HTTP worker (no boolean flags).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PendingWorkerState {
    #[default]
    Idle,
    InFlight {
        operation: WorkerOperation,
        pending_count: usize,
    },
}

/// Request sent from the Bevy UI thread to the background HTTP worker thread.
#[derive(Debug)]
pub enum LiveWorkerRequest {
    SignIn {
        email: String,
        now_secs: u64,
    },
    RevokeSession {
        session_id: MobileSessionId,
    },
    FlushOwedSignOuts {
        owed_sessions: Vec<MobileSessionId>,
    },
    VerifyDob {
        session_id: MobileSessionId,
        credential: InviteCredential,
        dob: DateOfBirth,
        now_secs: u64,
    },
    ShareConnectedCare {
        session_id: MobileSessionId,
        credential: InviteCredential,
        now_secs: u64,
    },
    SendForMyDoctor {
        session_id: MobileSessionId,
        idempotency_key: IdempotencyKey,
        connection_id: ConnectionId,
        items: Vec<EnvelopeItem>,
        part2_consent: Option<Part2WrittenConsent>,
        now_secs: u64,
    },
    SyncHealthSnapshot {
        session_id: MobileSessionId,
        batch: AccountFencedHealthBatch,
        records: Vec<IncomingBatchItem>,
        now_secs: u64,
    },
    ExportData {
        session_id: MobileSessionId,
        now_secs: u64,
    },
    DisconnectConnectedCare {
        session_id: MobileSessionId,
        connection_id: ConnectionId,
        now_secs: u64,
    },
    DeleteAccount {
        session_id: MobileSessionId,
        event_id: String,
        occurred_at: String,
        now_secs: u64,
    },
}

impl LiveWorkerRequest {
    #[must_use]
    pub const fn operation(&self) -> WorkerOperation {
        match self {
            Self::SignIn { .. } => WorkerOperation::SignIn,
            Self::RevokeSession { .. } => WorkerOperation::SignOut,
            Self::FlushOwedSignOuts { .. } => WorkerOperation::FlushOwedSignOuts,
            Self::VerifyDob { .. } => WorkerOperation::VerifyDob,
            Self::ShareConnectedCare { .. } => WorkerOperation::ShareConnectedCare,
            Self::DisconnectConnectedCare { .. } => WorkerOperation::DisconnectConnectedCare,
            Self::SendForMyDoctor { .. } => WorkerOperation::SendForMyDoctor,
            Self::SyncHealthSnapshot { .. } => WorkerOperation::SyncHealthSnapshot,
            Self::ExportData { .. } => WorkerOperation::ExportData,
            Self::DeleteAccount { .. } => WorkerOperation::DeleteAccount,
        }
    }
}

/// Result delivered asynchronously from the background HTTP worker thread back to Bevy.
#[derive(Debug)]
pub enum LiveWorkerResponse {
    SignIn {
        email: String,
        result: Result<VerifiedMobileSession, MobileTransportError>,
    },
    RevokeSession {
        result: Result<(), MobileTransportError>,
    },
    FlushOwedSignOuts {
        result: Result<usize, MobileTransportError>,
    },
    VerifyDob {
        result: Result<DobVerificationOutcome, MobileTransportError>,
    },
    ShareConnectedCare {
        result: Result<ConnectionStanding, MobileTransportError>,
    },
    SendForMyDoctor {
        result: Result<TransferEnvelope, MobileTransportError>,
    },
    SyncHealthSnapshot {
        result: Result<(Vec<HealthSampleEntry>, Vec<PatientRecord>), MobileTransportError>,
    },
    ExportData {
        result: Result<PortablePatientExport, MobileTransportError>,
    },
    DisconnectConnectedCare {
        result: Result<ConnectionStanding, MobileTransportError>,
    },
    DeleteAccount {
        result: Result<AccountDeleteReceipt, MobileTransportError>,
    },
}

/// Typed dispatch failure when submitting work to the background HTTP worker.
#[derive(Debug, Error)]
pub enum WorkerDispatchError {
    #[error("background HTTP worker lock poisoned: {detail}")]
    LockPoisoned { detail: String },
    #[error("background HTTP worker channel is closed: {detail}")]
    ChannelClosed { detail: String },
}

/// Dedicated background HTTP worker resource for `--server <url>` live mode.
#[derive(Resource)]
pub struct LiveServerWorker {
    addr: SocketAddr,
    req_tx: Mutex<Option<mpsc::Sender<LiveWorkerRequest>>>,
    resp_rx: Mutex<mpsc::Receiver<LiveWorkerResponse>>,
    in_flight: VecDeque<WorkerOperation>,
    join_handle: Mutex<Option<JoinHandle<()>>>,
}

impl LiveServerWorker {
    /// Spawns a dedicated background thread executing [`LiveHttpMobileTransport`] requests.
    #[must_use]
    pub fn spawn(addr: SocketAddr) -> Self {
        Self::spawn_with_timeout(addr, Duration::from_secs(10))
    }

    /// Spawns a dedicated background HTTP worker with an explicit socket timeout.
    #[must_use]
    pub fn spawn_with_timeout(addr: SocketAddr, timeout: Duration) -> Self {
        let (req_tx, req_rx) = mpsc::channel::<LiveWorkerRequest>();
        let (resp_tx, resp_rx) = mpsc::channel::<LiveWorkerResponse>();

        let handle = thread::Builder::new()
            .name("ef-mobile-http-worker".to_owned())
            .spawn(move || {
                let mut transport = LiveHttpMobileTransport::new(addr);
                transport.set_timeout(timeout);
                run_worker_loop(&mut transport, &req_rx, &resp_tx);
            })
            .expect("spawn ef-mobile-http-worker thread");

        Self {
            addr,
            req_tx: Mutex::new(Some(req_tx)),
            resp_rx: Mutex::new(resp_rx),
            in_flight: VecDeque::new(),
            join_handle: Mutex::new(Some(handle)),
        }
    }

    /// Returns the target server socket address.
    #[must_use]
    pub const fn server_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Returns the current typed in-flight status of the worker.
    #[must_use]
    pub fn pending_state(&self) -> PendingWorkerState {
        match self.in_flight.front().copied() {
            None => PendingWorkerState::Idle,
            Some(operation) => PendingWorkerState::InFlight {
                operation,
                pending_count: self.in_flight.len(),
            },
        }
    }

    /// Returns `true` when zero requests are currently in flight.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.in_flight.is_empty()
    }

    /// Queues a request on the background HTTP worker without blocking the UI thread.
    pub fn submit(&mut self, request: LiveWorkerRequest) -> Result<(), WorkerDispatchError> {
        let op = request.operation();
        let guard = self
            .req_tx
            .lock()
            .map_err(|err| WorkerDispatchError::LockPoisoned {
                detail: err.to_string(),
            })?;
        let Some(tx) = guard.as_ref() else {
            return Err(WorkerDispatchError::ChannelClosed {
                detail: "worker sender already shut down".to_owned(),
            });
        };
        tx.send(request)
            .map_err(|err| WorkerDispatchError::ChannelClosed {
                detail: err.to_string(),
            })?;
        self.in_flight.push_back(op);
        Ok(())
    }

    /// Non-blocking poll draining all completed responses available on the current frame.
    pub fn drain_responses(&mut self) -> Vec<LiveWorkerResponse> {
        let mut completed = Vec::new();
        if let Ok(rx) = self.resp_rx.lock() {
            while let Ok(resp) = rx.try_recv() {
                let _ = self.in_flight.pop_front();
                completed.push(resp);
            }
        }
        completed
    }
}

impl Drop for LiveServerWorker {
    fn drop(&mut self) {
        if let Ok(mut tx_guard) = self.req_tx.lock() {
            let _ = tx_guard.take();
        }
        if self.in_flight.is_empty()
            && let Ok(mut handle_guard) = self.join_handle.lock()
            && let Some(handle) = handle_guard.take()
        {
            let _ = handle.join();
        }
    }
}

fn run_worker_loop(
    transport: &mut LiveHttpMobileTransport,
    req_rx: &mpsc::Receiver<LiveWorkerRequest>,
    resp_tx: &mpsc::Sender<LiveWorkerResponse>,
) {
    while let Ok(req) = req_rx.recv() {
        let response = match req {
            LiveWorkerRequest::SignIn { email, now_secs } => {
                let result = transport
                    .request_sign_in_link(&email, now_secs)
                    .and_then(|token| transport.verify_sign_in(&email, &token, now_secs));
                LiveWorkerResponse::SignIn { email, result }
            }
            LiveWorkerRequest::RevokeSession { session_id } => LiveWorkerResponse::RevokeSession {
                result: transport.revoke_session(&session_id),
            },
            LiveWorkerRequest::FlushOwedSignOuts { owed_sessions } => {
                LiveWorkerResponse::FlushOwedSignOuts {
                    result: transport.revoke_sessions(None, &owed_sessions),
                }
            }
            LiveWorkerRequest::VerifyDob {
                session_id,
                credential,
                dob,
                now_secs,
            } => LiveWorkerResponse::VerifyDob {
                result: transport.connected_care_verify_dob(&session_id, credential, dob, now_secs),
            },
            LiveWorkerRequest::ShareConnectedCare {
                session_id,
                credential,
                now_secs,
            } => LiveWorkerResponse::ShareConnectedCare {
                result: transport.connected_care_share(&session_id, credential, now_secs),
            },
            LiveWorkerRequest::SendForMyDoctor {
                session_id,
                idempotency_key,
                connection_id,
                items,
                part2_consent,
                now_secs,
            } => LiveWorkerResponse::SendForMyDoctor {
                result: transport.connected_care_transfer(
                    &session_id,
                    idempotency_key,
                    connection_id,
                    items,
                    part2_consent,
                    now_secs,
                ),
            },
            LiveWorkerRequest::SyncHealthSnapshot {
                session_id,
                batch,
                records,
                now_secs,
            } => {
                let result = transport
                    .upload_health_batch(&session_id, batch, now_secs)
                    .and_then(|_| transport.ingest_records(&session_id, records, now_secs))
                    .and_then(|_| {
                        let trends = transport.fetch_health_trends(&session_id, now_secs)?;
                        let fetched_records = transport.fetch_records(&session_id, now_secs)?;
                        Ok((trends, fetched_records))
                    });
                LiveWorkerResponse::SyncHealthSnapshot { result }
            }
            LiveWorkerRequest::ExportData {
                session_id,
                now_secs,
            } => LiveWorkerResponse::ExportData {
                result: transport.export_patient_data(&session_id, now_secs),
            },
            LiveWorkerRequest::DisconnectConnectedCare {
                session_id,
                connection_id,
                now_secs,
            } => LiveWorkerResponse::DisconnectConnectedCare {
                result: transport.connected_care_disconnect(&session_id, connection_id, now_secs),
            },
            LiveWorkerRequest::DeleteAccount {
                session_id,
                event_id,
                occurred_at,
                now_secs,
            } => LiveWorkerResponse::DeleteAccount {
                result: transport.delete_account(&session_id, &event_id, &occurred_at, now_secs),
            },
        };

        if resp_tx.send(response).is_err() {
            break;
        }
    }
}

/// Clears visible patient account state and composer content (`SignInHoldsNothing` invariant).
pub fn clear_visible_account_state(
    mobile_state: &mut MobileClientState,
    composer: &mut ComposerSurface,
) {
    mobile_state.visible_health_trends.clear();
    mobile_state.visible_records.clear();
    mobile_state.last_ingest_report = None;
    mobile_state.last_invite_challenge = None;
    mobile_state.last_dob_outcome = None;
    mobile_state.last_connection_standing = None;
    mobile_state.sent_envelopes.clear();
    mobile_state.last_export = None;
    let _ = composer.apply(ComposerAction::SelectAll);
    let _ = composer.apply(ComposerAction::Backspace);
}

/// Non-blocking Bevy `Update` system polling [`LiveServerWorker`] responses and applying them cleanly.
pub fn poll_live_server_worker_system(
    live_worker: Option<ResMut<LiveServerWorker>>,
    mut mobile_state: ResMut<MobileClientState>,
    mut desktop: ResMut<MobileDesktopUiState>,
    mut composer: ResMut<ComposerSurface>,
    ui_locale: Res<UiLocale>,
    mut gate: ResMut<IdleRedrawGate>,
) {
    let Some(mut worker) = live_worker else {
        return;
    };

    let responses = worker.drain_responses();
    if responses.is_empty() {
        return;
    }

    for response in responses {
        match response {
            LiveWorkerResponse::SignIn { email, result } => match result {
                Ok(verified) => {
                    clear_visible_account_state(&mut mobile_state, &mut composer);
                    mobile_state.auth_device.complete_sign_in(verified);
                    desktop.set_status_message(format!("Signed in as {email}."));
                }
                Err(err) => {
                    mobile_state.auth_device.return_to_sign_in();
                    mobile_state.errors.push(err.to_string());
                    desktop.set_status_message(format!("Sign-in failed: {err}"));
                }
            },
            LiveWorkerResponse::RevokeSession { result } => match result {
                Ok(()) => {
                    desktop.set_status_message("Signed out and revoked live server session.");
                }
                Err(err) => {
                    mobile_state.errors.push(err.to_string());
                    desktop.set_status_message(format!(
                        "Signed out locally; server revoke failed: {err}"
                    ));
                }
            },
            LiveWorkerResponse::FlushOwedSignOuts { result } => match result {
                Ok(revoked_count) => {
                    let mut dummy = MobileAuthAuthority::new();
                    let _ = mobile_state.auth_device.flush_owed_sign_outs(&mut dummy);
                    desktop.set_status_message(format!(
                        "Flushed {revoked_count} owed sign-out(s) with live server."
                    ));
                }
                Err(err) => {
                    mobile_state.errors.push(err.to_string());
                    desktop.set_status_message(format!("Flush owed sign-outs failed: {err}"));
                }
            },
            LiveWorkerResponse::VerifyDob { result } => match result {
                Ok(outcome) => {
                    let message = match &outcome {
                        DobVerificationOutcome::Verified { preview } => {
                            desktop.practice_preview_name =
                                format!("{} - {}", preview.practice_name, preview.physician_name);
                            format!(
                                "DOB verified for {} (select Share to enable sharing).",
                                preview.practice_name
                            )
                        }
                        DobVerificationOutcome::Mismatch { remaining_attempts } => {
                            format!(
                                "Date of birth did not match ({remaining_attempts} attempt(s) remaining)."
                            )
                        }
                        DobVerificationOutcome::LockedOut => {
                            "Invitation locked after repeated DOB mismatches.".to_owned()
                        }
                    };
                    mobile_state.last_dob_outcome = Some(outcome);
                    desktop.set_status_message(message);
                }
                Err(err) => {
                    mobile_state.errors.push(err.to_string());
                    desktop.set_status_message(format!("DOB verification failed: {err}"));
                }
            },
            LiveWorkerResponse::ShareConnectedCare { result } => match result {
                Ok(standing) => {
                    mobile_state.last_connection_standing = Some(standing);
                    desktop.set_status_message("Connected Care sharing is now active.");
                }
                Err(err) => {
                    mobile_state.errors.push(err.to_string());
                    desktop.set_status_message(format!("Connected Care share failed: {err}"));
                }
            },
            LiveWorkerResponse::SendForMyDoctor { result } => match result {
                Ok(envelope) => {
                    mobile_state.sent_envelopes.push(envelope);
                    let total = mobile_state.sent_envelopes.len();
                    desktop.set_status_message(format!(
                        "Curated note #{total} delivered to your doctor."
                    ));
                }
                Err(err) => {
                    mobile_state.errors.push(err.to_string());
                    desktop.set_status_message(format!("Send note failed: {err}"));
                }
            },
            LiveWorkerResponse::SyncHealthSnapshot { result } => match result {
                Ok((trends, records)) => {
                    if matches!(
                        mobile_state.auth_device.screen(),
                        ef_mobile_domain::MobileClientScreen::SignedIn { .. }
                    ) {
                        let trend_count = trends.len();
                        let record_count = records.len();
                        mobile_state.visible_health_trends = trends;
                        mobile_state.visible_records = records;
                        desktop.set_status_message(format!(
                            "Synced {trend_count} health trend sample(s) and {record_count} personal record(s)."
                        ));
                    }
                }
                Err(err) => {
                    mobile_state.errors.push(err.to_string());
                    desktop.set_status_message(format!("Health sync failed: {err}"));
                }
            },
            LiveWorkerResponse::ExportData { result } => match result {
                Ok(export) => {
                    if matches!(
                        mobile_state.auth_device.screen(),
                        ef_mobile_domain::MobileClientScreen::SignedIn { .. }
                    ) {
                        let sample_count = export.health_samples.len();
                        let record_count = export.records.len();
                        mobile_state.last_export = Some(export);
                        desktop.set_status_message(format!(
                            "{} ({sample_count} sample(s), {record_count} record(s)).",
                            translate(ui_locale.current, UiKey::MobileExportReady)
                        ));
                    }
                }
                Err(err) => {
                    mobile_state.errors.push(err.to_string());
                    desktop.set_status_message(format!("Export failed: {err}"));
                }
            },
            LiveWorkerResponse::DisconnectConnectedCare { result } => match result {
                Ok(standing) => {
                    if matches!(
                        mobile_state.auth_device.screen(),
                        ef_mobile_domain::MobileClientScreen::SignedIn { .. }
                    ) {
                        mobile_state.last_connection_standing = Some(standing);
                        desktop.set_status_message("Connected Care disconnected.");
                    }
                }
                Err(err) => {
                    mobile_state.errors.push(err.to_string());
                    desktop.set_status_message(format!("Disconnect failed: {err}"));
                }
            },
            LiveWorkerResponse::DeleteAccount { result } => match result {
                Ok(receipt) => {
                    let _ = mobile_state
                        .push_registry
                        .revoke_device("desktop-mobile-device");
                    // Enforce SignInHoldsNothing immediately upon permanent deletion.
                    clear_visible_account_state(&mut mobile_state, &mut composer);
                    mobile_state.last_delete_receipt = Some(receipt.clone());
                    let mut shadow = MobileAuthAuthority::new();
                    let _ = mobile_state.auth_device.sign_out(&mut shadow);
                    desktop.set_status_message(format!(
                        "{} (receipt: {})",
                        translate(ui_locale.current, UiKey::MobileAccountDeleted),
                        receipt.receipt_event_id
                    ));
                }
                Err(err) => {
                    mobile_state.errors.push(err.to_string());
                    desktop.set_status_message(format!("Account deletion failed: {err}"));
                }
            },
        }
    }

    mobile_state.a11y_tree_generation = mobile_state.a11y_tree_generation.saturating_add(1);
    gate.request_redraw(RedrawReason::StreamDelta);
}
