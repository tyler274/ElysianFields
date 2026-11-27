//! Typed `ClinicTransport` trait and in-process `LocalVerticalSliceTransport` bridging
//! directly to `ef-clinic-domain` and `ef-harness` without a second client store.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use ef_clinic_domain::auth::{
    AuthAuthority, AuthError, SignInInitiationResponse, SignInMode, VerificationTokenKind,
    VerifiedSessionPayload, WhoAmIPayload,
};
use ef_clinic_domain::chat::{
    ChatCommandError, ChatEngine, ChatStreamEvent, CommandAcceptance, CommandEnvelope,
    CommandPayload, DraftSaveOutcome, DraftStore, PersistedDraft, RetryBackoffMode,
};
use ef_clinic_domain::workspace::{
    CitedSourcePayload, MembershipTier, PatientRosterState, WorkspaceAccessError,
    WorkspaceEvidenceStore,
};
use ef_core::{
    CachePolicy, ChatId, PhysicianId, PracticeId, SessionId, SourceKey, SubpathError, WorkItemId,
    WorkspaceId,
};
use ef_harness::{
    AgentLoop, CancellationToken, ClinicalTool, HarnessError, ReadRecordsTool, SearchChartTool,
    StubProvider, TurnExecutionOutcome,
};
use thiserror::Error;

use crate::wire::{
    ChatSnapshotWire, ClinicEndpoint, PatientWorkspaceDetail, PutDraftRequest, SignInRequest,
    VerifySignInRequest, decode_sse_frame, encode_sse_frame,
};

