//! Headless Curie Clinic HTTP/SSE service and router mounted strictly under `/elysianfields/`.
//!
//! Cross-compiles cleanly on both native hosts and `wasm32-unknown-unknown`.

use std::collections::{BTreeMap, VecDeque};
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

use ef_clinic_domain::auth::{
    AuthAuthority, AuthError, EmailAddress, PhysicianAccount, SignInMode, VerificationTokenKind,
    WhoAmIPayload,
};
use ef_clinic_domain::chat::{
    AnswerState, ChatCommandError, ChatEngine, ChatStreamEvent, CommandAcceptance, CommandEnvelope,
    CommandPayload, DraftSaveOutcome, DraftStore, ExchangeRecord, MAX_FAILURE_ATTEMPTS,
    PersistedDraft, RetryBackoffMode,
};
use ef_clinic_domain::harness_adapter::{ReadRecordsTool, SearchChartTool};
use ef_clinic_domain::workspace::{
    CitedSourcePayload, MembershipTier, PatientWorkspaceSummary, WorkspaceAccessError,
    WorkspaceEvidenceStore,
};
use ef_clinic_workflows::matching::{MessageMatchingEngine, RawChannelFrame};
use ef_clinic_workflows::memory::PatientMemoryStore;
use ef_clinic_workflows::replies::{MessageReplyEngine, ReviewCompletion};
use ef_clinic_workflows::today::{
    ChannelHealthStatus, ScheduledVisitEntry, TodayQueueEngine, TodayQueueItem,
};
use ef_connected_care::{ConnectedCareBridge, InvitationId, PairKey};
use ef_core::{
    ChatId, PhysicianId, PracticeId, SessionId, SourceKey, Subpath, WorkItemId, WorkspaceId,
};
use ef_harness::{AgentLoop, AgentTool, CancellationToken, StubProvider, TurnExecutionOutcome};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::http::{
    AUTH_MAX_BODY_BYTES, COMMANDS_MAX_BODY_BYTES, HttpMethod, HttpRequest, HttpResponse,
    format_sse_event, query_param, stream_event_id,
};
#[cfg(not(target_arch = "wasm32"))]
use crate::persistence::{DurableClinicStore, PgClinicStore};
use crate::persistence::{InMemoryWal, WalEntry, WalError, WalSink, WalTokenKind};
use crate::rate_limit::{IpSlidingWindowLimiter, RateLimitConfig, RateLimitDecision};
#[cfg(not(target_arch = "wasm32"))]
use crate::static_files::serve_static_asset;

/// Minimal static entry document served at `GET /elysianfields/` under strict CSP.
const ENTRY_HTML: &str = "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Curie Clinic</title></head><body><div id=\"app\" data-mount=\"/elysianfields/\">Curie Clinic</div></body></html>";

/// Service initialization or persistence error.
#[derive(Debug, Error)]
pub enum ClinicServiceError {
    #[error("durable WAL persistence error: {0}")]
    Wal(#[from] WalError),
}

#[derive(Debug, Deserialize)]
struct RequestLinkBody {
    email: String,
    #[serde(default)]
    kind: Option<WalTokenKind>,
}

#[derive(Debug, Deserialize)]
struct VerifyTokenBody {
    email: String,
    token: String,
    #[serde(default)]
    fixed_ends_at: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct SignOutBody {
    #[serde(default)]
    session: Option<SessionId>,
}

#[derive(Debug, Deserialize)]
struct PutDraftBody {
    text: String,
    expected_revision: u64,
}

#[derive(Debug, Serialize)]
struct WorkspaceDetailResponse {
    workspace_id: WorkspaceId,
    membership: MembershipTier,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<PatientWorkspaceSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chats: Option<BTreeMap<String, Vec<ExchangeRecord>>>,
}

#[derive(Debug, Serialize)]
struct ChatSnapshotResponse {
    workspace_id: WorkspaceId,
    chat_id: ChatId,
    exchanges: Vec<ExchangeRecord>,
    draft: PersistedDraft,
}

#[derive(Debug, Serialize)]
struct ClientWorkspaceDetailWire {
    patient_workspace_id: WorkspaceId,
    display_name: String,
    membership: MembershipTier,
    time_zone: String,
    default_chat_id: ChatId,
    patient_synopsis: Option<String>,
}

#[derive(Debug, Serialize)]
struct ClientChatSnapshotWire {
    workspace_id: WorkspaceId,
    chat_id: ChatId,
    transcript: Vec<ExchangeRecord>,
    sources: BTreeMap<SourceKey, CitedSourcePayload>,
    queued: Vec<String>,
    archived: bool,
}

/// Authoritative headless Curie Clinic vertical-slice service.
pub struct ClinicService {
    auth: AuthAuthority,
    pub(crate) evidence: WorkspaceEvidenceStore,
    chats: BTreeMap<(WorkspaceId, ChatId), ChatEngine>,
    drafts: DraftStore,
    retry_mode: RetryBackoffMode,
    agent_loop: AgentLoop,
    default_cited_source: SourceKey,
    queued_providers: VecDeque<StubProvider>,
    pub(crate) clock_secs: u64,
    pub(crate) wal: Box<dyn WalSink>,
    rate_limiter: IpSlidingWindowLimiter,
    #[cfg(not(target_arch = "wasm32"))]
    web_dir: Option<PathBuf>,
    pub(crate) today_engine: TodayQueueEngine,
    pub(crate) today_candidates: Vec<TodayQueueItem>,
    pub(crate) scheduled_visits: Vec<ScheduledVisitEntry>,
    pub(crate) today_also_reviewed: usize,
    pub(crate) matching_engine: MessageMatchingEngine,
    pub(crate) reply_engine: MessageReplyEngine,
    pub(crate) patient_memories: BTreeMap<WorkspaceId, PatientMemoryStore>,
    pub(crate) connected_care: ConnectedCareBridge,
    pub(crate) invitation_pairs: BTreeMap<InvitationId, PairKey>,
    pub(crate) intelligence: crate::intelligence::ClinicIntelligenceState,
}

impl ClinicService {
    /// Creates an in-memory clinic service backed by [`InMemoryWal`].
    #[must_use]
    pub fn new_in_memory(
        sign_in_mode: SignInMode,
        retry_mode: RetryBackoffMode,
        default_cited_source: SourceKey,
        initial_clock_secs: u64,
    ) -> Self {
        Self {
            auth: AuthAuthority::new(sign_in_mode),
            evidence: WorkspaceEvidenceStore::new(),
            chats: BTreeMap::new(),
            drafts: DraftStore::new(),
            retry_mode,
            agent_loop: AgentLoop::new(),
            default_cited_source,
            queued_providers: VecDeque::new(),
            clock_secs: initial_clock_secs,
            wal: Box::new(InMemoryWal::new()),
            rate_limiter: IpSlidingWindowLimiter::new(RateLimitConfig::default()),
            #[cfg(not(target_arch = "wasm32"))]
            web_dir: None,
            today_engine: TodayQueueEngine::new(),
            today_candidates: Vec::new(),
            scheduled_visits: Vec::new(),
            today_also_reviewed: 0,
            matching_engine: MessageMatchingEngine::new(),
            reply_engine: MessageReplyEngine::new(),
            patient_memories: BTreeMap::new(),
            connected_care: ConnectedCareBridge::new(),
            invitation_pairs: BTreeMap::new(),
            intelligence: crate::intelligence::ClinicIntelligenceState::default(),
        }
    }

