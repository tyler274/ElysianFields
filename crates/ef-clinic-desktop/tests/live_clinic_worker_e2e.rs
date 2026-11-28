#![cfg(not(target_arch = "wasm32"))]

//! Live TCP integration tests proving progressive SSE `DeltaChunk` delivery,
//! `StepRecorded`, `Completed`, mid-flight `Cancel` (`CommandPayload::WorkStop`),
//! and Clinic Desktop streaming draft + Escape cancel + Retry UI behavior against
//! a real `ef-clinic-server` (`TcpClinicServer`).

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bevy::asset::AssetPlugin;
use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyCode, KeyboardInput};
use bevy::prelude::*;
use ef_clinic_client::{
    ClientBootConfig, ClientClock, ClinicTransport, CurieClinicClientPlugin,
    LiveHttpClinicTransport, SharedBrowserSessionHub, SharedLocaleStorage, SignInRequest,
    VerifySignInRequest,
};
use ef_clinic_desktop::{
    ClinicDesktopUiPlugin, ClinicWorkflowState, ClinicWorkspaceTab, DesktopClipboard,
    DesktopInputFocus, DesktopLaunchConfig, DesktopStatusBanner, DesktopUiEvent, DesktopUiSlot,
    DesktopWorkspaceState, LaunchBackend, LivePromptOutcome, LivePromptRequest, LivePromptWorker,
    LiveReplyOrigin, LiveSubmissionState, TodayStandingUpdate, ZoomLevel, bootstrap_clinic_session,
    create_transport_for_backend,
};
use ef_clinic_domain::auth::{
    AccountStanding, EmailAddress, PhysicianAccount, PhysicianRole, SignInMode,
};
use ef_clinic_domain::chat::RetryBackoffMode;
use ef_clinic_domain::workspace::{
    CitationOrigin, CitedSourcePayload, ReadablePart, classify_record_status,
};
use ef_clinic_server::{ClinicService, TcpClinicServer};
use ef_clinic_workflows::memory::MemoryAuthority;
use ef_clinic_workflows::today::TodayUrgency;
use ef_connected_care::ConnectedCareState;
use ef_core::{
    ChatId, IdempotencyKey, Locale as CoreLocale, PhysicianId, PracticeId, SourceKey, TabId,
    WorkspaceId,
};
use ef_ui_catalog::{Locale, ThemeMode, UiKey, translate};

struct LiveServerFixture {
    server: TcpClinicServer,
    wal_dir: PathBuf,
    email: String,
    workspace_id: WorkspaceId,
    chat_id: ChatId,
    now_secs: u64,
}

impl LiveServerFixture {
    fn spawn(label: &str) -> Self {
        Self::spawn_inner(label, false)
    }

    fn spawn_with_workflows(label: &str) -> Self {
        Self::spawn_inner(label, true)
    }

    fn spawn_inner(label: &str, seed_workflows: bool) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let wal_dir = std::env::temp_dir().join(format!(
            "ef-clinic-worker-e2e-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&wal_dir).expect("create temp wal dir");

        let now_secs = 1_760_300_000u64;
        let src_key = SourceKey::new("src-stream-e2e-1").expect("src key");
        let mut service = ClinicService::open_durable(
            &wal_dir,
            SignInMode::LocalProof,
            RetryBackoffMode::DeterministicTest,
            src_key.clone(),
            now_secs,
        )
        .expect("open durable clinic service");

        let practice = PracticeId::new("prac-stream-e2e").expect("practice");
        let doc = PhysicianId::new("phys-stream-e2e").expect("doc");
        let email = "stream.e2e@example.invalid".to_owned();
        service
            .seed_account(PhysicianAccount {
                physician_id: doc.clone(),
                practice_id: practice.clone(),
                email: EmailAddress::parse(&email).expect("email"),
                display_name: "Dr. Stream E2E".to_owned(),
                role: PhysicianRole::Standard,
                standing: AccountStanding::Active,
                sessions_ended_before: None,
            })
            .expect("seed account");

        let workspace_id = WorkspaceId::new("ws-stream-e2e").expect("ws");
        let chat_id = ChatId::new("chat-default").expect("chat");
        service
            .seed_workspace(
                workspace_id.clone(),
                practice,
                "Avery, Jordan (Streaming E2E)",
                vec![doc],
            )
            .expect("seed workspace");
        service
            .seed_source(CitedSourcePayload {
                source_key: src_key,
                workspace_id: workspace_id.clone(),
                number: 1,
                source_label: "Streaming Note [1]".to_owned(),
                title: "Outpatient Glycemic Review".to_owned(),
                record_type: "progress_note".to_owned(),
                recorded_at: "2026-10-18T12:00:00Z".to_owned(),
                record_status: "final".to_owned(),
                status_class: classify_record_status("final"),
                origin: CitationOrigin::SourceRecord,
                original_language: CoreLocale::En,
                quote: "HbA1c 6.8% stable on Metformin 1000 mg BID.".to_owned(),
                body: "Synthetic streaming verification note.".to_owned(),
                readable_parts: vec![ReadablePart {
                    label: "Plan".to_owned(),
                    text: "Continue current therapy.".to_owned(),
                    unreadable_notice: None,
                }],
                fields: Vec::new(),
                raw_content: None,
                external_url: None,
            })
            .expect("seed citation");

        if seed_workflows {
            service
                .seed_demo_workflows(&workspace_id, "Avery, Jordan (Streaming E2E)")
                .expect("seed demo workflows");
        }

        let server = TcpClinicServer::bind("127.0.0.1:0", service).expect("bind live server");
        Self {
            server,
            wal_dir,
            email,
            workspace_id,
            chat_id,
            now_secs,
        }
    }

    fn addr(&self) -> SocketAddr {
        self.server.local_addr()
    }

    fn sign_in_session(&self) -> ef_core::SessionId {
        let mut transport = LiveHttpClinicTransport::new(self.addr());
        let init = transport
            .initiate_sign_in(
                &SignInRequest {
                    email: self.email.clone(),
                },
                self.now_secs,
            )
            .expect("initiate sign-in");
        let token = match init {
            ef_clinic_domain::auth::SignInInitiationResponse::TokenIssued { token, .. } => token,
            other => panic!("expected local proof TokenIssued, got {other:?}"),
        };
        let verified = transport
            .verify_sign_in(
                &VerifySignInRequest {
                    email: self.email.clone(),
                    token,
                },
                self.now_secs,
            )
            .expect("verify sign-in");
        verified.session
    }
}

impl Drop for LiveServerFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.wal_dir);
    }
}

