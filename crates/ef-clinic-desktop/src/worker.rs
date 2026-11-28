//! Non-blocking background HTTP worker for live `--server <url>` prompt submission.
//!
//! Executes `LiveHttpClinicTransport` command submission and SSE polling on a dedicated OS thread
//! communicating over `std::sync::mpsc` so the Bevy desktop UI thread never blocks on network I/O.

use std::net::SocketAddr;
use std::sync::Mutex;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use bevy::prelude::Resource;
use ef_core::{ChatId, IdempotencyKey, SessionId, WorkspaceId};

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

/// Typed completion or failure returned by the background worker over `mpsc::Receiver`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LivePromptOutcome {
    Completed {
        request_id: u64,
        workspace_id: WorkspaceId,
        assistant_text: String,
        last_event_id: u64,
    },
    Failed {
        request_id: u64,
        workspace_id: WorkspaceId,
        detail: String,
    },
}

struct LiveWorkerChannel {
    tx: Sender<LivePromptRequest>,
    rx: Mutex<Receiver<LivePromptOutcome>>,
}

/// Bevy [`Resource`] owning the non-blocking background HTTP worker channel.
///
/// Disabled (`None`) under `--demo`, `--headless-smoke`, and `wasm32`.
#[derive(Default, Resource)]
pub struct LivePromptWorker {
    channel: Option<LiveWorkerChannel>,
}

impl LivePromptWorker {
    /// Returns a disabled no-op worker (for `--demo`, `--headless-smoke`, and `wasm32`).
    #[must_use]
    pub const fn disabled() -> Self {
        Self { channel: None }
    }

    /// Spawns a dedicated background OS thread talking to `ef-clinic-server` at `socket_addr`.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn spawn(socket_addr: SocketAddr, timeout: Duration) -> Self {
        let (req_tx, req_rx) = mpsc::channel::<LivePromptRequest>();
        let (resp_tx, resp_rx) = mpsc::channel::<LivePromptOutcome>();

        let _ = std::thread::Builder::new()
            .name("ef-clinic-live-http-worker".to_owned())
            .spawn(move || {
                run_worker_loop(socket_addr, timeout, req_rx, resp_tx);
            });

        Self {
            channel: Some(LiveWorkerChannel {
                tx: req_tx,
                rx: Mutex::new(resp_rx),
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
        self.channel.is_some()
    }

    /// Queues a prompt submission without blocking the Bevy UI thread.
    pub fn submit(&self, req: LivePromptRequest) -> Result<(), String> {
        let Some(ref ch) = self.channel else {
            return Err("live background worker is disabled".to_owned());
        };
        ch.tx
            .send(req)
            .map_err(|err| format!("live worker channel closed: {err}"))
    }

    /// Non-blocking poll for completed or failed outcomes (called once per Bevy frame).
    #[must_use]
    pub fn try_recv(&self) -> Option<LivePromptOutcome> {
        let ch = self.channel.as_ref()?;
        let guard = ch.rx.lock().ok()?;
        guard.try_recv().ok()
    }

    /// Bounded blocking wait used exclusively by integration tests to await a worker response.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn recv_timeout(&self, timeout: Duration) -> Option<LivePromptOutcome> {
        let ch = self.channel.as_ref()?;
        let guard = ch.rx.lock().ok()?;
        guard.recv_timeout(timeout).ok()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn run_worker_loop(
    socket_addr: SocketAddr,
    timeout: Duration,
    req_rx: Receiver<LivePromptRequest>,
    resp_tx: Sender<LivePromptOutcome>,
) {
    use ef_clinic_client::{ClinicTransport, LiveHttpClinicTransport};
    use ef_clinic_domain::chat::{
        ChatStreamEvent, CommandEnvelope, CommandPayload, SubmitMessagePayload,
    };

    let mut transport = LiveHttpClinicTransport::new(socket_addr);
    transport.set_timeout(timeout);

    while let Ok(req) = req_rx.recv() {
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

        if let Err(err) = transport.submit_command(&req.session_id, &envelope, req.now_secs) {
            let _ = resp_tx.send(LivePromptOutcome::Failed {
                request_id: req.request_id,
                workspace_id: req.workspace_id,
                detail: err.to_string(),
            });
            continue;
        }

        let mut since_id = req.since_event_id;
        let mut assistant_text = String::new();
        let mut terminal_reached = false;
        let mut failure_detail: Option<String> = None;

        for _attempt in 0..30 {
            match transport.poll_stream_events(&req.session_id, since_id, req.now_secs) {
                Ok(events) => {
                    for event in events {
                        match event {
                            ChatStreamEvent::DeltaChunk {
                                event_id,
                                text_delta,
                                ..
                            } => {
                                since_id = since_id.max(event_id);
                                assistant_text.push_str(&text_delta);
                            }
                            ChatStreamEvent::StepRecorded { event_id, .. } => {
                                since_id = since_id.max(event_id);
                            }
                            ChatStreamEvent::Completed { event_id, .. } => {
                                since_id = since_id.max(event_id);
                                terminal_reached = true;
                            }
                            ChatStreamEvent::Cancelled { event_id, .. } => {
                                since_id = since_id.max(event_id);
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
                    if terminal_reached {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(err) => {
                    failure_detail = Some(err.to_string());
                    break;
                }
            }
        }

        let outcome = if let Some(detail) = failure_detail {
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