    /// Opens a durable WAL directory, replays all persisted mutations through the
    /// live domain owners, and returns a ready-to-serve [`ClinicService`].
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_durable(
        data_dir: &Path,
        sign_in_mode: SignInMode,
        retry_mode: RetryBackoffMode,
        default_cited_source: SourceKey,
        initial_clock_secs: u64,
    ) -> Result<Self, ClinicServiceError> {
        let (store, entries) = DurableClinicStore::open(data_dir)?;
        let mut service = Self {
            auth: AuthAuthority::new(sign_in_mode),
            evidence: WorkspaceEvidenceStore::new(),
            chats: BTreeMap::new(),
            drafts: DraftStore::new(),
            retry_mode,
            agent_loop: AgentLoop::new(),
            default_cited_source,
            queued_providers: VecDeque::new(),
            clock_secs: initial_clock_secs,
            wal: Box::new(store),
            rate_limiter: IpSlidingWindowLimiter::new(RateLimitConfig::default()),
            web_dir: None,
            today_engine: TodayQueueEngine::new(),
            today_candidates: Vec::new(),
            scheduled_visits: Vec::new(),
            today_also_reviewed: 0,
            matching_engine: MessageMatchingEngine::new(),
            reply_engine: MessageReplyEngine::new(),
            patient_memories: BTreeMap::new(),
            connected_care: ConnectedCareBridge::new(),
            invitation_pairs: BTreeMap::new(),
            intelligence: crate::intelligence::ClinicIntelligenceState::default(),
        };
        for entry in entries {
            service.replay_wal_entry(entry);
        }
        Ok(service)
    }

    /// Opens a PostgreSQL-backed durable [`ClinicService`] via `ef-pgwire`, applying
    /// `0001_m2_clinic_slice.sql` idempotently and replaying all persisted mutations.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_postgres(
        config: &ef_pgwire::PgConnectionConfig,
        sign_in_mode: SignInMode,
        retry_mode: RetryBackoffMode,
        default_cited_source: SourceKey,
        initial_clock_secs: u64,
    ) -> Result<Self, ClinicServiceError> {
        let (store, entries) = PgClinicStore::open(config)?;
        let mut service = Self {
            auth: AuthAuthority::new(sign_in_mode),
            evidence: WorkspaceEvidenceStore::new(),
            chats: BTreeMap::new(),
            drafts: DraftStore::new(),
            retry_mode,
            agent_loop: AgentLoop::new(),
            default_cited_source,
            queued_providers: VecDeque::new(),
            clock_secs: initial_clock_secs,
            wal: Box::new(store),
            rate_limiter: IpSlidingWindowLimiter::new(RateLimitConfig::default()),
            web_dir: None,
            today_engine: TodayQueueEngine::new(),
            today_candidates: Vec::new(),
            scheduled_visits: Vec::new(),
            today_also_reviewed: 0,
            matching_engine: MessageMatchingEngine::new(),
            reply_engine: MessageReplyEngine::new(),
            patient_memories: BTreeMap::new(),
            connected_care: ConnectedCareBridge::new(),
            invitation_pairs: BTreeMap::new(),
        };
        for entry in entries {
            service.replay_wal_entry(entry);
        }
        Ok(service)
    }

    /// Configures the per-IP sliding-window ingress rate limiter.
    pub fn set_rate_limit(&mut self, config: RateLimitConfig) {
        self.rate_limiter.reconfigure(config);
    }

    /// Configures an optional directory serving static web assets under `/elysianfields/`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_web_dir(&mut self, dir: PathBuf) {
        self.web_dir = Some(dir);
    }

    /// Durably seeds a Today queue item.
    pub fn seed_today_item(&mut self, item: TodayQueueItem) -> Result<(), ClinicServiceError> {
        self.today_candidates.push(item.clone());
        self.wal.append(&WalEntry::AddTodayItem { item })?;
        Ok(())
    }

    /// Durably seeds a scheduled visit entry for Today.
    pub fn seed_scheduled_visit(
        &mut self,
        visit: ScheduledVisitEntry,
    ) -> Result<(), ClinicServiceError> {
        self.scheduled_visits.push(visit.clone());
        self.wal.append(&WalEntry::AddScheduledVisit { visit })?;
        Ok(())
    }

    /// Durably updates a practice communication channel health status (`TodaySaysSo`).
    pub fn set_channel_health(
        &mut self,
        channel_id: impl Into<String>,
        status: ChannelHealthStatus,
    ) -> Result<(), ClinicServiceError> {
        let channel_id = channel_id.into();
        self.today_engine
            .set_channel_health(channel_id.clone(), status.clone());
        self.matching_engine
            .register_channel(channel_id.clone(), status.clone());
        self.wal
            .append(&WalEntry::SetChannelHealth { channel_id, status })?;
        Ok(())
    }

    /// Advances or sets the deterministic service clock in Unix epoch seconds.
    pub fn set_clock_secs(&mut self, now_secs: u64) {
        self.clock_secs = now_secs;
    }

    /// Enqueues a scripted [`StubProvider`] to be consumed by the next executed turn.
    pub fn enqueue_provider_script(&mut self, provider: StubProvider) {
        self.queued_providers.push_back(provider);
    }

    /// Durably registers or updates an invited physician account.
    pub fn seed_account(&mut self, account: PhysicianAccount) -> Result<(), ClinicServiceError> {
        self.auth.upsert_account(account.clone());
        self.wal.append(&WalEntry::UpsertAccount { account })?;
        Ok(())
    }

    /// Durably deactivates an existing physician account.
    pub fn deactivate_account(&mut self, email: &EmailAddress) -> Result<(), ClinicServiceError> {
        self.auth.deactivate_account(email);
        self.wal.append(&WalEntry::DeactivateAccount {
            email: email.clone(),
        })?;
        Ok(())
    }

    /// Durably registers a practice-scoped patient workspace and its panel physicians.
    pub fn seed_workspace(
        &mut self,
        workspace_id: WorkspaceId,
        practice_id: PracticeId,
        display_name: impl Into<String>,
        panel_physicians: Vec<PhysicianId>,
    ) -> Result<(), ClinicServiceError> {
        let display_name = display_name.into();
        self.evidence.register_workspace(
            workspace_id.clone(),
            practice_id.clone(),
            display_name.clone(),
            panel_physicians.clone(),
        );
        self.wal.append(&WalEntry::RegisterWorkspace {
            workspace_id,
            practice_id,
            display_name,
            panel_physicians,
        })?;
        Ok(())
    }

    /// Durably inserts a cited clinical source record scoped to `payload.workspace_id`.
    pub fn seed_source(&mut self, payload: CitedSourcePayload) -> Result<(), ClinicServiceError> {
        self.intelligence.index_source(&payload);
        self.evidence.insert_source(payload.clone());
        self.wal.append(&WalEntry::InsertSource { payload })?;
        Ok(())
    }

    /// Durably withdraws a cited clinical source so future fetches redact clinical text.
    pub fn withdraw_source(
        &mut self,
        workspace_id: &WorkspaceId,
        source_key: &SourceKey,
    ) -> Result<bool, ClinicServiceError> {
        let changed = self.evidence.withdraw_source(workspace_id, source_key);
        if changed {
            self.intelligence.mark_withdrawn(workspace_id, source_key);
            self.wal.append(&WalEntry::WithdrawSource {
                workspace_id: workspace_id.clone(),
                source_key: source_key.clone(),
            })?;
        }
        Ok(changed)
    }

    /// Exposes read-only access to the auth authority for verification assertions.
    #[must_use]
    pub const fn auth(&self) -> &AuthAuthority {
        &self.auth
    }