#[test]
fn live_worker_streams_progressive_delta_chunks_steps_and_completion_over_tcp() {
    let fixture = LiveServerFixture::spawn("progressive");
    let session_id = fixture.sign_in_session();
    let worker = LivePromptWorker::spawn(fixture.addr(), Duration::from_secs(5));

    worker
        .submit(LivePromptRequest {
            request_id: 1,
            session_id,
            workspace_id: fixture.workspace_id.clone(),
            chat_id: fixture.chat_id.clone(),
            idempotency_key: IdempotencyKey::new("e2e-progressive-1").expect("key"),
            prompt: "Summarize HbA1c trend and active medications.".to_owned(),
            since_event_id: 0,
            now_secs: fixture.now_secs,
        })
        .expect("submit prompt");

    let mut deltas = Vec::new();
    let mut steps = Vec::new();
    let mut last_seen_event_id = 0u64;
    let mut completed_text: Option<String> = None;

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let Some(outcome) = worker.recv_timeout(Duration::from_secs(2)) else {
            panic!("timed out waiting for progressive worker events");
        };
        match outcome {
            LivePromptOutcome::DeltaChunk {
                request_id,
                workspace_id,
                text_delta,
                last_event_id,
            } => {
                assert_eq!(request_id, 1);
                assert_eq!(workspace_id, fixture.workspace_id);
                assert!(
                    last_event_id > last_seen_event_id,
                    "SSE event IDs must advance strictly monotonically"
                );
                last_seen_event_id = last_event_id;
                deltas.push(text_delta);
            }
            LivePromptOutcome::StepRecorded {
                request_id,
                workspace_id,
                step_label,
                last_event_id,
            } => {
                assert_eq!(request_id, 1);
                assert_eq!(workspace_id, fixture.workspace_id);
                assert!(!step_label.is_empty());
                assert!(last_event_id >= last_seen_event_id);
                last_seen_event_id = last_event_id;
                steps.push(step_label);
            }
            LivePromptOutcome::Completed {
                request_id,
                workspace_id,
                assistant_text,
                last_event_id,
            } => {
                assert_eq!(request_id, 1);
                assert_eq!(workspace_id, fixture.workspace_id);
                assert!(last_event_id >= last_seen_event_id);
                completed_text = Some(assistant_text);
                break;
            }
            other => panic!("unexpected outcome during progressive streaming: {other:?}"),
        }
    }

    assert!(
        deltas.len() >= 2,
        "expected progressive multi-chunk DeltaChunk emission before Completed, got {deltas:?}"
    );
    assert!(
        !steps.is_empty(),
        "expected at least one StepRecorded event during live turn"
    );
    let final_text = completed_text.expect("expected Completed outcome");
    assert_eq!(deltas.concat(), final_text);
}

