//! Non-blocking background HTTP worker for live `--server <url>` progressive SSE streaming.
//!
//! Executes `LiveHttpClinicTransport` command submission, progressive SSE delta streaming,
//! and mid-flight cancellation (`CommandPayload::WorkStop`) on a dedicated OS thread
//! communicating over `std::sync::mpsc` so the Bevy desktop UI thread never blocks on network I/O.

use std::net::SocketAddr;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{self, Receiver, Sender};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::prelude::Resource;
use ef_clinic_workflows::memory::MemoryEntry;
use ef_clinic_workflows::today::{RenderedTodayItem, ScheduledVisitEntry, TodayChannelBanner};
use ef_core::{ChatId, IdempotencyKey, SessionId, WorkspaceId};
use serde::{Deserialize, Serialize};

/// Typed standing update action for `POST /elysianfields/api/today/standing`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodayStandingUpdate {
    Seen,
    Done,
}

impl TodayStandingUpdate {
    /// Returns the wire snake_case identifier for this standing update.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Seen => "seen",
            Self::Done => "done",
        }
    }
}

/// Explicit typed in-flight posture for live `--server` prompt submission.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum LiveSubmissionState {
    #[default]
    Idle,
    InFlight {
        request_id: u64,
        workspace_id: WorkspaceId,
    },
    Failed {
        reason: String,
    },
}

/// Typed prompt submission job dispatched from the Bevy UI thread to the background worker.
#[derive(Debug, Clone)]
pub struct LivePromptRequest {
    pub request_id: u64,
    pub session_id: SessionId,
    pub workspace_id: WorkspaceId,
    pub chat_id: ChatId,
    pub idempotency_key: IdempotencyKey,
    pub prompt: String,
    pub since_event_id: u64,
    pub now_secs: u64,
}

/// Typed command dispatched from the Bevy UI thread to the background HTTP worker.
#[derive(Debug, Clone)]
pub enum LiveWorkerCommand {
    Submit(LivePromptRequest),
    Cancel {
        request_id: u64,
    },
    FetchWorkflowSnapshot {
        session_id: SessionId,
        workspace_id: WorkspaceId,
    },
    UpdateTodayStanding {
        session_id: SessionId,
        item_id: String,
        standing: TodayStandingUpdate,
        version_id: u64,
    },
    IssueConnectedCareInvite {
        session_id: SessionId,
        workspace_id: WorkspaceId,
        patient_id: String,
        expected_dob: String,
    },
}

/// Progressive streaming event, completion, cancellation, workflow sync result, or failure emitted by the worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LivePromptOutcome {
    DeltaChunk {
        request_id: u64,
        workspace_id: WorkspaceId,
        text_delta: String,
        last_event_id: u64,
    },
    StepRecorded {
        request_id: u64,
        workspace_id: WorkspaceId,
        step_label: String,
        last_event_id: u64,
    },
    Completed {
        request_id: u64,
        workspace_id: WorkspaceId,
        assistant_text: String,
        last_event_id: u64,
    },
    Cancelled {
        request_id: u64,
        workspace_id: WorkspaceId,
        last_event_id: u64,
    },
    Failed {
        request_id: u64,
        workspace_id: WorkspaceId,
        detail: String,
    },
    WorkflowSnapshotLoaded {
        workspace_id: WorkspaceId,
        today_items: Vec<RenderedTodayItem>,
        scheduled_visits: Vec<ScheduledVisitEntry>,
        channel_banners: Vec<TodayChannelBanner>,
        memory_entries: Vec<MemoryEntry>,
    },
    TodayStandingUpdated {
        item_id: String,
        standing: TodayStandingUpdate,
        version_id: u64,
    },
    ConnectedCareInviteIssued {
        workspace_id: WorkspaceId,
        invite_code: String,
        invitation_id: String,
    },
    WorkflowFailed {
        detail: String,
    },
}

#[cfg(not(target_arch = "wasm32"))]
struct LiveWorkerChannel {
    tx: Sender<LiveWorkerCommand>,
    rx: Mutex<Receiver<LivePromptOutcome>>,
    cancelled_request_id: Arc<AtomicU64>,
}