    /// Routes and executes a single HTTP/1.1 request strictly under `/elysianfields/`.
    pub fn handle_request(&mut self, req: &HttpRequest) -> HttpResponse {
        let client_ip = req
            .header("x-forwarded-for")
            .and_then(|raw| raw.split(',').next())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("127.0.0.1")
            .to_owned();
        self.handle_request_from_ip(req, &client_ip)
    }

    /// Replays any WAL entries committed by peer server instances sharing the same durable directory.
    pub fn sync_from_wal(&mut self) -> Result<usize, ClinicServiceError> {
        let new_entries = self.wal.sync_new_entries()?;
        let count = new_entries.len();
        for entry in new_entries {
            self.replay_wal_entry(entry);
        }
        Ok(count)
    }

    /// Routes and executes a single HTTP/1.1 request attributed to `client_ip` with
    /// per-IP sliding-window ingress rate limiting and cross-instance WAL synchronization.
    pub fn handle_request_from_ip(&mut self, req: &HttpRequest, client_ip: &str) -> HttpResponse {
        let _lock = match self.wal.acquire_cross_instance_lock() {
            Ok(guard) => guard,
            Err(err) => return wal_failure_response(&err),
        };
        if let Err(ClinicServiceError::Wal(err)) = self.sync_from_wal() {
            return wal_failure_response(&err);
        }
        self.handle_request_from_ip_locked(req, client_ip)
    }

    fn handle_request_from_ip_locked(
        &mut self,
        req: &HttpRequest,
        client_ip: &str,
    ) -> HttpResponse {
        // 0. Enforce per-IP sliding-window ingress rate limit.
        if let RateLimitDecision::Exceeded { retry_after_secs } = self
            .rate_limiter
            .check_and_record(client_ip, self.clock_secs)
        {
            return HttpResponse::too_many_requests(retry_after_secs);
        }

        // 1. Canonical trailing slash redirect for bare `/elysianfields`.
        if matches!(req.method, HttpMethod::Get | HttpMethod::Head)
            && let Some(location) = Subpath::canonical_redirect_if_bare(&req.target)
        {
            return HttpResponse::permanent_redirect(location);
        }

        // 2. Enforce segment-bounded `/elysianfields` subpath mount.
        let (pathname, query) = req.split_target();
        if !Subpath::is_within_mount(pathname) {
            return HttpResponse::not_found();
        }
        if Subpath::parse_mounted(&req.target).is_err() {
            return HttpResponse::not_found();
        }

        let Some(subpath) = pathname.strip_prefix(Subpath::CANONICAL_PREFIX) else {
            return HttpResponse::not_found();
        };

        // 3. Entry HTML and optional static assets under `/elysianfields/` (non-API paths).
        if !subpath.starts_with("/api/") {
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(web_dir) = self.web_dir.clone()
                && let Some(resp) = serve_static_asset(&web_dir, req.method, subpath)
            {
                return resp;
            }

            if subpath == "/" || subpath == "/index.html" {
                return match req.method {
                    HttpMethod::Get | HttpMethod::Head => HttpResponse::html_entry(ENTRY_HTML),
                    HttpMethod::Post | HttpMethod::Put => HttpResponse::method_not_allowed(),
                };
            }
            return HttpResponse::not_found();
        }

        // 4. API routes under `/elysianfields/api/...`.
        let Some(api_path) = subpath.strip_prefix("/api/") else {
            return HttpResponse::not_found();
        };

        let segments: Vec<&str> = api_path.split('/').filter(|s| !s.is_empty()).collect();

        // Auth routes carry a strict 64 KiB body cap.
        if segments.first().copied() == Some("auth") {
            if req.body.len() > AUTH_MAX_BODY_BYTES {
                return HttpResponse::payload_too_large(AUTH_MAX_BODY_BYTES);
            }
            return self.route_auth(req, &segments);
        }

        if req.body.len() > COMMANDS_MAX_BODY_BYTES {
            return HttpResponse::payload_too_large(COMMANDS_MAX_BODY_BYTES);
        }

        match segments.as_slice() {
            // M2 Clinic core routes + canonical client ClinicEndpoint routes
            ["patients"] | ["patient-workspaces"] | ["patient-roster"] => {
                self.route_patients_list(req, query)
            }
            ["patients", workspace_id, "workspace"] => {
                self.route_patient_workspace(req, workspace_id)
            }
            ["patient-workspaces", "events"] => self.route_workspace_events_stream(req, query),
            ["patient-workspaces", workspace_id] => {
                self.route_client_workspace_detail(req, workspace_id)
            }
            [
                "patient-workspaces",
                workspace_id,
                "chats",
                chat_id,
                "snapshot",
            ] => self.route_client_chat_snapshot(req, workspace_id, chat_id),
            ["patients", workspace_id, "citations", source_key]
            | [
                "patient-workspaces",
                workspace_id,
                "chats",
                _,
                "sources",
                source_key,
            ] => self.route_patient_citation(req, workspace_id, source_key),
            ["workspaces", workspace_id, "chats", chat_id] => {
                self.route_chat_snapshot(req, workspace_id, chat_id)
            }
            ["workspaces", workspace_id, "chats", chat_id, "commands"] => {
                self.route_chat_commands(req, Some((workspace_id, chat_id)))
            }
            ["commands"] => self.route_chat_commands(req, None),
            ["workspaces", workspace_id, "chats", chat_id, "stream"] => {
                self.route_chat_stream(req, workspace_id, chat_id, query)
            }
            ["workspaces", workspace_id, "chats", chat_id, "draft"]
            | ["physician-state", "drafts", workspace_id, chat_id] => {
                self.route_chat_draft(req, workspace_id, chat_id)
            }

            // M3 Clinic Workflow routes
            ["today"] => self.route_today_get(req),
            ["today", "standing"] => self.route_today_standing(req),
            ["messages", "ingest"] => self.route_messages_ingest(req),
            ["messages", "decide"] => self.route_messages_decide(req),
            ["messages", "undo"] => self.route_messages_undo(req),
            ["replies", conversation_id] => self.route_replies(req, conversation_id, None),
            ["replies", conversation_id, action] => {
                self.route_replies(req, conversation_id, Some(action))
            }
            ["patients", workspace_id, "memory"] => self.route_patient_memory(req, workspace_id),
            ["patients", workspace_id, "search"] => {
                self.route_patient_semantic_search(req, workspace_id)
            }
            ["clinical-library", "query"] => self.route_clinical_library_query(req),

            // M3 Connected Care Clinic invitation routes
            ["connected-care", "invitations"] => self.route_connected_care_create_invitation(req),
            ["connected-care", "invitations", invitation_id] => {
                self.route_connected_care_get_invitation(req, invitation_id)
            }
            ["connected-care", "invitations", invitation_id, "cancel"] => {
                self.route_connected_care_cancel_invitation(req, invitation_id)
            }
            ["connected-care", "invitations", invitation_id, "resend"] => {
                self.route_connected_care_resend_invitation(req, invitation_id)
            }

            _ => HttpResponse::not_found(),
        }
    }