#[test]
fn live_worker_mid_flight_cancellation_sends_work_stop_and_emits_cancelled() {
    let fixture = LiveServerFixture::spawn("cancel");
    let session_id = fixture.sign_in_session();
    let upstream_addr = fixture.addr();

    // Lightweight loopback TCP relay forwarding commands directly to the real `TcpClinicServer`,
    // while pacing the SSE stream so poll #1 yields only the first live `DeltaChunk` (keeping the
    // worker mid-flight) so we deterministically verify mid-flight `worker.cancel(req_id)` sends
    // `work.stop` (`CommandPayload::WorkStop`) and emits `LivePromptOutcome::Cancelled`.
    let relay_listener = TcpListener::bind("127.0.0.1:0").expect("bind relay");
    let relay_addr = relay_listener.local_addr().expect("relay addr");
    let stop_relay = Arc::new(AtomicBool::new(false));
    let work_stop_observed = Arc::new(AtomicBool::new(false));
    let stream_polls = Arc::new(AtomicUsize::new(0));
    let recorded_commands = Arc::new(Mutex::new(Vec::<String>::new()));

    let stop_flag = Arc::clone(&stop_relay);
    let work_stop_flag = Arc::clone(&work_stop_observed);
    let poll_counter = Arc::clone(&stream_polls);
    let cmd_log = Arc::clone(&recorded_commands);

    let relay_thread = thread::spawn(move || {
        for conn in relay_listener.incoming() {
            if stop_flag.load(Ordering::SeqCst) {
                break;
            }
            let Ok(mut client_stream) = conn else {
                continue;
            };
            let Some((raw_request, target, body_text)) = read_http_request(&client_stream) else {
                continue;
            };

            if target.contains("/commands") {
                if let Ok(mut guard) = cmd_log.lock() {
                    guard.push(body_text.clone());
                }
                if body_text.contains("\"work.stop\"") {
                    work_stop_flag.store(true, Ordering::SeqCst);
                }
            }

            let upstream_resp = forward_http_raw(upstream_addr, &raw_request);

            // On the SSE stream route, drip only the first `delta_chunk` frame on poll #1 and
            // keep-alive on subsequent polls until cancelled so the worker remains mid-flight.
            let response_to_send = if target.contains("/patient-workspaces/events") {
                let poll_idx = poll_counter.fetch_add(1, Ordering::SeqCst);
                if poll_idx == 0 {
                    extract_first_delta_sse_response(&upstream_resp)
                } else {
                    let body = ": keep-alive\n\n";
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .into_bytes()
                }
            } else {
                upstream_resp
            };

            let _ = client_stream.write_all(&response_to_send);
            let _ = client_stream.flush();
            let _ = client_stream.shutdown(Shutdown::Write);
        }
    });

    let worker = LivePromptWorker::spawn(relay_addr, Duration::from_secs(5));
    worker
        .submit(LivePromptRequest {
            request_id: 7,
            session_id,
            workspace_id: fixture.workspace_id.clone(),
            chat_id: fixture.chat_id.clone(),
            idempotency_key: IdempotencyKey::new("e2e-cancel-7").expect("key"),
            prompt: "Stream glycemic trend and cancel mid-flight.".to_owned(),
            since_event_id: 0,
            now_secs: fixture.now_secs,
        })
        .expect("submit prompt");

    // 1. Wait for the first live DeltaChunk to arrive progressively BEFORE cancellation.
    let first = worker
        .recv_timeout(Duration::from_secs(5))
        .expect("expected first progressive DeltaChunk");
    let first_event_id = match first {
        LivePromptOutcome::DeltaChunk {
            request_id,
            workspace_id,
            text_delta,
            last_event_id,
        } => {
            assert_eq!(request_id, 7);
            assert_eq!(workspace_id, fixture.workspace_id);
            assert!(!text_delta.is_empty());
            last_event_id
        }
        other => panic!("expected first event to be DeltaChunk, got {other:?}"),
    };

    // 2. Trigger mid-flight cancellation while the worker is polling for remaining SSE events.
    worker.cancel(7).expect("cancel in-flight request");

    let next = worker
        .recv_timeout(Duration::from_secs(5))
        .expect("expected Cancelled outcome after worker.cancel(7)");
    match next {
        LivePromptOutcome::Cancelled {
            request_id,
            workspace_id,
            last_event_id,
        } => {
            assert_eq!(request_id, 7);
            assert_eq!(workspace_id, fixture.workspace_id);
            assert!(last_event_id >= first_event_id);
        }
        other => panic!("expected Cancelled outcome after cancel(7), got {other:?}"),
    }

    assert!(
        work_stop_observed.load(Ordering::SeqCst),
        "expected worker cancellation to submit `work.stop` to server; observed commands: {:?}",
        recorded_commands.lock().ok()
    );

    stop_relay.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect(relay_addr);
    let _ = relay_thread.join();
}

#[test]
fn clinic_desktop_ui_handles_streaming_draft_escape_cancel_and_retry_button() {
    let fixture = LiveServerFixture::spawn("ui-retry-cancel");
    let addr = fixture.addr();
    let base_url = format!("http://{addr}");

    let cfg = DesktopLaunchConfig {
        backend: LaunchBackend::Server {
            base_url,
            socket_addr: addr,
            email: Some(fixture.email.clone()),
            token: None,
        },
        locale: Locale::En,
        theme: ThemeMode::Dark,
        zoom: ZoomLevel::Pct100,
        ..DesktopLaunchConfig::default()
    };
    let (transport, conn_mode, boot_email, boot_token, patients, citation_catalog) =
        create_transport_for_backend(&cfg.backend);

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .insert_resource(LivePromptWorker::spawn(addr, Duration::from_secs(5)))
        .insert_resource(DesktopClipboard::in_memory())
        .insert_resource(ClientClock {
            now_secs: fixture.now_secs,
            now_ms: fixture.now_secs * 1_000,
        })
        .insert_resource(transport)
        .insert_resource(citation_catalog)
        .insert_resource(DesktopWorkspaceState {
            connection_mode: conn_mode,
            patients,
            active_patient_index: 0,
            citations: Vec::new(),
            selected_citation_index: 0,
            transcript: Vec::new(),
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
            zoom_percent: 100,
            frames_rendered_ui: 0,
            prompts_submitted: 0,
        })
        .add_plugins(CurieClinicClientPlugin::new(
            TabId::new("w15-ui-stream-tab").expect("tab"),
            ClientBootConfig::from_location("/elysianfields/?lang=en").expect("boot"),
            SharedBrowserSessionHub::new(None),
            SharedLocaleStorage::default(),
        ))
        .add_plugins(ClinicDesktopUiPlugin::new(
            ThemeMode::Dark,
            ZoomLevel::Pct100,
            None,
        ));

    bootstrap_clinic_session(&mut app, boot_email, boot_token);

    // 1. Verify simulated in-flight streaming draft renders live and Escape cancels cleanly.
    {
        let mut ws = app.world_mut().resource_mut::<DesktopWorkspaceState>();
        ws.live_submission = LiveSubmissionState::InFlight {
            request_id: 99,
            workspace_id: fixture.workspace_id.clone(),
        };
        ws.streaming_draft = Some("Partial streamed glycemic note".to_owned());
        ws.status_banner = DesktopStatusBanner::Streaming(None);
    }
    app.update();
    {
        let mut q = app
            .world_mut()
            .query::<(&DesktopUiSlot, &Node, Option<&Text>)>();
        let mut cancel_visible = false;
        let mut transcript_text = String::new();
        for (slot, node, text) in q.iter(app.world()) {
            match *slot {
                DesktopUiSlot::CancelButton => cancel_visible = node.display == Display::Flex,
                DesktopUiSlot::TranscriptBody => {
                    if let Some(t) = text {
                        transcript_text = t.0.clone();
                    }
                }
                _ => {}
            }
        }
        assert!(
            cancel_visible,
            "Cancel button must be visible while InFlight"
        );
        assert!(
            transcript_text.contains("Partial streamed glycemic note"),
            "TranscriptBody must render progressive streaming_draft live: {transcript_text}"
        );
    }

    // Pressing Escape cancels the in-flight prompt, preserves partial draft, and hides Cancel button.
    app.world_mut().write_message(KeyboardInput {
        key_code: KeyCode::Escape,
        logical_key: Key::Escape,
        state: ButtonState::Pressed,
        text: None,
        repeat: false,
        window: Entity::PLACEHOLDER,
    });
    app.update();
    app.update();
    {
        let ws = app.world().resource::<DesktopWorkspaceState>();
        assert_eq!(ws.live_submission, LiveSubmissionState::Idle);
        assert!(ws.streaming_draft.is_none());
        assert_eq!(ws.status_message, translate(Locale::En, UiKey::Cancelled));
        assert!(
            ws.transcript
                .iter()
                .any(|e| e.body.contains("Partial streamed glycemic note")),
            "partial streaming draft must be preserved in transcript on cancel"
        );
    }

    // 2. Shut down server -> submit fails -> Retry button becomes visible and RetryPrompt re-attempts.
    drop(fixture);
    app.world_mut()
        .write_message(DesktopUiEvent::TypeText("Retryable prompt".to_owned()));
    app.world_mut().write_message(DesktopUiEvent::SubmitPrompt);
    app.update();

    let deadline = Instant::now() + Duration::from_secs(5);
    while matches!(
        app.world()
            .resource::<DesktopWorkspaceState>()
            .live_submission,
        LiveSubmissionState::InFlight { .. }
    ) && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(10));
        app.update();
    }
    app.update();

    {
        let ws = app.world().resource::<DesktopWorkspaceState>();
        assert!(matches!(
            ws.live_submission,
            LiveSubmissionState::Failed { .. }
        ));
        assert_eq!(ws.last_failed_prompt.as_deref(), Some("Retryable prompt"));
        let mut q = app.world_mut().query::<(&DesktopUiSlot, &Node)>();
        let retry_display = q
            .iter(app.world())
            .find(|(slot, _)| **slot == DesktopUiSlot::RetryButton)
            .map(|(_, node)| node.display);
        assert_eq!(
            retry_display,
            Some(Display::Flex),
            "Retry button must be shown when submission fails"
        );
    }

    let before_retry = app
        .world()
        .resource::<DesktopWorkspaceState>()
        .prompts_submitted;
    app.world_mut().write_message(DesktopUiEvent::RetryPrompt);
    app.update();
    assert_eq!(
        app.world()
            .resource::<DesktopWorkspaceState>()
            .prompts_submitted,
        before_retry + 1
    );
}