/// Bevy [`Resource`] owning the non-blocking background HTTP worker channel.
///
/// Disabled (`None`) under `--demo`, `--headless-smoke`, and `wasm32`.
#[derive(Default, Resource)]
pub struct LivePromptWorker {
    #[cfg(not(target_arch = "wasm32"))]
    channel: Option<LiveWorkerChannel>,
}

impl LivePromptWorker {
    /// Returns a disabled no-op worker (for `--demo`, `--headless-smoke`, and `wasm32`).
    #[must_use]
    pub const fn disabled() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            channel: None,
        }
    }

    /// Spawns a dedicated background OS thread talking to `ef-clinic-server` at `socket_addr`.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn spawn(socket_addr: SocketAddr, timeout: Duration) -> Self {
        let (req_tx, req_rx) = mpsc::channel::<LiveWorkerCommand>();
        let (resp_tx, resp_rx) = mpsc::channel::<LivePromptOutcome>();
        let cancelled_request_id = Arc::new(AtomicU64::new(0));
        let worker_cancel_token = Arc::clone(&cancelled_request_id);

        let _ = std::thread::Builder::new()
            .name("ef-clinic-live-http-worker".to_owned())
            .spawn(move || {
                run_worker_loop(socket_addr, timeout, req_rx, resp_tx, worker_cancel_token);
            });

        Self {
            channel: Some(LiveWorkerChannel {
                tx: req_tx,
                rx: Mutex::new(resp_rx),
                cancelled_request_id,
            }),
        }
    }

    #[cfg(target_arch = "wasm32")]
    #[must_use]
    pub const fn spawn(_socket_addr: SocketAddr, _timeout: Duration) -> Self {
        Self::disabled()
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.channel.is_some()
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    }

    /// Queues a prompt submission without blocking the Bevy UI thread.
    pub fn submit(&self, req: LivePromptRequest) -> Result<(), String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(ref ch) = self.channel else {
                return Err("live background worker is disabled".to_owned());
            };
            let _ = ch.cancelled_request_id.compare_exchange(
                req.request_id,
                0,
                Ordering::SeqCst,
                Ordering::SeqCst,
            );
            ch.tx
                .send(LiveWorkerCommand::Submit(req))
                .map_err(|err| format!("live worker channel closed: {err}"))
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = req;
            Err("live background worker is disabled on wasm32".to_owned())
        }
    }

    /// Signals mid-flight cancellation for `request_id` via both the atomic cancel token
    /// (observed immediately inside the worker's SSE poll loop) and the command channel.
    pub fn cancel(&self, request_id: u64) -> Result<(), String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(ref ch) = self.channel else {
                return Err("live background worker is disabled".to_owned());
            };
            ch.cancelled_request_id.store(request_id, Ordering::SeqCst);
            ch.tx
                .send(LiveWorkerCommand::Cancel { request_id })
                .map_err(|err| format!("live worker channel closed: {err}"))
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = request_id;
            Err("live background worker is disabled on wasm32".to_owned())
        }
    }

    /// Queues a live Today + Longitudinal Memory snapshot fetch without blocking the Bevy UI thread.
    pub fn fetch_workflow_snapshot(
        &self,
        session_id: SessionId,
        workspace_id: WorkspaceId,
    ) -> Result<(), String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(ref ch) = self.channel else {
                return Err("live background worker is disabled".to_owned());
            };
            ch.tx
                .send(LiveWorkerCommand::FetchWorkflowSnapshot {
                    session_id,
                    workspace_id,
                })
                .map_err(|err| format!("live worker channel closed: {err}"))
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (session_id, workspace_id);
            Err("live background worker is disabled on wasm32".to_owned())
        }
    }

    /// Queues a live `POST /elysianfields/api/today/standing` update without blocking the Bevy UI thread.
    pub fn update_today_standing(
        &self,
        session_id: SessionId,
        item_id: String,
        standing: TodayStandingUpdate,
        version_id: u64,
    ) -> Result<(), String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(ref ch) = self.channel else {
                return Err("live background worker is disabled".to_owned());
            };
            ch.tx
                .send(LiveWorkerCommand::UpdateTodayStanding {
                    session_id,
                    item_id,
                    standing,
                    version_id,
                })
                .map_err(|err| format!("live worker channel closed: {err}"))
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (session_id, item_id, standing, version_id);
            Err("live background worker is disabled on wasm32".to_owned())
        }
    }

    /// Queues a live Connected Care invitation request without blocking the Bevy UI thread.
    pub fn issue_connected_care_invite(
        &self,
        session_id: SessionId,
        workspace_id: WorkspaceId,
        patient_id: String,
        expected_dob: String,
    ) -> Result<(), String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(ref ch) = self.channel else {
                return Err("live background worker is disabled".to_owned());
            };
            ch.tx
                .send(LiveWorkerCommand::IssueConnectedCareInvite {
                    session_id,
                    workspace_id,
                    patient_id,
                    expected_dob,
                })
                .map_err(|err| format!("live worker channel closed: {err}"))
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (session_id, workspace_id, patient_id, expected_dob);
            Err("live background worker is disabled on wasm32".to_owned())
        }
    }

    /// Non-blocking poll for progressive deltas, steps, completion, cancellation, or failure.
    #[must_use]
    pub fn try_recv(&self) -> Option<LivePromptOutcome> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let ch = self.channel.as_ref()?;
            let guard = ch.rx.lock().ok()?;
            guard.try_recv().ok()
        }
        #[cfg(target_arch = "wasm32")]
        {
            None
        }
    }

    /// Bounded blocking wait used exclusively by integration tests to await a worker event.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn recv_timeout(&self, timeout: Duration) -> Option<LivePromptOutcome> {
        let ch = self.channel.as_ref()?;
        let guard = ch.rx.lock().ok()?;
        guard.recv_timeout(timeout).ok()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn format_exchange_step(step: &ef_clinic_domain::chat::ExchangeStep) -> String {
    use ef_clinic_domain::chat::ExchangeStep;
    match step {
        ExchangeStep::SearchedChart { query, .. } => format!("Searched chart ({query})"),
        ExchangeStep::ReadRecords { requested, .. } => {
            format!("Read {} record(s)", requested.len())
        }
        ExchangeStep::ReadConversation { .. } => "Read conversation".to_owned(),
        ExchangeStep::SavedMemoryNote { .. } => "Saved memory note".to_owned(),
        ExchangeStep::EditedMemory { .. } => "Edited memory".to_owned(),
        ExchangeStep::CreatedFollowUp { .. } => "Created follow-up".to_owned(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy)]
enum HttpVerb {
    Get,
    Post,
}

#[cfg(not(target_arch = "wasm32"))]
impl HttpVerb {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Deserialize)]
struct TodayApiPayload {
    #[serde(default)]
    items: Vec<RenderedTodayItem>,
    #[serde(default)]
    scheduled_visits: Vec<ScheduledVisitEntry>,
    #[serde(default)]
    channel_banners: Vec<TodayChannelBanner>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Deserialize)]