    fn route_auth(&mut self, req: &HttpRequest, segments: &[&str]) -> HttpResponse {
        match (req.method, segments) {
            (HttpMethod::Post, ["auth", "request-link"] | ["auth", "sign-in"]) => {
                let body: RequestLinkBody = match serde_json::from_slice(&req.body) {
                    Ok(parsed) => parsed,
                    Err(err) => return HttpResponse::bad_request(err.to_string()),
                };
                let kind: VerificationTokenKind = body
                    .kind
                    .map(Into::into)
                    .unwrap_or(VerificationTokenKind::SignInLink);
                let now_secs = self.clock_secs;
                match self.auth.initiate_sign_in(&body.email, kind, now_secs) {
                    Ok(resp) => {
                        if let Err(err) = self.wal.append(&WalEntry::InitiateSignIn {
                            raw_email: body.email,
                            kind: kind.into(),
                            now_secs,
                        }) {
                            return wal_failure_response(&err);
                        }
                        HttpResponse::json_clinical(200, &resp)
                    }
                    Err(err) => auth_error_response(err),
                }
            }
            (HttpMethod::Post, ["auth", "verify"]) => {
                let body: VerifyTokenBody = match serde_json::from_slice(&req.body) {
                    Ok(parsed) => parsed,
                    Err(err) => return HttpResponse::bad_request(err.to_string()),
                };
                let now_secs = self.clock_secs;
                match self
                    .auth
                    .verify_token(&body.email, &body.token, now_secs, body.fixed_ends_at)
                {
                    Ok(verified) => {
                        if let Err(err) = self.wal.append(&WalEntry::VerifyToken {
                            raw_email: body.email,
                            raw_token: body.token,
                            now_secs,
                            fixed_ends_at: body.fixed_ends_at,
                        }) {
                            return wal_failure_response(&err);
                        }
                        HttpResponse::json_clinical(200, &verified)
                    }
                    Err(err) => auth_error_response(err),
                }
            }
            (HttpMethod::Post, ["auth", "sign-out"]) => {
                let bearer_session = req.bearer_token().and_then(|raw| SessionId::new(raw).ok());
                let body_session = if req.body.is_empty() {
                    None
                } else {
                    serde_json::from_slice::<SignOutBody>(&req.body)
                        .ok()
                        .and_then(|b| b.session)
                };
                let now_secs = self.clock_secs;
                for sess in bearer_session.into_iter().chain(body_session) {
                    self.auth.sign_out(&sess, now_secs);
                    if let Err(err) = self.wal.append(&WalEntry::SignOut {
                        session_id: sess,
                        now_secs,
                    }) {
                        return wal_failure_response(&err);
                    }
                }
                HttpResponse::json_clinical(200, &serde_json::json!({ "signed_out": true }))
            }
            (HttpMethod::Get, ["auth", "session"] | ["auth", "whoami"]) => {
                match self.authenticate_request(req) {
                    Ok(whoami) => HttpResponse::json_clinical(200, &whoami),
                    Err(resp) => resp,
                }
            }
            _ => HttpResponse::method_not_allowed(),
        }
    }

    pub(crate) fn authenticate_request(
        &mut self,
        req: &HttpRequest,
    ) -> Result<WhoAmIPayload, HttpResponse> {
        let bearer = match req.bearer_token() {
            None => None,
            Some(raw) => match SessionId::new(raw) {
                Ok(id) => Some(id),
                Err(err) => return Err(auth_error_response(AuthError::InvalidSessionId(err))),
            },
        };
        let now_secs = self.clock_secs;
        match self.auth.whoami(bearer.as_ref(), now_secs) {
            Ok(whoami) => {
                if matches!(
                    whoami.renewal,
                    ef_clinic_domain::auth::SessionActivityDecision::Extended { .. }
                ) && let Some(session_id) = bearer
                    && let Err(err) = self.wal.append(&WalEntry::TouchSession {
                        session_id,
                        now_secs,
                    })
                {
                    return Err(wal_failure_response(&err));
                }
                Ok(whoami)
            }
            Err(err) => Err(auth_error_response(err)),
        }
    }

    fn route_patients_list(&mut self, req: &HttpRequest, query: Option<&str>) -> HttpResponse {
        if req.method != HttpMethod::Get {
            return HttpResponse::method_not_allowed();
        }
        let whoami = match self.authenticate_request(req) {
            Ok(w) => w,
            Err(resp) => return resp,
        };
        let q = query_param(query, "q").or_else(|| query_param(query, "query"));
        let roster =
            self.evidence
                .query_patient_roster(&whoami.practice_id, &whoami.physician_id, q, false);
        match roster {
            ef_clinic_domain::workspace::PatientRosterState::Failed { .. } => {
                HttpResponse::json_clinical(503, &roster)
            }
            other => HttpResponse::json_clinical(200, &other),
        }
    }

    fn route_patient_workspace(
        &mut self,
        req: &HttpRequest,
        raw_workspace_id: &str,
    ) -> HttpResponse {
        if req.method != HttpMethod::Get {
            return HttpResponse::method_not_allowed();
        }
        let whoami = match self.authenticate_request(req) {
            Ok(w) => w,
            Err(resp) => return resp,
        };
        let workspace_id = match WorkspaceId::new(raw_workspace_id) {
            Ok(id) => id,
            Err(err) => return HttpResponse::bad_request(err.to_string()),
        };

        let membership = match self.evidence.membership_for(
            &whoami.practice_id,
            &whoami.physician_id,
            &workspace_id,
        ) {
            Ok(tier) => tier,
            Err(err) => return workspace_error_response(err),
        };

        if membership == MembershipTier::Denied {
            return HttpResponse::not_found();
        }

        let summary = match self.evidence.query_patient_roster(
            &whoami.practice_id,
            &whoami.physician_id,
            None,
            false,
        ) {
            ef_clinic_domain::workspace::PatientRosterState::Loaded { patients, .. } => patients
                .into_iter()
                .find(|p| p.patient_workspace_id == workspace_id),
            _ => None,
        };

        let chats = match membership {
            MembershipTier::Allowed => {
                let mut map = BTreeMap::new();
                for ((ws, chat_id), engine) in &self.chats {
                    if ws == &workspace_id {
                        let records: Vec<ExchangeRecord> =
                            engine.transcript_snapshot().into_iter().cloned().collect();
                        map.insert(chat_id.as_str().to_owned(), records);
                    }
                }
                Some(map)
            }
            MembershipTier::ClinicDemographic | MembershipTier::Denied => None,
        };

        HttpResponse::json_clinical(
            200,
            &WorkspaceDetailResponse {
                workspace_id,
                membership,
                summary,
                chats,
            },
        )
    }

    fn route_client_workspace_detail(
        &mut self,
        req: &HttpRequest,
        raw_workspace_id: &str,
    ) -> HttpResponse {
        if req.method != HttpMethod::Get {
            return HttpResponse::method_not_allowed();
        }
        let whoami = match self.authenticate_request(req) {
            Ok(w) => w,
            Err(resp) => return resp,
        };
        let workspace_id = match WorkspaceId::new(raw_workspace_id) {
            Ok(id) => id,
            Err(err) => return HttpResponse::bad_request(err.to_string()),
        };

        let membership = match self.evidence.membership_for(
            &whoami.practice_id,
            &whoami.physician_id,
            &workspace_id,
        ) {
            Ok(tier) => tier,
            Err(err) => return workspace_error_response(err),
        };
        if membership == MembershipTier::Denied {
            return HttpResponse::not_found();
        }

        let display_name = match self.evidence.query_patient_roster(
            &whoami.practice_id,
            &whoami.physician_id,
            None,
            false,
        ) {
            ef_clinic_domain::workspace::PatientRosterState::Loaded { patients, .. } => patients
                .into_iter()
                .find(|p| p.patient_workspace_id == workspace_id)
                .map(|p| p.display_name),
            _ => None,
        };
        let Some(display_name) = display_name else {
            return HttpResponse::not_found();
        };

        let default_chat_id = self
            .chats
            .keys()
            .find(|(ws, _)| ws == &workspace_id)
            .map(|(_, chat_id)| chat_id.clone())
            .unwrap_or_else(|| ChatId::new("chat-main").expect("valid static chat id"));

        HttpResponse::json_clinical(
            200,
            &ClientWorkspaceDetailWire {
                patient_workspace_id: workspace_id,
                display_name,
                membership,
                time_zone: "UTC".to_owned(),
                default_chat_id,
                patient_synopsis: None,
            },
        )
    }