#[test]
fn live_worker_syncs_today_standing_memory_and_connected_care_invite_over_tcp() {
    let fixture = LiveServerFixture::spawn_with_workflows("workflows");
    let session_id = fixture.sign_in_session();
    let worker = LivePromptWorker::spawn(fixture.addr(), Duration::from_secs(5));

    // 1. Fetch initial live Today + Scheduled Visits + Longitudinal Memory snapshot over TCP.
    worker
        .fetch_workflow_snapshot(session_id.clone(), fixture.workspace_id.clone())
        .expect("queue workflow snapshot fetch");
    let initial = worker
        .recv_timeout(Duration::from_secs(5))
        .expect("timed out waiting for WorkflowSnapshotLoaded");
    let (urgent_item_id, urgent_version_id) = match initial {
        LivePromptOutcome::WorkflowSnapshotLoaded {
            workspace_id,
            today_items,
            scheduled_visits,
            memory_entries,
            ..
        } => {
            assert_eq!(workspace_id, fixture.workspace_id);
            assert!(
                today_items.len() >= 2,
                "expected >= 2 live seeded Today items, got {today_items:?}"
            );
            assert_eq!(
                today_items[0].item.urgency,
                TodayUrgency::Now,
                "Now item must sort ahead of Today item"
            );
            assert!(!today_items[0].seen);
            assert!(!today_items[0].done);
            assert!(
                !scheduled_visits.is_empty(),
                "expected live seeded scheduled visits"
            );
            assert!(
                memory_entries
                    .iter()
                    .any(|e| e.authority == MemoryAuthority::ChartRecord),
                "expected ChartRecord memory entry from live server"
            );
            assert!(
                memory_entries
                    .iter()
                    .any(|e| e.authority == MemoryAuthority::PhysicianInstruction),
                "expected PhysicianInstruction memory entry from live server"
            );
            (
                today_items[0].item.item_id.clone(),
                today_items[0].item.version_id,
            )
        }
        other => panic!("expected WorkflowSnapshotLoaded, got {other:?}"),
    };

    // 2. Mark urgent Today item as Seen and confirm live persistence on subsequent snapshot fetch.
    worker
        .update_today_standing(
            session_id.clone(),
            urgent_item_id.clone(),
            TodayStandingUpdate::Seen,
            urgent_version_id,
        )
        .expect("queue mark seen");
    match worker
        .recv_timeout(Duration::from_secs(5))
        .expect("timed out waiting for TodayStandingUpdated(Seen)")
    {
        LivePromptOutcome::TodayStandingUpdated {
            item_id,
            standing,
            version_id,
        } => {
            assert_eq!(item_id, urgent_item_id);
            assert_eq!(standing, TodayStandingUpdate::Seen);
            assert_eq!(version_id, urgent_version_id);
        }
        other => panic!("expected TodayStandingUpdated(Seen), got {other:?}"),
    }

    worker
        .fetch_workflow_snapshot(session_id.clone(), fixture.workspace_id.clone())
        .expect("queue snapshot fetch after Seen");
    match worker
        .recv_timeout(Duration::from_secs(5))
        .expect("timed out waiting for snapshot after Seen")
    {
        LivePromptOutcome::WorkflowSnapshotLoaded { today_items, .. } => {
            let updated = today_items
                .iter()
                .find(|r| r.item.item_id == urgent_item_id)
                .expect("seen item should remain in Today queue");
            assert!(updated.seen, "seen standing must persist on live server");
            assert!(!updated.done);
        }
        other => panic!("expected WorkflowSnapshotLoaded after Seen, got {other:?}"),
    }

    // 3. Mark urgent Today item as Done and confirm it leaves the active Today queue on the server.
    worker
        .update_today_standing(
            session_id.clone(),
            urgent_item_id.clone(),
            TodayStandingUpdate::Done,
            urgent_version_id,
        )
        .expect("queue mark done");
    match worker
        .recv_timeout(Duration::from_secs(5))
        .expect("timed out waiting for TodayStandingUpdated(Done)")
    {
        LivePromptOutcome::TodayStandingUpdated {
            item_id,
            standing,
            version_id,
        } => {
            assert_eq!(item_id, urgent_item_id);
            assert_eq!(standing, TodayStandingUpdate::Done);
            assert_eq!(version_id, urgent_version_id);
        }
        other => panic!("expected TodayStandingUpdated(Done), got {other:?}"),
    }

    worker
        .fetch_workflow_snapshot(session_id.clone(), fixture.workspace_id.clone())
        .expect("queue snapshot fetch after Done");
    match worker
        .recv_timeout(Duration::from_secs(5))
        .expect("timed out waiting for snapshot after Done")
    {
        LivePromptOutcome::WorkflowSnapshotLoaded { today_items, .. } => {
            assert!(
                !today_items.iter().any(|r| r.item.item_id == urgent_item_id),
                "done item must be persisted and omitted from active Today queue"
            );
        }
        other => panic!("expected WorkflowSnapshotLoaded after Done, got {other:?}"),
    }

    // 4. Issue a live Connected Care EF-XXXXXX invitation over HTTP 201 Created.
    worker
        .issue_connected_care_invite(
            session_id,
            fixture.workspace_id.clone(),
            "pat-stream-e2e".to_owned(),
            "1985-04-12".to_owned(),
        )
        .expect("queue connected care invite");
    match worker
        .recv_timeout(Duration::from_secs(5))
        .expect("timed out waiting for ConnectedCareInviteIssued")
    {
        LivePromptOutcome::ConnectedCareInviteIssued {
            workspace_id,
            invite_code,
            invitation_id,
        } => {
            assert_eq!(workspace_id, fixture.workspace_id);
            assert!(
                invite_code.starts_with("EF-") && invite_code.len() == 9,
                "expected 6-digit EF-XXXXXX invite code, got {invite_code}"
            );
            assert!(
                !invitation_id.trim().is_empty(),
                "expected non-empty invitation_id"
            );
        }
        other => panic!("expected ConnectedCareInviteIssued, got {other:?}"),
    }
}