struct MemoryApiPayload {
    #[serde(default)]
    entries: Vec<MemoryEntry>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Serialize)]
struct TodayStandingRequestPayload<'a> {
    item_id: &'a str,
    standing: TodayStandingUpdate,
    version_id: u64,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Deserialize)]
struct TodayStandingResponsePayload {
    item_id: String,
    standing: TodayStandingUpdate,
    version_id: u64,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Serialize)]
struct IssueInviteRequestPayload<'a> {
    patient_id: &'a str,
    expected_dob: &'a str,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Deserialize)]
struct IssueInviteResponsePayload {
    invitation_id: String,
    invite_code: String,
}

#[cfg(not(target_arch = "wasm32"))]
fn http_json_request(
    socket_addr: SocketAddr,
    timeout: Duration,
    verb: HttpVerb,
    path: &str,
    session_id: &SessionId,
    body_json: Option<&str>,
) -> Result<(u16, String), String> {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    let mut stream = TcpStream::connect_timeout(&socket_addr, timeout)
        .map_err(|err| format!("failed to connect to {socket_addr}: {err}"))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|err| format!("failed to set read timeout: {err}"))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|err| format!("failed to set write timeout: {err}"))?;

    let request = if let Some(body) = body_json {
        format!(
            "{} {path} HTTP/1.1\r\nHost: {socket_addr}\r\nAuthorization: Bearer {}\r\nAccept: application/json\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            verb.as_str(),
            session_id.as_str(),
            body.len()
        )
    } else {
        format!(
            "{} {path} HTTP/1.1\r\nHost: {socket_addr}\r\nAuthorization: Bearer {}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
            verb.as_str(),
            session_id.as_str()
        )
    };

    stream
        .write_all(request.as_bytes())
        .map_err(|err| format!("failed to send {path} request: {err}"))?;
    stream
        .flush()
        .map_err(|err| format!("failed to flush {path} request: {err}"))?;

    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|err| format!("failed to read {path} response: {err}"))?;

    let separator = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| format!("malformed HTTP response from {path}: missing header terminator"))?;
    let head = String::from_utf8_lossy(&raw[..separator]);
    let status_line = head
        .lines()
        .next()
        .ok_or_else(|| format!("empty HTTP status line from {path}"))?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| format!("missing HTTP status code in '{status_line}'"))?
        .parse()
        .map_err(|err| format!("invalid HTTP status code in '{status_line}': {err}"))?;
    let body = String::from_utf8(raw[separator + 4..].to_vec())
        .map_err(|err| format!("non-UTF-8 response body from {path}: {err}"))?;

    Ok((status, body))
}