    fn route_client_chat_snapshot(
        &mut self,
        req: &HttpRequest,
        raw_workspace_id: &str,
        raw_chat_id: &str,
    ) -> HttpResponse {
        if req.method != HttpMethod::Get {
            return HttpResponse::method_not_allowed();
        }
        let whoami = match self.authenticate_request(req) {
            Ok(w) => w,
            Err(resp) => return resp,
        };
        let (workspace_id, chat_id) = match parse_ws_and_chat(raw_workspace_id, raw_chat_id) {
            Ok(pair) => pair,
            Err(resp) => return resp,
        };
        if let Err(err) = self.evidence.require_clinical_access(
            &whoami.practice_id,
            &whoami.physician_id,
            &workspace_id,
        ) {
            return workspace_error_response(err);
        }

        let transcript = self
            .chats
            .get(&(workspace_id.clone(), chat_id.clone()))
            .map(|engine| engine.transcript_snapshot().into_iter().cloned().collect())
            .unwrap_or_default();

        HttpResponse::json_clinical(
            200,
            &ClientChatSnapshotWire {
                workspace_id,
                chat_id,
                transcript,
                sources: BTreeMap::new(),
                queued: Vec::new(),
                archived: false,
            },
        )
    }

    fn route_workspace_events_stream(
        &mut self,
        req: &HttpRequest,
        query: Option<&str>,
    ) -> HttpResponse {
        if req.method != HttpMethod::Get {
            return HttpResponse::method_not_allowed();
        }
        let whoami = match self.authenticate_request(req) {
            Ok(w) => w,
            Err(resp) => return resp,
        };

        let after_event_id = query_param(query, "since")
            .or_else(|| query_param(query, "after_event_id"))
            .or_else(|| req.header("last-event-id"))
            .and_then(|raw| raw.parse::<u64>().ok())
            .unwrap_or(0);

        let filter_ws =
            query_param(query, "workspace_id").and_then(|raw| WorkspaceId::new(raw).ok());
        let filter_chat = query_param(query, "chat_id").and_then(|raw| ChatId::new(raw).ok());

        let mut sse = String::new();
        for ((ws_id, chat_id), engine) in &self.chats {
            if let Some(ref target_ws) = filter_ws
                && ws_id != target_ws
            {
                continue;
            }
            if let Some(ref target_chat) = filter_chat
                && chat_id != target_chat
            {
                continue;
            }
            if self
                .evidence
                .require_clinical_access(&whoami.practice_id, &whoami.physician_id, ws_id)
                .is_err()
            {
                continue;
            }
            for event in engine.events() {
                if let Some(id) = stream_event_id(event)
                    && id <= after_event_id
                {
                    continue;
                }
                sse.push_str(&format_sse_event(event));
            }
        }
        sse.push_str(&format_sse_event(&ChatStreamEvent::KeepAlive));
        HttpResponse::sse_stream(sse)
    }

    fn route_patient_citation(
        &mut self,
        req: &HttpRequest,
        raw_workspace_id: &str,
        raw_source_key: &str,
    ) -> HttpResponse {
        if req.method != HttpMethod::Get {
            return HttpResponse::method_not_allowed();
        }
        let whoami = match self.authenticate_request(req) {
            Ok(w) => w,
            Err(resp) => return resp,
        };
        let workspace_id = match WorkspaceId::new(raw_workspace_id) {
            Ok(id) => id,
            Err(err) => return HttpResponse::bad_request(err.to_string()),
        };
        let source_key = match SourceKey::new(raw_source_key) {
            Ok(key) => key,
            Err(err) => return HttpResponse::bad_request(err.to_string()),
        };

        match self.evidence.fetch_cited_source(
            &whoami.practice_id,
            &whoami.physician_id,
            &workspace_id,
            &source_key,
        ) {
            Ok(payload) => HttpResponse::json_clinical(200, &payload),
            Err(err) => workspace_error_response(err),
        }
    }

    fn route_chat_snapshot(
        &mut self,
        req: &HttpRequest,
        raw_workspace_id: &str,
        raw_chat_id: &str,
    ) -> HttpResponse {
        if req.method != HttpMethod::Get {
            return HttpResponse::method_not_allowed();
        }
        let whoami = match self.authenticate_request(req) {
            Ok(w) => w,
            Err(resp) => return resp,
        };
        let (workspace_id, chat_id) = match parse_ws_and_chat(raw_workspace_id, raw_chat_id) {
            Ok(pair) => pair,
            Err(resp) => return resp,
        };
        if let Err(err) = self.evidence.require_clinical_access(
            &whoami.practice_id,
            &whoami.physician_id,
            &workspace_id,
        ) {
            return workspace_error_response(err);
        }

        let exchanges = self
            .chats
            .get(&(workspace_id.clone(), chat_id.clone()))
            .map(|engine| engine.transcript_snapshot().into_iter().cloned().collect())
            .unwrap_or_default();
        let draft = self.drafts.get(&workspace_id, chat_id.as_str());

        HttpResponse::json_clinical(
            200,
            &ChatSnapshotResponse {
                workspace_id,
                chat_id,
                exchanges,
                draft,
            },
        )
    }

    fn route_chat_commands(
        &mut self,
        req: &HttpRequest,
        route_scope: Option<(&str, &str)>,
    ) -> HttpResponse {
        if req.method != HttpMethod::Post {
            return HttpResponse::method_not_allowed();
        }
        let whoami = match self.authenticate_request(req) {
            Ok(w) => w,
            Err(resp) => return resp,
        };

        let envelope: CommandEnvelope = match serde_json::from_slice(&req.body) {
            Ok(env) => env,
            Err(err) => return HttpResponse::bad_request(err.to_string()),
        };

        let (payload_ws, payload_chat) = match &envelope.command {
            CommandPayload::MessageSubmit(p) => (p.patient_workspace_id.clone(), p.chat_id.clone()),
            CommandPayload::WorkStop(p) => (p.patient_workspace_id.clone(), p.chat_id.clone()),
            CommandPayload::WorkResume(p) => (p.patient_workspace_id.clone(), p.chat_id.clone()),
        };

        let (workspace_id, chat_id) = match route_scope {
            Some((raw_ws, raw_chat)) => {
                let (ws, chat) = match parse_ws_and_chat(raw_ws, raw_chat) {
                    Ok(pair) => pair,
                    Err(resp) => return resp,
                };
                // Enforce path-to-payload workspace/chat consistency.
                if payload_ws != ws || payload_chat != chat {
                    return HttpResponse::bad_request(
                        "command payload workspace/chat does not match route",
                    );
                }
                (ws, chat)
            }
            None => (payload_ws, payload_chat),
        };

        if let Err(err) = self.evidence.require_clinical_access(
            &whoami.practice_id,
            &whoami.physician_id,
            &workspace_id,
        ) {
            return workspace_error_response(err);
        }

        let should_drive_worker = matches!(
            envelope.command,
            CommandPayload::MessageSubmit(_) | CommandPayload::WorkResume(_)
        );
        let now_secs = self.clock_secs;
        let retry_mode = self.retry_mode;

        let acceptance: CommandAcceptance = {
            let engine = self
                .chats
                .entry((workspace_id.clone(), chat_id.clone()))
                .or_insert_with(|| ChatEngine::new(retry_mode));
            match engine.execute_command(envelope.clone(), now_secs) {
                Ok(acc) => acc,
                Err(err) => return chat_error_response(err),
            }
        };

        if !acceptance.deduplicated {
            if let Err(err) = self.wal.append(&WalEntry::ExecuteChatCommand {
                workspace_id: workspace_id.clone(),
                chat_id: chat_id.clone(),
                envelope,
                now_secs,
            }) {
                return wal_failure_response(&err);
            }

            if should_drive_worker
                && let Err(resp) = self.drive_work_item(
                    &whoami.practice_id,
                    &whoami.physician_id,
                    &workspace_id,
                    &chat_id,
                    &acceptance.work_item_id,
                )
            {
                return resp;
            }
        }

        HttpResponse::json_clinical(200, &acceptance)
    }

