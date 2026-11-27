//! Headless Curie Clinic HTTP/SSE service and router mounted strictly under `/elysianfields/`.
//!
//! Cross-compiles cleanly on both native hosts and `wasm32-unknown-unknown`.

use std::collections::{BTreeMap, VecDeque};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use ef_clinic_domain::auth::{
    AuthAuthority, AuthError, EmailAddress, PhysicianAccount, SignInMode, VerificationTokenKind,
    WhoAmIPayload,
};
use ef_clinic_domain::chat::{
    AnswerState, ChatCommandError, ChatEngine, ChatStreamEvent, CommandAcceptance, CommandEnvelope,
    CommandPayload, DraftSaveOutcome, DraftStore, ExchangeRecord, MAX_FAILURE_ATTEMPTS,
    PersistedDraft, RetryBackoffMode,
};
use ef_clinic_domain::workspace::{
    CitedSourcePayload, MembershipTier, PatientWorkspaceSummary, WorkspaceAccessError,
    WorkspaceEvidenceStore,
};
use ef_core::{
    ChatId, PhysicianId, PracticeId, SessionId, SourceKey, Subpath, WorkItemId, WorkspaceId,
};
use ef_harness::{
    AgentLoop, CancellationToken, ClinicalTool, ReadRecordsTool, SearchChartTool, StubProvider,
    TurnExecutionOutcome,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::http::{
    AUTH_MAX_BODY_BYTES, COMMANDS_MAX_BODY_BYTES, HttpMethod, HttpRequest, HttpResponse,
    format_sse_event, query_param, stream_event_id,
};
#[cfg(not(target_arch = "wasm32"))]
use crate::persistence::DurableClinicStore;
use crate::persistence::{InMemoryWal, WalEntry, WalError, WalSink, WalTokenKind};

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

/// Authoritative headless Curie Clinic vertical-slice service.
pub struct ClinicService {
    auth: AuthAuthority,
    evidence: WorkspaceEvidenceStore,
    chats: BTreeMap<(WorkspaceId, ChatId), ChatEngine>,
    drafts: DraftStore,
    retry_mode: RetryBackoffMode,
    agent_loop: AgentLoop,
    default_cited_source: SourceKey,
    queued_providers: VecDeque<StubProvider>,
    clock_secs: u64,
    wal: Box<dyn WalSink>,
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
        };
        for entry in entries {
            service.replay_wal_entry(entry);
        }
        Ok(service)
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

        // 3. Entry HTML under `/elysianfields/`.
        if subpath == "/" || subpath == "/index.html" {
            return match req.method {
                HttpMethod::Get | HttpMethod::Head => HttpResponse::html_entry(ENTRY_HTML),
                HttpMethod::Post | HttpMethod::Put => HttpResponse::method_not_allowed(),
            };
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
            ["patients"] => self.route_patients_list(req, query),
            ["patients", workspace_id, "workspace"] => {
                self.route_patient_workspace(req, workspace_id)
            }
            ["patients", workspace_id, "citations", source_key] => {
                self.route_patient_citation(req, workspace_id, source_key)
            }
            ["workspaces", workspace_id, "chats", chat_id] => {
                self.route_chat_snapshot(req, workspace_id, chat_id)
            }
            ["workspaces", workspace_id, "chats", chat_id, "commands"] => {
                self.route_chat_commands(req, workspace_id, chat_id)
            }
            ["workspaces", workspace_id, "chats", chat_id, "stream"] => {
                self.route_chat_stream(req, workspace_id, chat_id, query)
            }
            ["workspaces", workspace_id, "chats", chat_id, "draft"] => {
                self.route_chat_draft(req, workspace_id, chat_id)
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

    fn authenticate_request(&mut self, req: &HttpRequest) -> Result<WhoAmIPayload, HttpResponse> {
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
        let q = query_param(query, "q");
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
        raw_workspace_id: &str,
        raw_chat_id: &str,
    ) -> HttpResponse {
        if req.method != HttpMethod::Post {
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

        let envelope: CommandEnvelope = match serde_json::from_slice(&req.body) {
            Ok(env) => env,
            Err(err) => return HttpResponse::bad_request(err.to_string()),
        };

        // Enforce path-to-payload workspace/chat consistency.
        let (payload_ws, payload_chat) = match &envelope.command {
            CommandPayload::MessageSubmit(p) => (&p.patient_workspace_id, &p.chat_id),
            CommandPayload::WorkStop(p) => (&p.patient_workspace_id, &p.chat_id),
            CommandPayload::WorkResume(p) => (&p.patient_workspace_id, &p.chat_id),
        };
        if payload_ws != &workspace_id || payload_chat != &chat_id {
            return HttpResponse::bad_request(
                "command payload workspace/chat does not match route",
            );
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
        let tools: [&dyn ClinicalTool; 2] = [&search_tool, &read_tool];

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
            WalEntry::InsertSource { payload } => self.evidence.insert_source(payload),
            WalEntry::WithdrawSource {
                workspace_id,
                source_key,
            } => {
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

fn workspace_error_response(err: WorkspaceAccessError) -> HttpResponse {
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

fn wal_failure_response(err: &WalError) -> HttpResponse {
    HttpResponse::json_clinical(
        503,
        &serde_json::json!({
            "error": "persistence_unavailable",
            "detail": err.to_string(),
        }),
    )
}