#[test]
fn desktop_ui_live_mode_dispatches_today_standing_and_memory_invite_over_tcp() {
    let fixture = LiveServerFixture::spawn_with_workflows("ui-workflows");
    let addr = fixture.addr();
    let base_url = format!("http://{addr}");

    let cfg = DesktopLaunchConfig {
        backend: LaunchBackend::Server {
            base_url,
            socket_addr: addr,
            email: Some(fixture.email.clone()),
            token: None,
        },
        locale: Locale::En,
        theme: ThemeMode::Dark,
        zoom: ZoomLevel::Pct100,
        ..DesktopLaunchConfig::default()
    };
    let (transport, conn_mode, boot_email, boot_token, patients, citation_catalog) =
        create_transport_for_backend(&cfg.backend);

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .insert_resource(LivePromptWorker::spawn(addr, Duration::from_secs(5)))
        .insert_resource(DesktopClipboard::in_memory())
        .insert_resource(ClientClock {
            now_secs: fixture.now_secs,
            now_ms: fixture.now_secs * 1_000,
        })
        .insert_resource(transport)
        .insert_resource(citation_catalog)
        .insert_resource(DesktopWorkspaceState {
            connection_mode: conn_mode,
            patients,
            active_patient_index: 0,
            citations: Vec::new(),
            selected_citation_index: 0,
            transcript: Vec::new(),
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
            zoom_percent: 100,
            frames_rendered_ui: 0,
            prompts_submitted: 0,
        })
        .add_plugins(CurieClinicClientPlugin::new(
            TabId::new("w19-ui-workflows-tab").expect("tab"),
            ClientBootConfig::from_location("/elysianfields/?lang=en").expect("boot"),
            SharedBrowserSessionHub::new(None),
            SharedLocaleStorage::default(),
        ))
        .add_plugins(ClinicDesktopUiPlugin::new(
            ThemeMode::Dark,
            ZoomLevel::Pct100,
            None,
        ));

    bootstrap_clinic_session(&mut app, boot_email, boot_token);
    app.update();
    app.update();

    let active_ws = app
        .world()
        .resource::<DesktopWorkspaceState>()
        .active_patient()
        .expect("active patient")
        .workspace_id
        .clone();
    assert_eq!(active_ws, fixture.workspace_id);

    // 1. Switching to Today tab fetches the live server snapshot into ClinicWorkflowState.
    app.world_mut()
        .write_message(DesktopUiEvent::SelectTab(ClinicWorkspaceTab::Today));
    app.update();

    let deadline = Instant::now() + Duration::from_secs(15);
    while app
        .world()
        .resource::<ClinicWorkflowState>()
        .live_today
        .is_none()
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(10));
        app.update();
    }
    app.update();

    let (initial_count, first_item_id) = {
        let wf = app.world().resource::<ClinicWorkflowState>();
        let items = wf.visible_today_items();
        assert!(
            items.len() >= 2,
            "expected live Today items after tab switch, got {items:?}"
        );
        assert!(
            wf.memory_entries_for(&active_ws)
                .iter()
                .any(|e| e.authority == MemoryAuthority::ChartRecord)
        );
        assert!(
            wf.memory_entries_for(&active_ws)
                .iter()
                .any(|e| e.authority == MemoryAuthority::PhysicianInstruction)
        );
        (items.len(), items[0].item.item_id.clone())
    };

    // 2. Clicking MarkSeen dispatches UpdateTodayStanding(Seen) to the live server.
    app.world_mut()
        .write_message(DesktopUiEvent::TodayMarkSeen(0));
    app.update();

    let deadline = Instant::now() + Duration::from_secs(15);
    while !app
        .world()
        .resource::<ClinicWorkflowState>()
        .visible_today_items()
        .first()
        .is_some_and(|r| r.seen)
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(10));
        app.update();
    }
    app.update();
    assert!(
        app.world()
            .resource::<DesktopWorkspaceState>()
            .status_message
            .contains(&first_item_id)
    );

    // 3. Clicking MarkDone dispatches UpdateTodayStanding(Done) and removes the item from live Today.
    app.world_mut()
        .write_message(DesktopUiEvent::TodayMarkDone(0));
    app.update();

    let deadline = Instant::now() + Duration::from_secs(15);
    while app
        .world()
        .resource::<ClinicWorkflowState>()
        .visible_today_items()
        .len()
        >= initial_count
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(10));
        app.update();
    }
    app.update();
    assert_eq!(
        app.world()
            .resource::<ClinicWorkflowState>()
            .visible_today_items()
            .len(),
        initial_count - 1
    );

    // 4. Switching to Memory & Care and clicking IssueInvite fetches a live EF-XXXXXX invite code.
    app.world_mut()
        .write_message(DesktopUiEvent::SelectTab(ClinicWorkspaceTab::MemoryCare));
    app.world_mut()
        .write_message(DesktopUiEvent::MemoryIssueInvite);
    app.update();

    let deadline = Instant::now() + Duration::from_secs(15);
    while app
        .world()
        .resource::<ClinicWorkflowState>()
        .active_invite_code_for(&active_ws)
        .is_none()
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(10));
        app.update();
    }
    app.update();

    let code = app
        .world()
        .resource::<ClinicWorkflowState>()
        .active_invite_code_for(&active_ws)
        .expect("live EF-XXXXXX invite code")
        .to_owned();
    assert!(
        code.starts_with("EF-") && code.len() == 9,
        "expected EF-XXXXXX code, got {code}"
    );
    assert_eq!(
        app.world()
            .resource::<ClinicWorkflowState>()
            .connected_care_state_for(&active_ws),
        ConnectedCareState::Invited
    );
    assert!(
        app.world()
            .resource::<DesktopWorkspaceState>()
            .status_message
            .contains(&code),
        "status bar must display live invite code {code}"
    );

    let mut q = app.world_mut().query::<(&DesktopUiSlot, &Text)>();
    let invite_slot_text = q
        .iter(app.world())
        .find(|(slot, _)| **slot == DesktopUiSlot::MemoryActiveInviteCodeText)
        .map(|(_, text)| text.0.clone())
        .expect("MemoryActiveInviteCodeText slot");
    assert!(
        invite_slot_text.contains(&code),
        "MemoryActiveInviteCodeText must render live EF-XXXXXX code: {invite_slot_text}"
    );
}