    fn route_chat_stream(
        &mut self,
        req: &HttpRequest,
        raw_workspace_id: &str,
        raw_chat_id: &str,
        query: Option<&str>,
    ) -> HttpResponse {
        if req.method != HttpMethod::Get {
            return HttpResponse::method_not_allowed();
        }
        let whoami = match self.authenticate_request(req) {
            Ok(w) => w,
            Err(resp) => return resp,
        };
        let (workspace_id, chat_id) = match parse_ws_and_chat(raw_workspace_id, raw_chat_id) {
            Ok(pair) => pair,
            Err(resp) => return resp,
        };
        if let Err(err) = self.evidence.require_clinical_access(
            &whoami.practice_id,
            &whoami.physician_id,
            &workspace_id,
        ) {
            return workspace_error_response(err);
        }

        if query_param(query, "resync") == Some("true") {
            let reason = "lossy_bus_resync_requested".to_owned();
            let retry_mode = self.retry_mode;
            let engine = self
                .chats
                .entry((workspace_id.clone(), chat_id.clone()))
                .or_insert_with(|| ChatEngine::new(retry_mode));
            engine.emit_resync(reason.clone());
            if let Err(err) = self.wal.append(&WalEntry::EmitResync {
                workspace_id: workspace_id.clone(),
                chat_id: chat_id.clone(),
                reason,
            }) {
                return wal_failure_response(&err);
            }
        }

        let after_event_id = query_param(query, "after_event_id")
            .or_else(|| req.header("last-event-id"))
            .and_then(|raw| raw.parse::<u64>().ok())
            .unwrap_or(0);

        let mut sse = String::new();
        if let Some(engine) = self.chats.get(&(workspace_id, chat_id)) {
            for event in engine.events() {
                if let Some(id) = stream_event_id(event)
                    && id <= after_event_id
                {
                    continue;
                }
                sse.push_str(&format_sse_event(event));
            }
        }
        sse.push_str(&format_sse_event(&ChatStreamEvent::KeepAlive));

        HttpResponse::sse_stream(sse)
    }

    fn route_chat_draft(
        &mut self,
        req: &HttpRequest,
        raw_workspace_id: &str,
        raw_chat_id: &str,
    ) -> HttpResponse {
        let whoami = match self.authenticate_request(req) {
            Ok(w) => w,
            Err(resp) => return resp,
        };
        let (workspace_id, chat_id) = match parse_ws_and_chat(raw_workspace_id, raw_chat_id) {
            Ok(pair) => pair,
            Err(resp) => return resp,
        };
        if let Err(err) = self.evidence.require_clinical_access(
            &whoami.practice_id,
            &whoami.physician_id,
            &workspace_id,
        ) {
            return workspace_error_response(err);
        }

        match req.method {
            HttpMethod::Get => {
                let draft = self.drafts.get(&workspace_id, chat_id.as_str());
                HttpResponse::json_clinical(200, &draft)
            }
            HttpMethod::Put => {
                let body: PutDraftBody = match serde_json::from_slice(&req.body) {
                    Ok(parsed) => parsed,
                    Err(err) => return HttpResponse::bad_request(err.to_string()),
                };
                match self.drafts.put(
                    &workspace_id,
                    chat_id.as_str(),
                    body.text.clone(),
                    body.expected_revision,
                ) {
                    DraftSaveOutcome::Saved(saved) => {
                        if let Err(err) = self.wal.append(&WalEntry::PutDraft {
                            workspace_id,
                            chat_id,
                            text: body.text,
                            expected_revision: body.expected_revision,
                        }) {
                            return wal_failure_response(&err);
                        }
                        HttpResponse::json_clinical(200, &saved)
                    }
                    DraftSaveOutcome::Conflict { current } => HttpResponse::json_clinical(
                        409,
                        &serde_json::json!({
                            "error": "draft_conflict",
                            "current": current,
                        }),
                    ),
                    DraftSaveOutcome::TransientFailure { message } => HttpResponse::json_clinical(
                        503,
                        &serde_json::json!({
                            "error": "draft_save_failed",
                            "detail": message,
                        }),
                    ),
                }
            }
            HttpMethod::Head | HttpMethod::Post => HttpResponse::method_not_allowed(),
        }
    }