#[cfg(not(target_arch = "wasm32"))]
fn execute_fetch_workflow_snapshot(
    socket_addr: SocketAddr,
    timeout: Duration,
    session_id: &SessionId,
    workspace_id: WorkspaceId,
) -> LivePromptOutcome {
    let today_path = "/elysianfields/api/today";
    let (today_status, today_body) = match http_json_request(
        socket_addr,
        timeout,
        HttpVerb::Get,
        today_path,
        session_id,
        None,
    ) {
        Ok(resp) => resp,
        Err(detail) => return LivePromptOutcome::WorkflowFailed { detail },
    };
    if today_status != 200 {
        return LivePromptOutcome::WorkflowFailed {
            detail: format!("GET {today_path} returned HTTP {today_status}: {today_body}"),
        };
    }
    let today_payload: TodayApiPayload = match serde_json::from_str(&today_body) {
        Ok(parsed) => parsed,
        Err(err) => {
            return LivePromptOutcome::WorkflowFailed {
                detail: format!("failed to parse Today response JSON: {err}"),
            };
        }
    };

    let memory_path = format!(
        "/elysianfields/api/workspaces/{}/memory",
        workspace_id.as_str()
    );
    let (mem_status, mem_body) = match http_json_request(
        socket_addr,
        timeout,
        HttpVerb::Get,
        &memory_path,
        session_id,
        None,
    ) {
        Ok(resp) => resp,
        Err(detail) => return LivePromptOutcome::WorkflowFailed { detail },
    };
    if mem_status != 200 {
        return LivePromptOutcome::WorkflowFailed {
            detail: format!("GET {memory_path} returned HTTP {mem_status}: {mem_body}"),
        };
    }
    let mem_payload: MemoryApiPayload = match serde_json::from_str(&mem_body) {
        Ok(parsed) => parsed,
        Err(err) => {
            return LivePromptOutcome::WorkflowFailed {
                detail: format!("failed to parse memory response JSON: {err}"),
            };
        }
    };

    LivePromptOutcome::WorkflowSnapshotLoaded {
        workspace_id,
        today_items: today_payload.items,
        scheduled_visits: today_payload.scheduled_visits,
        channel_banners: today_payload.channel_banners,
        memory_entries: mem_payload.entries,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn execute_update_today_standing(
    socket_addr: SocketAddr,
    timeout: Duration,
    session_id: &SessionId,
    item_id: &str,
    standing: TodayStandingUpdate,
    version_id: u64,
) -> LivePromptOutcome {
    let path = "/elysianfields/api/today/standing";
    let payload = TodayStandingRequestPayload {
        item_id,
        standing,
        version_id,
    };
    let body_json = match serde_json::to_string(&payload) {
        Ok(serialized) => serialized,
        Err(err) => {
            return LivePromptOutcome::WorkflowFailed {
                detail: format!("failed to encode standing payload: {err}"),
            };
        }
    };
    let (status, resp_body) = match http_json_request(
        socket_addr,
        timeout,
        HttpVerb::Post,
        path,
        session_id,
        Some(&body_json),
    ) {
        Ok(resp) => resp,
        Err(detail) => return LivePromptOutcome::WorkflowFailed { detail },
    };
    if status != 200 {
        return LivePromptOutcome::WorkflowFailed {
            detail: format!("POST {path} returned HTTP {status}: {resp_body}"),
        };
    }
    match serde_json::from_str::<TodayStandingResponsePayload>(&resp_body) {
        Ok(parsed) => LivePromptOutcome::TodayStandingUpdated {
            item_id: parsed.item_id,
            standing: parsed.standing,
            version_id: parsed.version_id,
        },
        Err(err) => LivePromptOutcome::WorkflowFailed {
            detail: format!("failed to parse standing response JSON: {err}"),
        },
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn execute_issue_connected_care_invite(
    socket_addr: SocketAddr,
    timeout: Duration,
    session_id: &SessionId,
    workspace_id: WorkspaceId,
    patient_id: &str,
    expected_dob: &str,
) -> LivePromptOutcome {
    let path = format!(
        "/elysianfields/api/workspaces/{}/connected-care/invitations",
        workspace_id.as_str()
    );
    let payload = IssueInviteRequestPayload {
        patient_id,
        expected_dob,
    };
    let body_json = match serde_json::to_string(&payload) {
        Ok(serialized) => serialized,
        Err(err) => {
            return LivePromptOutcome::WorkflowFailed {
                detail: format!("failed to encode invite payload: {err}"),
            };
        }
    };
    let (status, resp_body) = match http_json_request(
        socket_addr,
        timeout,
        HttpVerb::Post,
        &path,
        session_id,
        Some(&body_json),
    ) {
        Ok(resp) => resp,
        Err(detail) => return LivePromptOutcome::WorkflowFailed { detail },
    };
    if status != 201 && status != 200 {
        return LivePromptOutcome::WorkflowFailed {
            detail: format!("POST {path} returned HTTP {status}: {resp_body}"),
        };
    }
    match serde_json::from_str::<IssueInviteResponsePayload>(&resp_body) {
        Ok(parsed) => LivePromptOutcome::ConnectedCareInviteIssued {
            workspace_id,
            invite_code: parsed.invite_code,
            invitation_id: parsed.invitation_id,
        },
        Err(err) => LivePromptOutcome::WorkflowFailed {
            detail: format!("failed to parse invitation response JSON: {err}"),
        },
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn is_cancelled(
    request_id: u64,
    cancelled_request_id: &AtomicU64,
    req_rx: &Receiver<LiveWorkerCommand>,
    deferred_commands: &mut std::collections::VecDeque<LiveWorkerCommand>,
) -> bool {
    while let Ok(cmd) = req_rx.try_recv() {
        match cmd {
            LiveWorkerCommand::Cancel {
                request_id: cancel_id,
            } => {
                cancelled_request_id.store(cancel_id, Ordering::SeqCst);
            }
            other => {
                deferred_commands.push_back(other);
            }
        }
    }
    cancelled_request_id.load(Ordering::SeqCst) == request_id
}

#[cfg(not(target_arch = "wasm32"))]
fn send_server_stop_command(
    transport: &mut ef_clinic_client::LiveHttpClinicTransport,
    req: &LivePromptRequest,
    work_item_id: ef_core::WorkItemId,
) {
    use ef_clinic_client::ClinicTransport;
    use ef_clinic_domain::chat::{CommandEnvelope, CommandPayload, StopWorkPayload};

    if let Ok(cancel_key) = IdempotencyKey::new(format!("desktop-cancel-{}", req.request_id)) {
        let stop_envelope = CommandEnvelope {
            idempotency_key: cancel_key,
            command: CommandPayload::WorkStop(StopWorkPayload {
                patient_workspace_id: req.workspace_id.clone(),
                chat_id: req.chat_id.clone(),
                work_item_id,
            }),
        };
        let _ = transport.submit_command(&req.session_id, &stop_envelope, req.now_secs);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn run_worker_loop(
    socket_addr: SocketAddr,
    timeout: Duration,
    req_rx: Receiver<LiveWorkerCommand>,
    resp_tx: Sender<LivePromptOutcome>,
    cancelled_request_id: Arc<AtomicU64>,
) {
    use std::collections::VecDeque;

    use ef_clinic_client::{ClinicTransport, LiveHttpClinicTransport};
    use ef_clinic_domain::chat::{
        ChatStreamEvent, CommandEnvelope, CommandPayload, SubmitMessagePayload,
    };

    let mut transport = LiveHttpClinicTransport::new(socket_addr);
    transport.set_timeout(timeout);
    let mut deferred_commands: VecDeque<LiveWorkerCommand> = VecDeque::new();

    loop {
        let cmd = if let Some(queued) = deferred_commands.pop_front() {
            queued
        } else {
            match req_rx.recv() {
                Ok(cmd) => cmd,
                Err(_) => break,
            }
        };

        let req = match cmd {
            LiveWorkerCommand::Submit(req) => req,
            LiveWorkerCommand::Cancel { request_id } => {
                cancelled_request_id.store(request_id, Ordering::SeqCst);
                continue;
            }
            LiveWorkerCommand::FetchWorkflowSnapshot {
                session_id,
                workspace_id,
            } => {
                let outcome = execute_fetch_workflow_snapshot(
                    socket_addr,
                    timeout,
                    &session_id,
                    workspace_id,
                );
                if resp_tx.send(outcome).is_err() {
                    break;
                }
                continue;
            }
            LiveWorkerCommand::UpdateTodayStanding {
                session_id,
                item_id,
                standing,
                version_id,
            } => {
                let outcome = execute_update_today_standing(
                    socket_addr,
                    timeout,
                    &session_id,
                    &item_id,
                    standing,
                    version_id,
                );
                if resp_tx.send(outcome).is_err() {
                    break;
                }
                continue;
            }
            LiveWorkerCommand::IssueConnectedCareInvite {
                session_id,
                workspace_id,
                patient_id,
                expected_dob,
            } => {
                let outcome = execute_issue_connected_care_invite(
                    socket_addr,
                    timeout,
                    &session_id,
                    workspace_id,
                    &patient_id,
                    &expected_dob,
                );
                if resp_tx.send(outcome).is_err() {
                    break;
                }
                continue;
            }
        };

        if is_cancelled(
            req.request_id,
            &cancelled_request_id,
            &req_rx,
            &mut deferred_commands,
        ) {
            if resp_tx
                .send(LivePromptOutcome::Cancelled {
                    request_id: req.request_id,
                    workspace_id: req.workspace_id,
                    last_event_id: req.since_event_id,
                })
                .is_err()
            {
                break;
            }
            continue;
        }

        let envelope = CommandEnvelope {
            idempotency_key: req.idempotency_key.clone(),
            command: CommandPayload::MessageSubmit(SubmitMessagePayload {
                patient_workspace_id: req.workspace_id.clone(),
                chat_id: req.chat_id.clone(),
                text: req.prompt.clone(),
                physician_time_zone: "UTC".to_owned(),
                delivery: None,
            }),
        };

        let acceptance = match transport.submit_command(&req.session_id, &envelope, req.now_secs) {
            Ok(acc) => acc,
            Err(err) => {
                if resp_tx
                    .send(LivePromptOutcome::Failed {
                        request_id: req.request_id,
                        workspace_id: req.workspace_id,
                        detail: err.to_string(),
                    })
                    .is_err()
                {
                    break;
                }
                continue;
            }
        };

        let work_item_id = acceptance.work_item_id;
        let mut since_id = req.since_event_id;
        let mut assistant_text = String::new();
        let mut terminal_reached = false;
        let mut was_cancelled = false;
        let mut failure_detail: Option<String> = None;

        for _attempt in 0..100 {
            if is_cancelled(
                req.request_id,
                &cancelled_request_id,
                &req_rx,
                &mut deferred_commands,
            ) {
                send_server_stop_command(&mut transport, &req, work_item_id.clone());
                was_cancelled = true;
                break;
            }

            match transport.poll_stream_events(&req.session_id, since_id, req.now_secs) {
                Ok(events) => {
                    for event in events {
                        if is_cancelled(
                            req.request_id,
                            &cancelled_request_id,
                            &req_rx,
                            &mut deferred_commands,
                        ) {
                            send_server_stop_command(&mut transport, &req, work_item_id.clone());
                            was_cancelled = true;
                            break;
                        }

                        match event {
                            ChatStreamEvent::DeltaChunk {
                                event_id,
                                text_delta,
                                ..
                            } => {
                                since_id = since_id.max(event_id);
                                assistant_text.push_str(&text_delta);
                                if resp_tx
                                    .send(LivePromptOutcome::DeltaChunk {
                                        request_id: req.request_id,
                                        workspace_id: req.workspace_id.clone(),
                                        text_delta,
                                        last_event_id: since_id,
                                    })
                                    .is_err()
                                {
                                    return;
                                }
                            }
                            ChatStreamEvent::StepRecorded { event_id, step, .. } => {
                                since_id = since_id.max(event_id);
                                let step_label = format_exchange_step(&step);
                                if resp_tx
                                    .send(LivePromptOutcome::StepRecorded {
                                        request_id: req.request_id,
                                        workspace_id: req.workspace_id.clone(),
                                        step_label,
                                        last_event_id: since_id,
                                    })
                                    .is_err()
                                {
                                    return;
                                }
                            }
                            ChatStreamEvent::Completed { event_id, .. } => {
                                since_id = since_id.max(event_id);
                                terminal_reached = true;
                            }
                            ChatStreamEvent::Cancelled { event_id, .. } => {
                                since_id = since_id.max(event_id);
                                was_cancelled = true;
                                terminal_reached = true;
                            }
                            ChatStreamEvent::CouldntFinish {
                                event_id, attempts, ..
                            } => {
                                since_id = since_id.max(event_id);
                                failure_detail = Some(format!(
                                    "server could not finish response after {attempts} attempt(s)"
                                ));
                                terminal_reached = true;
                            }
                            ChatStreamEvent::ResyncRequired { event_id, reason } => {
                                since_id = since_id.max(event_id);
                                failure_detail = Some(format!("stream resync required: {reason}"));
                                terminal_reached = true;
                            }
                            ChatStreamEvent::KeepAlive => {}
                        }
                    }

                    if was_cancelled || terminal_reached {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(err) => {
                    if is_cancelled(
                        req.request_id,
                        &cancelled_request_id,
                        &req_rx,
                        &mut deferred_commands,
                    ) {
                        was_cancelled = true;
                    } else {
                        failure_detail = Some(err.to_string());
                    }
                    break;
                }
            }
        }

        let outcome = if was_cancelled {
            LivePromptOutcome::Cancelled {
                request_id: req.request_id,
                workspace_id: req.workspace_id,
                last_event_id: since_id,
            }
        } else if let Some(detail) = failure_detail {
            LivePromptOutcome::Failed {
                request_id: req.request_id,
                workspace_id: req.workspace_id,
                detail,
            }
        } else {
            let final_text = if assistant_text.trim().is_empty() {
                "Response recorded on live clinic server.".to_owned()
            } else {
                assistant_text
            };
            LivePromptOutcome::Completed {
                request_id: req.request_id,
                workspace_id: req.workspace_id,
                assistant_text: final_text,
                last_event_id: since_id,
            }
        };

        if resp_tx.send(outcome).is_err() {
            break;
        }
    }
}