#[test]
fn live_clinic_messages_sync_draft_copy_restore_close_undo_and_wal_restart_over_tcp() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let wal_dir = std::env::temp_dir().join(format!(
        "ef-clinic-live-messages-e2e-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&wal_dir).expect("create temp wal dir");

    let now_secs = 1_760_500_000u64;
    let src_key = SourceKey::new("src-msg-e2e-1").expect("src key");
    let practice = PracticeId::new("prac-msg-e2e").expect("practice");
    let doc = PhysicianId::new("phys-msg-e2e").expect("doc");
    let email = "messages.e2e@example.invalid".to_owned();
    let workspace_id = WorkspaceId::new("ws-msg-e2e").expect("ws");

    let (server, addr) = {
        let mut service = ClinicService::open_durable(
            &wal_dir,
            SignInMode::LocalProof,
            RetryBackoffMode::DeterministicTest,
            src_key.clone(),
            now_secs,
        )
        .expect("open durable service");
        service
            .seed_account(PhysicianAccount {
                physician_id: doc.clone(),
                practice_id: practice.clone(),
                email: EmailAddress::parse(&email).expect("email"),
                display_name: "Dr. Messages E2E".to_owned(),
                role: PhysicianRole::Standard,
                standing: AccountStanding::Active,
                sessions_ended_before: None,
            })
            .expect("seed account");
        service
            .seed_workspace(
                workspace_id.clone(),
                practice,
                "Avery, Jordan (Messages E2E)",
                vec![doc],
            )
            .expect("seed workspace");
        service
            .seed_demo_workflows(&workspace_id, "Avery, Jordan (Messages E2E)")
            .expect("seed demo workflows");
        let server = TcpClinicServer::bind("127.0.0.1:0", service).expect("bind server 1");
        let addr = server.local_addr();
        (server, addr)
    };

    let base_url = format!("http://{addr}");
    let cfg = DesktopLaunchConfig {
        backend: LaunchBackend::Server {
            base_url,
            socket_addr: addr,
            email: Some(email.clone()),
            token: None,
        },
        locale: Locale::En,
        theme: ThemeMode::Dark,
        zoom: ZoomLevel::Pct100,
        ..DesktopLaunchConfig::default()
    };
    let (transport, conn_mode, boot_email, boot_token, patients, citation_catalog) =
        create_transport_for_backend(&cfg.backend);

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .insert_resource(LivePromptWorker::spawn(addr, Duration::from_secs(10)))
        .insert_resource(DesktopClipboard::in_memory())
        .insert_resource(ClientClock {
            now_secs,
            now_ms: now_secs * 1_000,
        })
        .insert_resource(transport)
        .insert_resource(citation_catalog)
        .insert_resource(DesktopWorkspaceState {
            connection_mode: conn_mode,
            patients,
            active_patient_index: 0,
            citations: Vec::new(),
            selected_citation_index: 0,
            transcript: Vec::new(),
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
            zoom_percent: 100,
            frames_rendered_ui: 0,
            prompts_submitted: 0,
        })
        .add_plugins(CurieClinicClientPlugin::new(
            TabId::new("w21-ui-messages-tab").expect("tab"),
            ClientBootConfig::from_location("/elysianfields/?lang=en").expect("boot"),
            SharedBrowserSessionHub::new(None),
            SharedLocaleStorage::default(),
        ))
        .add_plugins(ClinicDesktopUiPlugin::new(
            ThemeMode::Dark,
            ZoomLevel::Pct100,
            None,
        ));

    bootstrap_clinic_session(&mut app, boot_email, boot_token);
    app.update();
    app.update();

    // 1. Switch to Messages tab -> triggers live fetch of workspace replies over HTTP.
    app.world_mut()
        .write_message(DesktopUiEvent::SelectTab(ClinicWorkspaceTab::Messages));
    app.update();

    let deadline = Instant::now() + Duration::from_secs(15);
    while app
        .world()
        .resource::<ClinicWorkflowState>()
        .active_live_reply_card(Some(&workspace_id))
        .is_none()
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(10));
        app.update();
    }
    app.update();

    let initial_card = app
        .world()
        .resource::<ClinicWorkflowState>()
        .active_live_reply_card(Some(&workspace_id))
        .expect("live reply card fetched")
        .clone();
    assert_eq!(initial_card.conversation_id, "conv-ws-msg-e2e-msg-1");
    assert_eq!(
        initial_card.visible_reply_origin,
        LiveReplyOrigin::SystemReply
    );
    let system_text = initial_card
        .visible_reply_text
        .clone()
        .expect("system reply text");
    assert!(initial_card.flagged_concern.is_some());

    // 2. Edit personal draft (`YourDraft`) and save over HTTP `POST .../draft`.
    let custom_draft =
        "Please stay on Metformin 1000 mg twice daily and message me your 3-day log before Friday.";
    app.world_mut()
        .write_message(DesktopUiEvent::UpdateReplyDraft(custom_draft.to_owned()));
    app.world_mut()
        .write_message(DesktopUiEvent::MessagesSaveDraft);
    app.update();

    let deadline = Instant::now() + Duration::from_secs(15);
    while app
        .world()
        .resource::<ClinicWorkflowState>()
        .active_live_reply_card(Some(&workspace_id))
        .map(|c| c.visible_reply_origin)
        != Some(LiveReplyOrigin::PersonalDraft)
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(10));
        app.update();
    }
    app.update();

    assert_eq!(
        app.world()
            .resource::<ClinicWorkflowState>()
            .active_live_reply_card(Some(&workspace_id))
            .and_then(|c| c.visible_reply_text.as_deref()),
        Some(custom_draft)
    );

    // 3. Copy Reply (`CopiesWhatYouSee`) returns exact personal draft text via `POST .../copy`.
    app.world_mut()
        .write_message(DesktopUiEvent::MessagesCopyReply);
    app.update();

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut copied_match = false;
    while Instant::now() < deadline {
        if let Ok(clip) = app
            .world_mut()
            .resource_mut::<DesktopClipboard>()
            .paste_text()
            && clip == custom_draft
        {
            copied_match = true;
            break;
        }
        thread::sleep(Duration::from_millis(10));
        app.update();
    }
    assert!(
        copied_match,
        "CopiesWhatYouSee must copy exact active PersonalDraft text into DesktopClipboard"
    );

    // 4. Restore AI System Draft (`POST .../restore`) switches back to `SystemReply`.
    app.world_mut()
        .write_message(DesktopUiEvent::MessagesRestoreSystem);
    app.update();

    let deadline = Instant::now() + Duration::from_secs(15);
    while app
        .world()
        .resource::<ClinicWorkflowState>()
        .active_live_reply_card(Some(&workspace_id))
        .map(|c| c.visible_reply_origin)
        != Some(LiveReplyOrigin::SystemReply)
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(10));
        app.update();
    }
    app.update();
    assert_eq!(
        app.world()
            .resource::<ClinicWorkflowState>()
            .active_live_reply_card(Some(&workspace_id))
            .and_then(|c| c.visible_reply_text.as_deref()),
        Some(system_text.as_str())
    );

    // 5. Close Reply (`POST .../close`) clears visible text while keeping flagged concern + enabling Undo.
    app.world_mut()
        .write_message(DesktopUiEvent::MessagesCloseReply);
    app.update();

    let deadline = Instant::now() + Duration::from_secs(15);
    while !app
        .world()
        .resource::<ClinicWorkflowState>()
        .active_live_reply_card(Some(&workspace_id))
        .is_some_and(|c| c.undo_available && c.visible_reply_text.is_none())
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(10));
        app.update();
    }
    app.update();
    assert!(
        app.world()
            .resource::<ClinicWorkflowState>()
            .active_live_reply_card(Some(&workspace_id))
            .and_then(|c| c.flagged_concern.as_deref())
            .is_some(),
        "ConcernStays: flagged concern must remain visible after close"
    );

    // 6. Undo Close (`POST .../undo`) restores the exact system reply (`UndoBringsBack`).
    app.world_mut()
        .write_message(DesktopUiEvent::MessagesUndoClose);
    app.update();

    let deadline = Instant::now() + Duration::from_secs(15);
    while !app
        .world()
        .resource::<ClinicWorkflowState>()
        .active_live_reply_card(Some(&workspace_id))
        .is_some_and(|c| {
            !c.undo_available && c.visible_reply_text.as_deref() == Some(system_text.as_str())
        })
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(10));
        app.update();
    }
    app.update();

    // 7. Save a final personal draft, shut down server 1, restart server 2 from the SAME WAL directory,
    // and verify WAL persistence across server restart via LivePromptWorker!
    let persisted_draft =
        "Persisted across WAL restart: hold Metformin 1000 mg BID and review readings Friday.";
    app.world_mut()
        .write_message(DesktopUiEvent::UpdateReplyDraft(persisted_draft.to_owned()));
    app.world_mut()
        .write_message(DesktopUiEvent::MessagesSaveDraft);
    app.update();

    let deadline = Instant::now() + Duration::from_secs(15);
    while app
        .world()
        .resource::<ClinicWorkflowState>()
        .active_live_reply_card(Some(&workspace_id))
        .and_then(|c| c.visible_reply_text.as_deref())
        != Some(persisted_draft)
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(10));
        app.update();
    }

    server.shutdown();

    let replayed_service = ClinicService::open_durable(
        &wal_dir,
        SignInMode::LocalProof,
        RetryBackoffMode::DeterministicTest,
        src_key,
        now_secs + 60,
    )
    .expect("reopen durable service after restart");
    let server2 = TcpClinicServer::bind("127.0.0.1:0", replayed_service).expect("bind server 2");
    let addr2 = server2.local_addr();

    let mut transport2 = LiveHttpClinicTransport::new(addr2);
    let init2 = transport2
        .initiate_sign_in(
            &SignInRequest {
                email: email.clone(),
            },
            now_secs + 60,
        )
        .expect("init sign in after restart");
    let token2 = match init2 {
        ef_clinic_domain::auth::SignInInitiationResponse::TokenIssued { token, .. } => token,
        other => panic!("expected TokenIssued, got {other:?}"),
    };
    let session2 = transport2
        .verify_sign_in(
            &VerifySignInRequest {
                email,
                token: token2,
            },
            now_secs + 60,
        )
        .expect("verify sign in after restart")
        .session;

    let worker2 = LivePromptWorker::spawn(addr2, Duration::from_secs(10));
    worker2
        .fetch_workflow_snapshot(session2, workspace_id.clone())
        .expect("fetch snapshot after restart");
    match worker2
        .recv_timeout(Duration::from_secs(15))
        .expect("timed out waiting for snapshot after restart")
    {
        LivePromptOutcome::WorkflowSnapshotLoaded { reply_cards, .. } => {
            assert_eq!(reply_cards.len(), 1);
            assert_eq!(
                reply_cards[0].visible_reply_origin,
                LiveReplyOrigin::PersonalDraft
            );
            assert_eq!(
                reply_cards[0].visible_reply_text.as_deref(),
                Some(persisted_draft)
            );
        }
        other => panic!("expected WorkflowSnapshotLoaded after restart, got {other:?}"),
    }

    server2.shutdown();
    let _ = fs::remove_dir_all(&wal_dir);
}