    fn drive_work_item(
        &mut self,
        practice_id: &PracticeId,
        physician_id: &PhysicianId,
        workspace_id: &WorkspaceId,
        chat_id: &ChatId,
        work_item_id: &WorkItemId,
    ) -> Result<(), HttpResponse> {
        let mut provider = self.queued_providers.pop_front().unwrap_or_else(|| {
            StubProvider::clinical_two_round_fixture(self.default_cited_source.clone())
        });
        let search_tool = SearchChartTool {
            practice_id: practice_id.clone(),
            physician_id: physician_id.clone(),
            store: &self.evidence,
        };
        let read_tool = ReadRecordsTool {
            practice_id: practice_id.clone(),
            physician_id: physician_id.clone(),
            store: &self.evidence,
        };
        let tools: [&dyn AgentTool; 2] = [&search_tool, &read_tool];

        for _ in 0..MAX_FAILURE_ATTEMPTS {
            let now_secs = self.clock_secs;
            let Some(engine) = self.chats.get_mut(&(workspace_id.clone(), chat_id.clone())) else {
                return Ok(());
            };
            let Some((claimed_id, _run_id)) = engine.claim_due_work("worker-m2-local", now_secs)
            else {
                return Ok(());
            };
            if &claimed_id != work_item_id {
                return Ok(());
            }

            let events_before = engine.events().len();
            let cancel = CancellationToken::new();
            let outcome = match self.agent_loop.run_turn_attempt(
                engine,
                work_item_id,
                &mut provider,
                &tools,
                &cancel,
                now_secs,
            ) {
                Ok(out) => out,
                Err(err) => {
                    return Err(HttpResponse::json_clinical(
                        500,
                        &serde_json::json!({
                            "error": "harness_execution_failed",
                            "detail": err.to_string(),
                        }),
                    ));
                }
            };

            // Collect WAL entries from emitted events + exchange state so recovery replays identically.
            let mut new_entries = Vec::new();
            for event in &engine.events()[events_before..] {
                match event {
                    ChatStreamEvent::StepRecorded { step, .. } => {
                        new_entries.push(WalEntry::RecordStep {
                            workspace_id: workspace_id.clone(),
                            chat_id: chat_id.clone(),
                            work_item_id: work_item_id.clone(),
                            step: step.clone(),
                            now_secs,
                        });
                    }
                    ChatStreamEvent::DeltaChunk { text_delta, .. } => {
                        new_entries.push(WalEntry::AppendDelta {
                            workspace_id: workspace_id.clone(),
                            chat_id: chat_id.clone(),
                            work_item_id: work_item_id.clone(),
                            text_delta: text_delta.clone(),
                            now_secs,
                        });
                    }
                    ChatStreamEvent::Completed { .. } => {
                        if let Some(exchange) = engine.exchange(work_item_id)
                            && let AnswerState::Finished {
                                response_id,
                                attempt_id,
                                finished_at,
                                segments,
                                ..
                            } = &exchange.answer
                        {
                            new_entries.push(WalEntry::CompleteExchange {
                                workspace_id: workspace_id.clone(),
                                chat_id: chat_id.clone(),
                                work_item_id: work_item_id.clone(),
                                response_id: response_id.clone(),
                                attempt_id: attempt_id.clone(),
                                finished_at: finished_at.clone(),
                                segments: segments.clone(),
                            });
                        }
                    }
                    ChatStreamEvent::CouldntFinish { .. }
                    | ChatStreamEvent::Cancelled { .. }
                    | ChatStreamEvent::ResyncRequired { .. }
                    | ChatStreamEvent::KeepAlive => {}
                }
            }

            if matches!(
                outcome,
                TurnExecutionOutcome::ScheduledRetry { .. }
                    | TurnExecutionOutcome::ExhaustedCouldntFinish { .. }
            ) {
                new_entries.push(WalEntry::RecordAttemptFailure {
                    workspace_id: workspace_id.clone(),
                    chat_id: chat_id.clone(),
                    work_item_id: work_item_id.clone(),
                    now_secs,
                });
            }

            for entry in &new_entries {
                if let Err(err) = self.wal.append(entry) {
                    return Err(wal_failure_response(&err));
                }
            }

            match outcome {
                TurnExecutionOutcome::Finished { .. }
                | TurnExecutionOutcome::StoppedMidStream
                | TurnExecutionOutcome::ExhaustedCouldntFinish { .. }
                | TurnExecutionOutcome::RejectedNonRetryable { .. } => break,
                TurnExecutionOutcome::ScheduledRetry { .. } => {
                    if self.retry_mode == RetryBackoffMode::Hosted {
                        break;
                    }
                }
            }
        }

        Ok(())
    }

    /// Replays a single persisted [`WalEntry`] through the live domain owners.
    pub fn replay_wal_entry(&mut self, entry: WalEntry) {
        let retry_mode = self.retry_mode;
        match entry {
            WalEntry::UpsertAccount { account } => self.auth.upsert_account(account),
            WalEntry::DeactivateAccount { email } => self.auth.deactivate_account(&email),
            WalEntry::SetSessionsEndedBefore { email, fence_secs } => {
                self.auth.set_sessions_ended_before(&email, fence_secs);
            }
            WalEntry::InitiateSignIn {
                raw_email,
                kind,
                now_secs,
            } => {
                let _ = self
                    .auth
                    .initiate_sign_in(&raw_email, kind.into(), now_secs);
            }
            WalEntry::VerifyToken {
                raw_email,
                raw_token,
                now_secs,
                fixed_ends_at,
            } => {
                let _ = self
                    .auth
                    .verify_token(&raw_email, &raw_token, now_secs, fixed_ends_at);
            }
            WalEntry::TouchSession {
                session_id,
                now_secs,
            } => {
                let _ = self.auth.whoami(Some(&session_id), now_secs);
            }
            WalEntry::SignOut {
                session_id,
                now_secs,
            } => {
                self.auth.sign_out(&session_id, now_secs);
            }
            WalEntry::ExpireSession {
                session_id,
                now_secs,
            } => {
                self.auth.expire_session(&session_id, now_secs);
            }
            WalEntry::RegisterWorkspace {
                workspace_id,
                practice_id,
                display_name,
                panel_physicians,
            } => {
                self.evidence.register_workspace(
                    workspace_id,
                    practice_id,
                    display_name,
                    panel_physicians,
                );
            }
            WalEntry::InsertSource { payload } => {
                self.intelligence.index_source(&payload);
                self.evidence.insert_source(payload);
            }
            WalEntry::WithdrawSource {
                workspace_id,
                source_key,
            } => {
                self.intelligence.mark_withdrawn(&workspace_id, &source_key);
                let _ = self.evidence.withdraw_source(&workspace_id, &source_key);
            }
            WalEntry::ExecuteChatCommand {
                workspace_id,
                chat_id,
                envelope,
                now_secs,
            } => {
                let engine = self
                    .chats
                    .entry((workspace_id, chat_id))
                    .or_insert_with(|| ChatEngine::new(retry_mode));
                let _ = engine.execute_command(envelope, now_secs);
            }
            WalEntry::RecordStep {
                workspace_id,
                chat_id,
                work_item_id,
                step,
                now_secs,
            } => {
                if let Some(engine) = self.chats.get_mut(&(workspace_id, chat_id)) {
                    let _ = engine.record_step(&work_item_id, step, now_secs);
                }
            }
            WalEntry::AppendDelta {
                workspace_id,
                chat_id,
                work_item_id,
                text_delta,
                now_secs,
            } => {
                if let Some(engine) = self.chats.get_mut(&(workspace_id, chat_id)) {
                    let _ = engine.append_delta(&work_item_id, &text_delta, now_secs);
                }
            }
            WalEntry::CompleteExchange {
                workspace_id,
                chat_id,
                work_item_id,
                response_id,
                attempt_id,
                finished_at,
                segments,
            } => {
                if let Some(engine) = self.chats.get_mut(&(workspace_id, chat_id)) {
                    let _ = engine.complete_exchange(
                        &work_item_id,
                        response_id,
                        attempt_id,
                        finished_at,
                        segments,
                    );
                }
            }
            WalEntry::RecordAttemptFailure {
                workspace_id,
                chat_id,
                work_item_id,
                now_secs,
            } => {
                if let Some(engine) = self.chats.get_mut(&(workspace_id, chat_id)) {
                    let _ = engine.record_attempt_failure(&work_item_id, now_secs);
                }
            }
            WalEntry::EmitResync {
                workspace_id,
                chat_id,
                reason,
            } => {
                let engine = self
                    .chats
                    .entry((workspace_id, chat_id))
                    .or_insert_with(|| ChatEngine::new(retry_mode));
                engine.emit_resync(reason);
            }
            WalEntry::PutDraft {
                workspace_id,
                chat_id,
                text,
                expected_revision,
            } => {
                let _ = self
                    .drafts
                    .put(&workspace_id, chat_id.as_str(), text, expected_revision);
            }
            WalEntry::AddTodayItem { item } => {
                self.today_candidates.push(item);
            }
            WalEntry::AddScheduledVisit { visit } => {
                self.scheduled_visits.push(visit);
            }
            WalEntry::SetChannelHealth { channel_id, status } => {
                self.today_engine
                    .set_channel_health(channel_id.clone(), status.clone());
                self.matching_engine.register_channel(channel_id, status);
            }
            WalEntry::MarkTodaySeen {
                physician_id,
                item_id,
                version_id,
            } => {
                self.today_engine
                    .mark_seen(&physician_id, &item_id, version_id);
            }
            WalEntry::MarkTodayDone { physician_id, item } => {
                let _ = self.today_engine.mark_done(&physician_id, &item);
            }
            WalEntry::IngestChannelMessage {
                channel_id,
                cursor_seq,
                conversation_id,
                message_id,
                body,
                unreadable_reason,
                auto_signal,
                now_secs,
            } => {
                let frame = if let Some(reason) = unreadable_reason {
                    RawChannelFrame::Unreadable { cursor_seq, reason }
                } else if let (Some(conv), Some(mid), Some(text)) =
                    (conversation_id, message_id, body)
                {
                    RawChannelFrame::Readable {
                        cursor_seq,
                        conversation_id: conv,
                        message_id: mid,
                        body: text,
                    }
                } else {
                    return;
                };
                let _ = self.apply_ingest_message(&channel_id, frame, auto_signal, now_secs);
            }
            WalEntry::DecideMessageMove {
                physician_id,
                practice_id,
                conversation_id,
                destination,
                now_secs,
            } => {
                let _ = self.apply_decide_move(
                    &practice_id,
                    &physician_id,
                    &conversation_id,
                    destination.into(),
                    now_secs,
                );
            }
            WalEntry::UndoMessageMove { conversation_id } => {
                let _ = self.matching_engine.undo_move(&conversation_id);
            }
            WalEntry::ReplyCompleteReview {
                conversation_id,
                for_patient,
                proposed_reply_text,
                basis_citations,
                flagged_concern,
                answered_by_human_reply,
            } => {
                if let Ok((review_id, _)) = self.reply_engine.start_review(&conversation_id) {
                    let _ = self.reply_engine.complete_review(
                        &conversation_id,
                        ReviewCompletion {
                            review_id,
                            for_patient,
                            proposed_reply_text,
                            basis_citations,
                            flagged_concern,
                            answered_by_human_reply,
                        },
                    );
                }
            }
            WalEntry::ReplyEditDraft {
                physician_id,
                practice_id,
                conversation_id,
                draft_text,
            } => {
                let _ = self.reply_engine.edit_personal_draft(
                    &physician_id,
                    &conversation_id,
                    draft_text,
                    |phys, ws| {
                        self.evidence
                            .require_clinical_access(&practice_id, phys, ws)
                            .is_ok()
                    },
                );
            }
            WalEntry::ReplyRestoreSystem {
                physician_id,
                practice_id,
                conversation_id,
            } => {
                let _ = self.reply_engine.restore_system_reply(
                    &physician_id,
                    &conversation_id,
                    |phys, ws| {
                        self.evidence
                            .require_clinical_access(&practice_id, phys, ws)
                            .is_ok()
                    },
                );
            }
            WalEntry::ReplyClose {
                physician_id,
                practice_id,
                conversation_id,
            } => {
                let _ =
                    self.reply_engine
                        .close_reply(&physician_id, &conversation_id, |phys, ws| {
                            self.evidence
                                .require_clinical_access(&practice_id, phys, ws)
                                .is_ok()
                        });
            }
            WalEntry::ReplyUndoClose {
                physician_id,
                practice_id,
                conversation_id,
            } => {
                let _ = self.reply_engine.undo_close_reply(
                    &physician_id,
                    &conversation_id,
                    |phys, ws| {
                        self.evidence
                            .require_clinical_access(&practice_id, phys, ws)
                            .is_ok()
                    },
                );
            }
            WalEntry::MemoryRecordEntry {
                workspace_id,
                topic,
                statement,
                observed_at,
                authority,
            } => {
                let store = self
                    .patient_memories
                    .entry(workspace_id.clone())
                    .or_insert_with(|| PatientMemoryStore::new(workspace_id));
                let _ = store.record_entry(topic, statement, observed_at, authority);
            }
            WalEntry::MemoryRunRounds {
                workspace_id,
                observations,
                abort_mid_run,
            } => {
                let store = self
                    .patient_memories
                    .entry(workspace_id.clone())
                    .or_insert_with(|| PatientMemoryStore::new(workspace_id));
                let _ = store.run_rounds(&observations, abort_mid_run);
            }
            WalEntry::MemoryRetireEntry {
                workspace_id,
                entry_id,
                resolved_at,
                reason,
            } => {
                if let Some(store) = self.patient_memories.get_mut(&workspace_id) {
                    let _ = store.retire_entry(&entry_id, resolved_at, reason);
                }
            }
            WalEntry::CreateConnectedCareInvitation { request } => {
                let pair = request.pair.clone();
                if let Ok(issued) = self.connected_care.create_invitation(request) {
                    self.invitation_pairs.insert(issued.invitation_id, pair);
                }
            }
            WalEntry::ResendConnectedCareInvitation { pair, resent_at } => {
                let _ = self.connected_care.resend_invitation(&pair, resent_at);
            }
            WalEntry::CancelConnectedCareInvitation { pair, cancelled_at } => {
                let _ = self.connected_care.cancel_invitation(&pair, cancelled_at);
            }
        }
    }