/// Typed transport error preserving exact domain and serialization causes.
#[derive(Debug, Error)]
pub enum TransportError {
    #[error("authentication error: {0}")]
    Auth(#[from] AuthError),
    #[error("workspace access error: {0}")]
    Workspace(#[from] WorkspaceAccessError),
    #[error("chat command error: {0}")]
    Chat(#[from] ChatCommandError),
    #[error("harness execution error: {0}")]
    Harness(#[from] HarnessError),
    #[error("subpath validation error: {0}")]
    Subpath(#[from] SubpathError),
    #[error("wire serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("SSE wire frame error: {0}")]
    SseWire(#[from] crate::wire::SseWireError),
    #[error("transport backend unavailable: {message}")]
    Unavailable { message: String },
}

impl TransportError {
    #[must_use]
    pub const fn http_status_code(&self) -> u16 {
        match self {
            Self::Auth(err) => err.http_status_code(),
            Self::Workspace(err) => err.http_status_code(),
            Self::Chat(err) => err.http_status_code(),
            Self::Harness(_) | Self::Unavailable { .. } => 503,
            Self::Subpath(_) | Self::Serialization(_) | Self::SseWire(_) => 400,
        }
    }
}

/// Recorded clinical/auth response cache policy metadata observed on each transport call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportResponseMeta {
    pub endpoint_url: String,
    pub cache_policy: CachePolicy,
}

/// Typed client transport contract implemented by in-process local vertical-slice tests
/// and hosted HTTP/SSE adapters under `/elysianfields/api/...`.
pub trait ClinicTransport: Send + Sync {
    fn initiate_sign_in(
        &mut self,
        req: &SignInRequest,
        now_secs: u64,
    ) -> Result<SignInInitiationResponse, TransportError>;

    fn verify_sign_in(
        &mut self,
        req: &VerifySignInRequest,
        now_secs: u64,
    ) -> Result<VerifiedSessionPayload, TransportError>;

    fn whoami(
        &mut self,
        session: Option<&SessionId>,
        now_secs: u64,
    ) -> Result<WhoAmIPayload, TransportError>;

    fn sign_out(&mut self, session: &SessionId, now_secs: u64) -> Result<(), TransportError>;

    fn fetch_patient_roster(
        &mut self,
        session: &SessionId,
        query: Option<&str>,
        now_secs: u64,
    ) -> Result<PatientRosterState, TransportError>;

    fn fetch_workspace_detail(
        &mut self,
        session: &SessionId,
        workspace_id: &WorkspaceId,
        now_secs: u64,
    ) -> Result<PatientWorkspaceDetail, TransportError>;

    fn fetch_chat_snapshot(
        &mut self,
        session: &SessionId,
        workspace_id: &WorkspaceId,
        chat_id: &ChatId,
        now_secs: u64,
    ) -> Result<ChatSnapshotWire, TransportError>;

    fn fetch_cited_source(
        &mut self,
        session: &SessionId,
        workspace_id: &WorkspaceId,
        chat_id: &ChatId,
        source_key: &SourceKey,
        now_secs: u64,
    ) -> Result<CitedSourcePayload, TransportError>;

    fn submit_command(
        &mut self,
        session: &SessionId,
        envelope: &CommandEnvelope,
        now_secs: u64,
    ) -> Result<CommandAcceptance, TransportError>;

    fn poll_stream_events(
        &mut self,
        session: &SessionId,
        since_event_id: u64,
        now_secs: u64,
    ) -> Result<Vec<ChatStreamEvent>, TransportError>;

    fn get_draft(
        &mut self,
        session: &SessionId,
        workspace_id: &WorkspaceId,
        chat_target: &str,
        now_secs: u64,
    ) -> Result<PersistedDraft, TransportError>;

    fn put_draft(
        &mut self,
        session: &SessionId,
        workspace_id: &WorkspaceId,
        chat_target: &str,
        req: &PutDraftRequest,
        now_secs: u64,
    ) -> Result<DraftSaveOutcome, TransportError>;

    /// Returns metadata for the last transport request (verifying `/elysianfields/api/...` and `no-store`).
    fn last_response_meta(&self) -> Option<TransportResponseMeta>;
}

/// Authoritative server-side vertical-slice state shared across tabs and simulated client reloads.
///
/// The client NEVER duplicates this state; every read/write travels through [`ClinicTransport`].
pub struct VerticalSliceBackend {
    pub auth: AuthAuthority,
    pub evidence: WorkspaceEvidenceStore,
    pub chat: ChatEngine,
    pub drafts: DraftStore,
    pub agent_loop: AgentLoop,
    pub stub_provider: StubProvider,
    pub default_chats: BTreeMap<WorkspaceId, ChatId>,
    pub synopses: BTreeMap<WorkspaceId, String>,
    pub partial_roster_outage: bool,
    pub draft_transient_failure: Option<String>,
    pub observed_requests: Vec<TransportResponseMeta>,
}

impl VerticalSliceBackend {
    /// Constructs a clean authoritative backend configured for deterministic local verification.
    #[must_use]
    pub fn new(sign_in_mode: SignInMode, retry_mode: RetryBackoffMode, stub: StubProvider) -> Self {
        Self {
            auth: AuthAuthority::new(sign_in_mode),
            evidence: WorkspaceEvidenceStore::new(),
            chat: ChatEngine::new(retry_mode),
            drafts: DraftStore::new(),
            agent_loop: AgentLoop::new(),
            stub_provider: stub,
            default_chats: BTreeMap::new(),
            synopses: BTreeMap::new(),
            partial_roster_outage: false,
            draft_transient_failure: None,
            observed_requests: Vec::new(),
        }
    }

    /// Executes one worker attempt for the next due work item using `AgentLoop` + `StubProvider`.
    pub fn run_next_worker_attempt(
        &mut self,
        practice_id: &PracticeId,
        physician_id: &PhysicianId,
        worker_id: &str,
        cancel: &CancellationToken,
        now_secs: u64,
    ) -> Result<Option<(WorkItemId, TurnExecutionOutcome)>, TransportError> {
        let Some((work_item_id, _run_id)) = self.chat.claim_due_work(worker_id, now_secs) else {
            return Ok(None);
        };
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
        let outcome = self.agent_loop.run_turn_attempt(
            &mut self.chat,
            &work_item_id,
            &mut self.stub_provider,
            &tools,
            cancel,
            now_secs,
        )?;
        Ok(Some((work_item_id, outcome)))
    }

    fn record_meta(&mut self, endpoint: &ClinicEndpoint) -> Result<(), TransportError> {
        let url = endpoint.resolve_url(None)?;
        self.observed_requests.push(TransportResponseMeta {
            endpoint_url: url,
            cache_policy: endpoint.required_cache_policy(),
        });
        Ok(())
    }
}

/// In-process [`ClinicTransport`] backed by a shared [`VerticalSliceBackend`].
///
/// Validates wire serialization (`CommandEnvelope` JSON and SSE frames) on every call
/// so client code exercises the exact M2 wire contracts while remaining deterministic.
#[derive(Clone)]
pub struct LocalVerticalSliceTransport {
    backend: Arc<Mutex<VerticalSliceBackend>>,
}

impl LocalVerticalSliceTransport {
    #[must_use]
    pub fn new(backend: Arc<Mutex<VerticalSliceBackend>>) -> Self {
        Self { backend }
    }

    #[must_use]
    pub fn backend(&self) -> Arc<Mutex<VerticalSliceBackend>> {
        Arc::clone(&self.backend)
    }

    fn lock_backend(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, VerticalSliceBackend>, TransportError> {
        self.backend
            .lock()
            .map_err(|err| TransportError::Unavailable {
                message: format!("vertical slice backend lock poisoned: {err}"),
            })
    }
}

impl ClinicTransport for LocalVerticalSliceTransport {
    fn initiate_sign_in(
        &mut self,
        req: &SignInRequest,
        now_secs: u64,
    ) -> Result<SignInInitiationResponse, TransportError> {
        let raw_json = serde_json::to_string(req)?;
        let decoded: SignInRequest = serde_json::from_str(&raw_json)?;
        let mut guard = self.lock_backend()?;
        guard.record_meta(&ClinicEndpoint::AuthSignIn)?;
        Ok(guard.auth.initiate_sign_in(
            &decoded.email,
            VerificationTokenKind::SignInLink,
            now_secs,
        )?)
    }

    fn verify_sign_in(
        &mut self,
        req: &VerifySignInRequest,
        now_secs: u64,
    ) -> Result<VerifiedSessionPayload, TransportError> {
        let raw_json = serde_json::to_string(req)?;
        let decoded: VerifySignInRequest = serde_json::from_str(&raw_json)?;
        let mut guard = self.lock_backend()?;
        guard.record_meta(&ClinicEndpoint::AuthVerify)?;
        Ok(guard
            .auth
            .verify_token(&decoded.email, &decoded.token, now_secs, None)?)
    }

    fn whoami(
        &mut self,
        session: Option<&SessionId>,
        now_secs: u64,
    ) -> Result<WhoAmIPayload, TransportError> {
        let mut guard = self.lock_backend()?;
        guard.record_meta(&ClinicEndpoint::AuthWhoAmI)?;
        Ok(guard.auth.whoami(session, now_secs)?)
    }

    fn sign_out(&mut self, session: &SessionId, now_secs: u64) -> Result<(), TransportError> {
        let mut guard = self.lock_backend()?;
        guard.record_meta(&ClinicEndpoint::AuthSignOut)?;
        guard.auth.sign_out(session, now_secs);
        Ok(())
    }

    fn fetch_patient_roster(
        &mut self,
        session: &SessionId,
        query: Option<&str>,
        now_secs: u64,
    ) -> Result<PatientRosterState, TransportError> {
        let mut guard = self.lock_backend()?;
        let endpoint = match query {
            Some(q) => ClinicEndpoint::PatientRoster {
                query: q.to_owned(),
            },
            None => ClinicEndpoint::PatientWorkspaces,
        };
        guard.record_meta(&endpoint)?;
        let who = guard.auth.whoami(Some(session), now_secs)?;
        let partial = guard.partial_roster_outage;
        Ok(guard
            .evidence
            .query_patient_roster(&who.practice_id, &who.physician_id, query, partial))
    }

    fn fetch_workspace_detail(
        &mut self,
        session: &SessionId,
        workspace_id: &WorkspaceId,
        now_secs: u64,
    ) -> Result<PatientWorkspaceDetail, TransportError> {
        let mut guard = self.lock_backend()?;
        guard.record_meta(&ClinicEndpoint::PatientWorkspaceDetail {
            workspace_id: workspace_id.clone(),
        })?;
        let who = guard.auth.whoami(Some(session), now_secs)?;
        let membership =
            guard
                .evidence
                .membership_for(&who.practice_id, &who.physician_id, workspace_id)?;
        if membership == MembershipTier::Denied {
            return Err(TransportError::Workspace(WorkspaceAccessError::NotFound));
        }

        let roster =
            guard
                .evidence
                .query_patient_roster(&who.practice_id, &who.physician_id, None, false);
        let display_name = match roster {
            PatientRosterState::Loaded { patients, .. } => patients
                .into_iter()
                .find(|p| &p.patient_workspace_id == workspace_id)
                .map(|p| p.display_name)
                .ok_or(WorkspaceAccessError::NotFound)?,
            PatientRosterState::Empty { .. } => {
                return Err(TransportError::Workspace(WorkspaceAccessError::NotFound));
            }
            PatientRosterState::Failed { message } => {
                return Err(TransportError::Workspace(
                    WorkspaceAccessError::StoreUnavailable { message },
                ));
            }
        };

        let default_chat_id = guard
            .default_chats
            .get(workspace_id)
            .cloned()
            .unwrap_or_else(|| ChatId::new("chat-main").expect("valid static chat id"));
        let patient_synopsis = match membership {
            MembershipTier::Allowed => guard.synopses.get(workspace_id).cloned(),
            MembershipTier::ClinicDemographic | MembershipTier::Denied => None,
        };

        Ok(PatientWorkspaceDetail {
            patient_workspace_id: workspace_id.clone(),
            display_name,
            membership,
            time_zone: "UTC".to_owned(),
            default_chat_id,
            patient_synopsis,
        })
    }

    fn fetch_chat_snapshot(
        &mut self,
        session: &SessionId,
        workspace_id: &WorkspaceId,
        chat_id: &ChatId,
        now_secs: u64,
    ) -> Result<ChatSnapshotWire, TransportError> {
        let mut guard = self.lock_backend()?;
        guard.record_meta(&ClinicEndpoint::ChatSnapshot {
            workspace_id: workspace_id.clone(),
            chat_id: chat_id.clone(),
        })?;
        let who = guard.auth.whoami(Some(session), now_secs)?;
        guard.evidence.require_clinical_access(
            &who.practice_id,
            &who.physician_id,
            workspace_id,
        )?;

        let transcript: Vec<_> = guard
            .chat
            .transcript_snapshot()
            .into_iter()
            .filter(|ex| &ex.workspace_id == workspace_id && &ex.chat_id == chat_id)
            .cloned()
            .collect();

        Ok(ChatSnapshotWire {
            workspace_id: workspace_id.clone(),
            chat_id: chat_id.clone(),
            transcript,
            sources: BTreeMap::new(),
            queued: Vec::new(),
            archived: false,
        })
    }

    fn fetch_cited_source(
        &mut self,
        session: &SessionId,
        workspace_id: &WorkspaceId,
        chat_id: &ChatId,
        source_key: &SourceKey,
        now_secs: u64,
    ) -> Result<CitedSourcePayload, TransportError> {
        let mut guard = self.lock_backend()?;
        guard.record_meta(&ClinicEndpoint::CitedSource {
            workspace_id: workspace_id.clone(),
            chat_id: chat_id.clone(),
            source_key: source_key.clone(),
        })?;
        let who = guard.auth.whoami(Some(session), now_secs)?;
        Ok(guard.evidence.fetch_cited_source(
            &who.practice_id,
            &who.physician_id,
            workspace_id,
            source_key,
        )?)
    }

    fn submit_command(
        &mut self,
        session: &SessionId,
        envelope: &CommandEnvelope,
        now_secs: u64,
    ) -> Result<CommandAcceptance, TransportError> {
        // Exercise exact JSON schema serialization on every command submission.
        let wire_json = serde_json::to_string(envelope)?;
        let decoded: CommandEnvelope = serde_json::from_str(&wire_json)?;

        let workspace_id = match &decoded.command {
            CommandPayload::MessageSubmit(p) => p.patient_workspace_id.clone(),
            CommandPayload::WorkStop(p) => p.patient_workspace_id.clone(),
            CommandPayload::WorkResume(p) => p.patient_workspace_id.clone(),
        };

        let mut guard = self.lock_backend()?;
        guard.record_meta(&ClinicEndpoint::Commands)?;
        let who = guard.auth.whoami(Some(session), now_secs)?;
        guard.evidence.require_clinical_access(
            &who.practice_id,
            &who.physician_id,
            &workspace_id,
        )?;

        Ok(guard.chat.execute_command(decoded, now_secs)?)
    }

    fn poll_stream_events(
        &mut self,
        session: &SessionId,
        since_event_id: u64,
        now_secs: u64,
    ) -> Result<Vec<ChatStreamEvent>, TransportError> {
        let mut guard = self.lock_backend()?;
        guard.record_meta(&ClinicEndpoint::WorkspaceEvents { since_event_id })?;
        let _who = guard.auth.whoami(Some(session), now_secs)?;

        let mut decoded_events = Vec::new();
        for ev in guard.chat.events() {
            let ev_id = stream_event_id(ev);
            if ev_id > since_event_id {
                let frame = encode_sse_frame(ev)?;
                decoded_events.push(decode_sse_frame(&frame)?);
            }
        }
        Ok(decoded_events)
    }

    fn get_draft(
        &mut self,
        session: &SessionId,
        workspace_id: &WorkspaceId,
        chat_target: &str,
        now_secs: u64,
    ) -> Result<PersistedDraft, TransportError> {
        let mut guard = self.lock_backend()?;
        guard.record_meta(&ClinicEndpoint::ComposerDraft {
            workspace_id: workspace_id.clone(),
            chat_target: chat_target.to_owned(),
        })?;
        let who = guard.auth.whoami(Some(session), now_secs)?;
        guard.evidence.require_clinical_access(
            &who.practice_id,
            &who.physician_id,
            workspace_id,
        )?;
        Ok(guard.drafts.get(workspace_id, chat_target))
    }

    fn put_draft(
        &mut self,
        session: &SessionId,
        workspace_id: &WorkspaceId,
        chat_target: &str,
        req: &PutDraftRequest,
        now_secs: u64,
    ) -> Result<DraftSaveOutcome, TransportError> {
        let wire_json = serde_json::to_string(req)?;
        let decoded: PutDraftRequest = serde_json::from_str(&wire_json)?;
        let mut guard = self.lock_backend()?;
        guard.record_meta(&ClinicEndpoint::ComposerDraft {
            workspace_id: workspace_id.clone(),
            chat_target: chat_target.to_owned(),
        })?;
        let who = guard.auth.whoami(Some(session), now_secs)?;
        guard.evidence.require_clinical_access(
            &who.practice_id,
            &who.physician_id,
            workspace_id,
        )?;

        if let Some(msg) = &guard.draft_transient_failure {
            return Ok(DraftSaveOutcome::TransientFailure {
                message: msg.clone(),
            });
        }
        Ok(guard.drafts.put(
            workspace_id,
            chat_target,
            decoded.text,
            decoded.expected_revision,
        ))
    }

    fn last_response_meta(&self) -> Option<TransportResponseMeta> {
        self.backend
            .lock()
            .ok()
            .and_then(|guard| guard.observed_requests.last().cloned())
    }
}

#[must_use]
pub const fn stream_event_id(event: &ChatStreamEvent) -> u64 {
    match event {
        ChatStreamEvent::DeltaChunk { event_id, .. }
        | ChatStreamEvent::StepRecorded { event_id, .. }
        | ChatStreamEvent::Completed { event_id, .. }
        | ChatStreamEvent::Cancelled { event_id, .. }
        | ChatStreamEvent::CouldntFinish { event_id, .. }
        | ChatStreamEvent::ResyncRequired { event_id, .. } => *event_id,
        ChatStreamEvent::KeepAlive => 0,
    }
}