fn read_http_request(stream: &TcpStream) -> Option<(Vec<u8>, String, String)> {
    let cloned = stream.try_clone().ok()?;
    let mut reader = BufReader::new(cloned);
    let mut head = Vec::new();
    let mut req_line = String::new();
    if reader.read_line(&mut req_line).ok()? == 0 {
        return None;
    }
    let target = req_line.split_whitespace().nth(1)?.to_owned();
    head.extend_from_slice(req_line.as_bytes());

    let mut content_len = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            break;
        }
        head.extend_from_slice(line.as_bytes());
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((k, v)) = line.split_once(':')
            && k.trim().eq_ignore_ascii_case("content-length")
        {
            content_len = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; content_len];
    if content_len > 0 {
        reader.read_exact(&mut body).ok()?;
    }
    let body_text = String::from_utf8_lossy(&body).into_owned();
    head.extend_from_slice(&body);
    Some((head, target, body_text))
}

fn forward_http_raw(addr: SocketAddr, raw_request: &[u8]) -> Vec<u8> {
    let mut upstream = TcpStream::connect(addr).expect("connect upstream clinic server");
    upstream.write_all(raw_request).expect("write upstream");
    upstream.flush().expect("flush upstream");
    let _ = upstream.shutdown(Shutdown::Write);
    let mut resp = Vec::new();
    upstream.read_to_end(&mut resp).expect("read upstream");
    resp
}

fn extract_first_delta_sse_response(raw_resp: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(raw_resp);
    let body = text.split_once("\r\n\r\n").map_or("", |(_, b)| b);
    let first_delta = body
        .split("\n\n")
        .find(|frame| frame.contains("event: delta_chunk"))
        .map(|frame| format!("{frame}\n\n"))
        .unwrap_or_else(|| ": keep-alive\n\n".to_owned());
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{first_delta}",
        first_delta.len()
    )
    .into_bytes()
}