    /// Helper for tests verifying attempt ID formatting.
    #[must_use]
    pub fn telemetry_span_count(&self) -> usize {
        self.agent_loop.telemetry_spans().len()
    }
}

fn parse_ws_and_chat(
    raw_workspace_id: &str,
    raw_chat_id: &str,
) -> Result<(WorkspaceId, ChatId), HttpResponse> {
    let workspace_id =
        WorkspaceId::new(raw_workspace_id).map_err(|e| HttpResponse::bad_request(e.to_string()))?;
    let chat_id = ChatId::new(raw_chat_id).map_err(|e| HttpResponse::bad_request(e.to_string()))?;
    Ok((workspace_id, chat_id))
}

fn auth_error_response(err: AuthError) -> HttpResponse {
    let status = err.http_status_code();
    match err {
        AuthError::Refused(refusal) => HttpResponse::json_clinical(status, &refusal),
        AuthError::MailDeliveryFailed => HttpResponse::json_clinical(
            status,
            &serde_json::json!({
                "error": "mail_delivery_failed",
                "detail": "Sign-in email could not be delivered right now.",
            }),
        ),
        AuthError::BlankOrMalformedCredentials | AuthError::InvalidSessionId(_) => {
            HttpResponse::json_clinical(
                status,
                &serde_json::json!({
                    "error": "invalid_credentials",
                    "detail": err.to_string(),
                }),
            )
        }
    }
}

pub(crate) fn workspace_error_response(err: WorkspaceAccessError) -> HttpResponse {
    let status = err.http_status_code();
    match err {
        WorkspaceAccessError::NotFound => HttpResponse::not_found(),
        WorkspaceAccessError::SourceWithdrawn => HttpResponse::json_clinical(
            status,
            &serde_json::json!({
                "status": "unavailable",
                "reason": "source_withdrawn",
                "detail": "This cited clinical record is no longer available.",
            }),
        ),
        WorkspaceAccessError::ClinicalDisclosureForbidden => HttpResponse::json_clinical(
            status,
            &serde_json::json!({
                "error": "clinical_disclosure_forbidden",
                "detail": err.to_string(),
            }),
        ),
        WorkspaceAccessError::StoreUnavailable { message } => HttpResponse::json_clinical(
            status,
            &serde_json::json!({
                "error": "store_unavailable",
                "detail": message,
            }),
        ),
    }
}

fn chat_error_response(err: ChatCommandError) -> HttpResponse {
    let status = err.http_status_code();
    HttpResponse::json_clinical(
        status,
        &serde_json::json!({
            "error": "chat_command_error",
            "detail": err.to_string(),
        }),
    )
}

pub(crate) fn wal_failure_response(err: &WalError) -> HttpResponse {
    HttpResponse::json_clinical(
        503,
        &serde_json::json!({
            "error": "persistence_unavailable",
            "detail": err.to_string(),
        }),
    )
}
