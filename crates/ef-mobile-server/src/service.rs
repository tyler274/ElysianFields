//! Headless Curie Mobile HTTP service mounted strictly under `/elysianfields/mobile-api/...`.
//!
//! Enforces migration + restore-replay `/readyz` gating, standalone patient auth,
//! per-record savepoint ingestion, account-fenced idempotent health uploads,
//! portable export, and permanent account deletion writing to an independent
//! create-only [`PrivacyReceiptJournal`].

use std::collections::BTreeMap;
use std::fmt::Write as _;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use ef_connected_care::{
    AccountIncarnation as BridgeAccountIncarnation, BridgeError, ConnectedCareBridge, ConnectionId,
    DateOfBirth, DisconnectActor, EnvelopeItem, InviteCredential,
    MobileAccountId as BridgeMobileAccountId, PairKey, Part2WrittenConsent, TransferSubmission,
};
use ef_core::{IdempotencyKey, PatientId, PracticeId, Subpath};
use ef_mobile_domain::{
    AccountFencedHealthBatch, DevicePushRegistration, ForMyDoctorWorkspace, InMemoryReceiptJournal,
    IncomingBatchItem, MobileAccountId, MobileAuthAuthority, MobileAuthError,
    MobileHealthIngestStore, MobilePrivacyTarget, MobileSessionId, PatientRecordStore,
    PortablePatientExport, PracticeSendReceiver, PrivacyReceipt, PrivacyReceiptJournal,
    PushTokenRegistry, RestrictionJobError, RestrictionJobReport, RestrictionOperation,
    apply_privacy_receipt, run_restore_replay_gate,
};
use serde::Deserialize;
use thiserror::Error;

use crate::http::{MobileHttpMethod, MobileHttpRequest, MobileHttpResponse};
#[cfg(not(target_arch = "wasm32"))]
use crate::persistence::{DurableMobileStore, PgMobileStore};
use crate::persistence::{InMemoryMobileWal, MobileWalEntry, MobileWalError, MobileWalSink};

pub const MOBILE_API_PREFIX: &str = "/elysianfields/mobile-api/";
pub const EXPECTED_MIGRATION_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum MobileServiceError {
    #[error("mobile WAL error: {0}")]
    Wal(#[from] MobileWalError),
}

#[derive(Debug, Deserialize)]
struct RequestLinkBody {
    email: String,
}

#[derive(Debug, Deserialize)]
struct VerifyBody {
    email: String,
    token: String,
}

#[derive(Debug, Deserialize)]
struct SignOutBody {
    #[serde(default)]
    owed_sessions: Vec<MobileSessionId>,
}

#[derive(Debug, Deserialize)]
struct IngestRecordsBody {
    items: Vec<IncomingBatchItem>,
}

#[derive(Debug, Deserialize)]
struct DeleteAccountBody {
    event_id: String,
    occurred_at: String,
}

#[derive(Debug, Deserialize)]
struct ConnectedCareLookupBody {
    credential: InviteCredential,
}

#[derive(Debug, Deserialize)]
struct ConnectedCareVerifyDobBody {
    credential: InviteCredential,
    year: u16,
    month: u8,
    day: u8,
}

#[derive(Debug, Deserialize)]
struct ConnectedCareShareBody {
    credential: InviteCredential,
}

#[derive(Debug, Deserialize)]
struct ConnectedCareTransferBody {
    idempotency_key: IdempotencyKey,
    connection_id: ConnectionId,
    items: Vec<EnvelopeItem>,
    #[serde(default)]
    part2_consent: Option<Part2WrittenConsent>,
}

#[derive(Debug, Deserialize)]
struct ConnectedCareDisconnectBody {
    connection_id: ConnectionId,
}

/// Zero-PHI atomic Prometheus counters/gauges for [`MobileService`].
#[derive(Debug)]
struct MobileMetrics {
    requests_2xx: AtomicU64,
    requests_4xx: AtomicU64,
    requests_5xx: AtomicU64,
    initial_clock_secs: u64,
    #[cfg(not(target_arch = "wasm32"))]
    started_at: Instant,
}

impl MobileMetrics {
    fn new(initial_clock_secs: u64) -> Self {
        Self {
            requests_2xx: AtomicU64::new(0),
            requests_4xx: AtomicU64::new(0),
            requests_5xx: AtomicU64::new(0),
            initial_clock_secs,
            #[cfg(not(target_arch = "wasm32"))]
            started_at: Instant::now(),
        }
    }

    fn record_status(&self, status: u16) {
        match status {
            200..=299 => {
                self.requests_2xx.fetch_add(1, Ordering::Relaxed);
            }
            400..=499 => {
                self.requests_4xx.fetch_add(1, Ordering::Relaxed);
            }
            500..=599 => {
                self.requests_5xx.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
    }

    fn uptime_seconds(&self, current_clock_secs: u64) -> u64 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (current_clock_secs, self.initial_clock_secs);
            self.started_at.elapsed().as_secs()
        }
        #[cfg(target_arch = "wasm32")]
        {
            current_clock_secs.saturating_sub(self.initial_clock_secs)
        }
    }

    fn render_prometheus(&self, wal_seq: u64, uptime_secs: u64, db_ready: bool) -> String {
        let r2 = self.requests_2xx.load(Ordering::Relaxed);
        let r4 = self.requests_4xx.load(Ordering::Relaxed);
        let r5 = self.requests_5xx.load(Ordering::Relaxed);
        let ready_val = u8::from(db_ready);
        let mut out = String::with_capacity(512);
        let _ = writeln!(
            out,
            "# HELP ef_mobile_http_requests_total Total HTTP requests handled by ef-mobile-server by status class."
        );
        let _ = writeln!(out, "# TYPE ef_mobile_http_requests_total counter");
        let _ = writeln!(
            out,
            "ef_mobile_http_requests_total{{status_class=\"2xx\"}} {r2}"
        );
        let _ = writeln!(
            out,
            "ef_mobile_http_requests_total{{status_class=\"4xx\"}} {r4}"
        );
        let _ = writeln!(
            out,
            "ef_mobile_http_requests_total{{status_class=\"5xx\"}} {r5}"
        );
        let _ = writeln!(
            out,
            "# HELP ef_mobile_wal_seq Highest durable WAL sequence number observed by ef-mobile-server."
        );
        let _ = writeln!(out, "# TYPE ef_mobile_wal_seq gauge");
        let _ = writeln!(out, "ef_mobile_wal_seq {wal_seq}");
        let _ = writeln!(
            out,
            "# HELP ef_mobile_uptime_seconds Seconds elapsed since ef-mobile-server started."
        );
        let _ = writeln!(out, "# TYPE ef_mobile_uptime_seconds gauge");
        let _ = writeln!(out, "ef_mobile_uptime_seconds {uptime_secs}");
        let _ = writeln!(
            out,
            "# HELP ef_mobile_db_ready Backing store liveness indicator (1 = ready, 0 = unavailable)."
        );
        let _ = writeln!(out, "# TYPE ef_mobile_db_ready gauge");
        let _ = writeln!(out, "ef_mobile_db_ready {ready_val}");
        out
    }
}

/// Authoritative headless Curie Mobile service strictly isolated from Curie Clinic.
pub struct MobileService {
    pub auth: MobileAuthAuthority,
    pub records: PatientRecordStore,
    pub health: MobileHealthIngestStore,
    pub push_tokens: PushTokenRegistry,
    pub doctor_workspaces: BTreeMap<MobileAccountId, ForMyDoctorWorkspace>,
    pub bridge_staging: PracticeSendReceiver,
    pub connected_care: ConnectedCareBridge,
    migration_version: u32,
    restore_replay_verified: bool,
    clock_secs: u64,
    wal: Box<dyn MobileWalSink>,
    journal: Box<dyn PrivacyReceiptJournal>,
    metrics: MobileMetrics,
}

impl MobileService {
    /// Creates a mobile service backed by a caller-supplied [`MobileWalSink`] (e.g., health-failure testing).
    #[must_use]
    pub fn new_with_wal(
        wal: Box<dyn MobileWalSink>,
        journal: Box<dyn PrivacyReceiptJournal>,
        initial_clock_secs: u64,
        require_restore_replay: bool,
    ) -> Self {
        Self {
            auth: MobileAuthAuthority::new(),
            records: PatientRecordStore::new(),
            health: MobileHealthIngestStore::new(),
            push_tokens: PushTokenRegistry::new(),
            doctor_workspaces: BTreeMap::new(),
            bridge_staging: PracticeSendReceiver::new(),
            connected_care: ConnectedCareBridge::new(),
            migration_version: EXPECTED_MIGRATION_VERSION,
            restore_replay_verified: !require_restore_replay,
            clock_secs: initial_clock_secs,
            wal,
            journal,
            metrics: MobileMetrics::new(initial_clock_secs),
        }
    }

    #[must_use]
    pub fn new_in_memory(initial_clock_secs: u64) -> Self {
        Self::new_with_wal(
            Box::new(InMemoryMobileWal::new()),
            Box::new(InMemoryReceiptJournal::new()),
            initial_clock_secs,
            false,
        )
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_durable(
        data_dir: &Path,
        journal: Box<dyn PrivacyReceiptJournal>,
        initial_clock_secs: u64,
        require_restore_replay: bool,
    ) -> Result<Self, MobileServiceError> {
        let (store, entries) = DurableMobileStore::open(data_dir)?;
        let mut svc = Self::new_with_wal(
            Box::new(store),
            journal,
            initial_clock_secs,
            require_restore_replay,
        );
        for entry in entries {
            svc.replay_wal_entry(entry);
        }
        Ok(svc)
    }

    /// Opens a PostgreSQL-backed durable [`MobileService`] via `ef-pgwire`, applying
    /// `0001_m3_mobile.sql` idempotently and replaying all persisted mutations.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_postgres(
        config: &ef_pgwire::PgConnectionConfig,
        journal: Box<dyn PrivacyReceiptJournal>,
        initial_clock_secs: u64,
        require_restore_replay: bool,
    ) -> Result<Self, MobileServiceError> {
        let (store, entries) = PgMobileStore::open(config)?;
        let mut svc = Self::new_with_wal(
            Box::new(store),
            journal,
            initial_clock_secs,
            require_restore_replay,
        );
        for entry in entries {
            svc.replay_wal_entry(entry);
        }
        Ok(svc)
    }

    /// Executes the mandatory independent privacy-receipt journal replay gate before opening traffic.
    pub fn execute_restore_replay_gate(
        &mut self,
        max_receipt_budget: usize,
    ) -> Result<RestrictionJobReport, RestrictionJobError> {
        let mut target = MobilePrivacyTarget {
            auth: &mut self.auth,
            records: &mut self.records,
            health: &mut self.health,
            push_tokens: &mut self.push_tokens,
            doctor_workspaces: &mut self.doctor_workspaces,
            bridge_staging: &mut self.bridge_staging,
        };
        let report =
            run_restore_replay_gate(self.journal.as_ref(), &mut target, max_receipt_budget)?;
        self.restore_replay_verified = true;
        Ok(report)
    }

    /// Replays a single persisted [`MobileWalEntry`] through live domain owners.
    pub fn replay_wal_entry(&mut self, entry: MobileWalEntry) {
        match entry {
            MobileWalEntry::IssueSignInLink {
                raw_email,
                now_secs,
            } => {
                let _ = self.auth.issue_sign_in_link(&raw_email, now_secs);
            }
            MobileWalEntry::VerifySignIn {
                raw_email,
                raw_token,
                now_secs,
            } => {
                let _ = self.auth.verify_sign_in(&raw_email, &raw_token, now_secs);
            }
            MobileWalEntry::RevokeSession { session_id } => {
                self.auth.revoke_session(&session_id);
            }
            MobileWalEntry::IngestRecords { items } => {
                let _ = self.records.ingest_batch_with_savepoints(items);
            }
            MobileWalEntry::IngestHealthBatch { account_id, batch } => {
                let _ = self.health.ingest_batch(&account_id, batch);
            }
            MobileWalEntry::RegisterPushToken { registration } => {
                self.push_tokens.register_or_rotate(registration);
            }
            MobileWalEntry::RevokePushDevice { device_id } => {
                let _ = self.push_tokens.revoke_device(&device_id);
            }
            MobileWalEntry::DeleteAccount {
                account_id,
                incarnation,
            } => {
                self.auth.delete_account(&account_id, incarnation);
                let _ = self.records.purge_account_records(&account_id, incarnation);
                let _ = self.health.purge_account_samples(&account_id);
                let _ = self.push_tokens.revoke_all_for_account(&account_id);
                if let Some(ws) = self.doctor_workspaces.get_mut(&account_id) {
                    ws.clear_all_to_send();
                    ws.terminate_all_connections();
                }
                let _ = self.bridge_staging.scrub_account_staging(&account_id);
            }
        }
    }

    /// Routes an HTTP/1.1 request strictly under `/elysianfields/mobile-api/...`.
    pub fn handle_request(&mut self, req: &MobileHttpRequest) -> MobileHttpResponse {
        let request_id = ef_core::HttpRequestId::parse_or_generate(
            req.header(ef_core::REQUEST_ID_HEADER),
            "mobile",
        );
        let mut resp = self.dispatch_request(req);
        resp.set_header(ef_core::REQUEST_ID_HEADER, request_id.as_str());
        self.metrics.record_status(resp.status);
        resp
    }

    fn dispatch_request(&mut self, req: &MobileHttpRequest) -> MobileHttpResponse {
        let (pathname, query) = req.split_target();
        if !Subpath::is_within_mount(pathname) {
            return MobileHttpResponse::not_found();
        }
        let Some(api_rel) = pathname.strip_prefix(MOBILE_API_PREFIX) else {
            return MobileHttpResponse::not_found();
        };
        let segments: Vec<&str> = api_rel.split('/').filter(|s| !s.is_empty()).collect();

        if segments.as_slice() == ["readyz"] || segments.as_slice() == ["healthz"] {
            return self.handle_readyz(req);
        }
        if segments.as_slice() == ["metrics"] {
            return self.handle_metrics(req);
        }

        if !self.restore_replay_verified || self.migration_version != EXPECTED_MIGRATION_VERSION {
            return MobileHttpResponse::json(
                503,
                &serde_json::json!({
                    "ready": false,
                    "reason": "restore_replay_or_migration_gate_unverified",
                }),
            );
        }

        match (req.method, segments.as_slice()) {
            (MobileHttpMethod::Post, ["auth", "request-link"]) => self.handle_request_link(req),
            (MobileHttpMethod::Post, ["auth", "verify"]) => self.handle_verify(req),
            (MobileHttpMethod::Post, ["auth", "sign-out"]) => self.handle_sign_out(req),
            (MobileHttpMethod::Get, ["auth", "session"]) => self.handle_session(req),
            (MobileHttpMethod::Post, ["records", "ingest"]) => self.handle_ingest_records(req),
            (MobileHttpMethod::Get, ["records"]) => self.handle_list_records(req),
            (MobileHttpMethod::Post, ["health", "upload"]) => self.handle_health_upload(req),
            (MobileHttpMethod::Get, ["health", "trends"]) => self.handle_health_trends(req),
            (MobileHttpMethod::Post, ["push", "register"]) => self.handle_push_register(req),
            (MobileHttpMethod::Get, ["export"]) => self.handle_export(req),
            (MobileHttpMethod::Post, ["account", "delete"]) => self.handle_account_delete(req),
            (MobileHttpMethod::Post, ["connected-care", "lookup"]) => {
                self.handle_connected_care_lookup(req)
            }
            (MobileHttpMethod::Post, ["connected-care", "verify-dob"]) => {
                self.handle_connected_care_verify_dob(req)
            }
            (MobileHttpMethod::Post, ["connected-care", "share"]) => {
                self.handle_connected_care_share(req)
            }
            (MobileHttpMethod::Post, ["connected-care", "transfer"]) => {
                self.handle_connected_care_transfer(req)
            }
            (MobileHttpMethod::Post, ["connected-care", "disconnect"]) => {
                self.handle_connected_care_disconnect(req)
            }
            (MobileHttpMethod::Get, ["connected-care", "status"]) => {
                self.handle_connected_care_status(req, query)
            }
            _ => MobileHttpResponse::not_found(),
        }
    }

    fn handle_metrics(&mut self, req: &MobileHttpRequest) -> MobileHttpResponse {
        if req.method != MobileHttpMethod::Get {
            return MobileHttpResponse::method_not_allowed();
        }
        let db_ready = self.wal.check_health().is_ok();
        let wal_seq = self.wal.wal_seq();
        let uptime_secs = self.metrics.uptime_seconds(self.clock_secs);
        MobileHttpResponse::prometheus_metrics(self.metrics.render_prometheus(
            wal_seq,
            uptime_secs,
            db_ready,
        ))
    }

    fn handle_readyz(&mut self, req: &MobileHttpRequest) -> MobileHttpResponse {
        if req.method != MobileHttpMethod::Get {
            return MobileHttpResponse::method_not_allowed();
        }
        let db_health = self.wal.check_health();
        if self.migration_version == EXPECTED_MIGRATION_VERSION
            && self.restore_replay_verified
            && db_health.is_ok()
        {
            MobileHttpResponse::json(
                200,
                &serde_json::json!({
                    "ready": true,
                    "migration_version": self.migration_version,
                    "restore_replay_verified": true,
                    "db_healthy": true,
                }),
            )
        } else {
            MobileHttpResponse::json(
                503,
                &serde_json::json!({
                    "ready": false,
                    "migration_version": self.migration_version,
                    "restore_replay_verified": self.restore_replay_verified,
                    "db_healthy": db_health.is_ok(),
                    "db_error": db_health.err(),
                }),
            )
        }
    }

    fn handle_request_link(&mut self, req: &MobileHttpRequest) -> MobileHttpResponse {
        let body: RequestLinkBody = match serde_json::from_slice(&req.body) {
            Ok(b) => b,
            Err(err) => return MobileHttpResponse::bad_request(err.to_string()),
        };
        let now_secs = self.clock_secs;
        match self.auth.issue_sign_in_link(&body.email, now_secs) {
            Ok(token) => {
                if let Err(err) = self.wal.append(&MobileWalEntry::IssueSignInLink {
                    raw_email: body.email,
                    now_secs,
                }) {
                    return MobileHttpResponse::json(
                        503,
                        &serde_json::json!({ "error": "wal_error", "detail": err.to_string() }),
                    );
                }
                MobileHttpResponse::json(200, &serde_json::json!({ "token": token }))
            }
            Err(err) => auth_err_resp(err),
        }
    }

    fn handle_verify(&mut self, req: &MobileHttpRequest) -> MobileHttpResponse {
        let body: VerifyBody = match serde_json::from_slice(&req.body) {
            Ok(b) => b,
            Err(err) => return MobileHttpResponse::bad_request(err.to_string()),
        };
        let now_secs = self.clock_secs;
        match self.auth.verify_sign_in(&body.email, &body.token, now_secs) {
            Ok(verified) => {
                if let Err(err) = self.wal.append(&MobileWalEntry::VerifySignIn {
                    raw_email: body.email,
                    raw_token: body.token,
                    now_secs,
                }) {
                    return MobileHttpResponse::json(
                        503,
                        &serde_json::json!({ "error": "wal_error", "detail": err.to_string() }),
                    );
                }
                MobileHttpResponse::json(200, &verified)
            }
            Err(err) => auth_err_resp(err),
        }
    }

    fn handle_sign_out(&mut self, req: &MobileHttpRequest) -> MobileHttpResponse {
        let mut sessions = Vec::new();
        if let Some(raw) = req.bearer_token()
            && let Ok(sess) = MobileSessionId::new(raw)
        {
            sessions.push(sess);
        }
        if !req.body.is_empty()
            && let Ok(parsed) = serde_json::from_slice::<SignOutBody>(&req.body)
        {
            sessions.extend(parsed.owed_sessions);
        }
        let revoked_count = sessions.len();
        for session_id in sessions {
            self.auth.revoke_session(&session_id);
            if let Err(err) = self
                .wal
                .append(&MobileWalEntry::RevokeSession { session_id })
            {
                return MobileHttpResponse::json(
                    503,
                    &serde_json::json!({ "error": "wal_error", "detail": err.to_string() }),
                );
            }
        }
        MobileHttpResponse::json(200, &serde_json::json!({ "revoked": revoked_count }))
    }

    fn authenticate(&self, req: &MobileHttpRequest) -> Result<MobileAccountId, MobileHttpResponse> {
        let Some(raw) = req.bearer_token() else {
            return Err(auth_err_resp(MobileAuthError::SessionEnded));
        };
        let session_id = MobileSessionId::new(raw).map_err(auth_err_resp)?;
        let record = self
            .auth
            .validate_session(&session_id, self.clock_secs)
            .map_err(auth_err_resp)?;
        Ok(record.account_id)
    }

    fn handle_session(&self, req: &MobileHttpRequest) -> MobileHttpResponse {
        match self.authenticate(req) {
            Ok(account_id) => {
                let inc = self
                    .auth
                    .account(&account_id)
                    .map_or(1, |a| a.incarnation.as_u64());
                MobileHttpResponse::json(
                    200,
                    &serde_json::json!({
                        "account_id": account_id,
                        "incarnation": inc,
                    }),
                )
            }
            Err(resp) => resp,
        }
    }

    fn handle_ingest_records(&mut self, req: &MobileHttpRequest) -> MobileHttpResponse {
        if let Err(resp) = self.authenticate(req) {
            return resp;
        }
        let body: IngestRecordsBody = match serde_json::from_slice(&req.body) {
            Ok(b) => b,
            Err(err) => return MobileHttpResponse::bad_request(err.to_string()),
        };
        let report = self
            .records
            .ingest_batch_with_savepoints(body.items.clone());
        if let Err(err) = self
            .wal
            .append(&MobileWalEntry::IngestRecords { items: body.items })
        {
            return MobileHttpResponse::json(
                503,
                &serde_json::json!({ "error": "wal_error", "detail": err.to_string() }),
            );
        }
        MobileHttpResponse::json(200, &report)
    }

    fn handle_list_records(&self, req: &MobileHttpRequest) -> MobileHttpResponse {
        let account_id = match self.authenticate(req) {
            Ok(id) => id,
            Err(resp) => return resp,
        };
        let records = self.records.records_for_account(&account_id);
        MobileHttpResponse::json(200, &serde_json::json!({ "records": records }))
    }

    fn handle_health_upload(&mut self, req: &MobileHttpRequest) -> MobileHttpResponse {
        let account_id = match self.authenticate(req) {
            Ok(id) => id,
            Err(resp) => return resp,
        };
        let batch: AccountFencedHealthBatch = match serde_json::from_slice(&req.body) {
            Ok(b) => b,
            Err(err) => return MobileHttpResponse::bad_request(err.to_string()),
        };
        match self.health.ingest_batch(&account_id, batch.clone()) {
            Ok(receipt) => {
                if !receipt.deduplicated
                    && let Err(err) = self
                        .wal
                        .append(&MobileWalEntry::IngestHealthBatch { account_id, batch })
                {
                    return MobileHttpResponse::json(
                        503,
                        &serde_json::json!({ "error": "wal_error", "detail": err.to_string() }),
                    );
                }
                MobileHttpResponse::json(200, &receipt)
            }
            Err(err) => MobileHttpResponse::json(
                403,
                &serde_json::json!({ "error": "health_sync_error", "detail": err.to_string() }),
            ),
        }
    }

    fn handle_health_trends(&self, req: &MobileHttpRequest) -> MobileHttpResponse {
        let account_id = match self.authenticate(req) {
            Ok(id) => id,
            Err(resp) => return resp,
        };
        let samples = self.health.samples_for_account(&account_id);
        MobileHttpResponse::json(200, &serde_json::json!({ "samples": samples }))
    }

    fn handle_push_register(&mut self, req: &MobileHttpRequest) -> MobileHttpResponse {
        let account_id = match self.authenticate(req) {
            Ok(id) => id,
            Err(resp) => return resp,
        };
        let mut reg: DevicePushRegistration = match serde_json::from_slice(&req.body) {
            Ok(r) => r,
            Err(err) => return MobileHttpResponse::bad_request(err.to_string()),
        };
        reg.account_id = account_id;
        self.push_tokens.register_or_rotate(reg.clone());
        if let Err(err) = self
            .wal
            .append(&MobileWalEntry::RegisterPushToken { registration: reg })
        {
            return MobileHttpResponse::json(
                503,
                &serde_json::json!({ "error": "wal_error", "detail": err.to_string() }),
            );
        }
        MobileHttpResponse::json(200, &serde_json::json!({ "registered": true }))
    }

    fn handle_export(&self, req: &MobileHttpRequest) -> MobileHttpResponse {
        let account_id = match self.authenticate(req) {
            Ok(id) => id,
            Err(resp) => return resp,
        };
        let export = PortablePatientExport {
            account_id: account_id.clone(),
            exported_at: format!("epoch-{}", self.clock_secs),
            records: self
                .records
                .records_for_account(&account_id)
                .into_iter()
                .cloned()
                .collect(),
            health_samples: self.health.samples_for_account(&account_id).to_vec(),
            confirmed_sends: self
                .doctor_workspaces
                .get(&account_id)
                .map(|ws| ws.sends().to_vec())
                .unwrap_or_default(),
        };
        MobileHttpResponse::json(200, &export)
    }

    fn handle_account_delete(&mut self, req: &MobileHttpRequest) -> MobileHttpResponse {
        let account_id = match self.authenticate(req) {
            Ok(id) => id,
            Err(resp) => return resp,
        };
        let body: DeleteAccountBody = match serde_json::from_slice(&req.body) {
            Ok(b) => b,
            Err(err) => return MobileHttpResponse::bad_request(err.to_string()),
        };
        let Some(acct) = self.auth.account(&account_id).cloned() else {
            return auth_err_resp(MobileAuthError::AccountDeleted);
        };

        // 1. Write metadata-only immutable receipt to independent create-only journal FIRST.
        let receipt = PrivacyReceipt {
            schema_version: 1,
            event_id: body.event_id,
            account_id: account_id.clone(),
            account_incarnation: acct.incarnation,
            operation: RestrictionOperation::AccountDelete,
            scope_id: String::new(),
            scope_version: 0,
            occurred_at: body.occurred_at,
        };
        if let Err(err) = self.journal.put_if_absent(&receipt) {
            return MobileHttpResponse::json(
                503,
                &serde_json::json!({ "error": "journal_unavailable", "detail": err.to_string() }),
            );
        }

        // 2. Apply receipt to primary DB state and persist WAL.
        let mut target = MobilePrivacyTarget {
            auth: &mut self.auth,
            records: &mut self.records,
            health: &mut self.health,
            push_tokens: &mut self.push_tokens,
            doctor_workspaces: &mut self.doctor_workspaces,
            bridge_staging: &mut self.bridge_staging,
        };
        let scrubbed = apply_privacy_receipt(&receipt, &mut target);
        if let Ok(bridge_acct) = BridgeMobileAccountId::new(account_id.as_str())
            && let Ok(bridge_inc) = BridgeAccountIncarnation::new(acct.incarnation.as_u64())
        {
            let _ = self.connected_care.terminate_account_links(
                &bridge_acct,
                bridge_inc,
                receipt.occurred_at.clone(),
            );
        }
        if let Err(err) = self.wal.append(&MobileWalEntry::DeleteAccount {
            account_id,
            incarnation: acct.incarnation,
        }) {
            return MobileHttpResponse::json(
                503,
                &serde_json::json!({ "error": "wal_error", "detail": err.to_string() }),
            );
        }

        MobileHttpResponse::json(
            200,
            &serde_json::json!({
                "deleted": true,
                "receipt_event_id": receipt.event_id,
                "scrubbed_staging_records": scrubbed,
            }),
        )
    }

    fn handle_connected_care_lookup(&self, req: &MobileHttpRequest) -> MobileHttpResponse {
        if let Err(resp) = self.authenticate(req) {
            return resp;
        }
        let body: ConnectedCareLookupBody = match serde_json::from_slice(&req.body) {
            Ok(b) => b,
            Err(err) => return MobileHttpResponse::bad_request(err.to_string()),
        };
        match self.connected_care.lookup_challenge(&body.credential) {
            Ok(challenge) => MobileHttpResponse::json(200, &challenge),
            Err(err) => bridge_err_resp(err),
        }
    }

    fn handle_connected_care_verify_dob(&mut self, req: &MobileHttpRequest) -> MobileHttpResponse {
        let account_id = match self.authenticate(req) {
            Ok(id) => id,
            Err(resp) => return resp,
        };
        let body: ConnectedCareVerifyDobBody = match serde_json::from_slice(&req.body) {
            Ok(b) => b,
            Err(err) => return MobileHttpResponse::bad_request(err.to_string()),
        };
        let dob = match DateOfBirth::new(body.year, body.month, body.day) {
            Ok(d) => d,
            Err(err) => return bridge_err_resp(err),
        };
        let bridge_acct = match BridgeMobileAccountId::new(account_id.as_str()) {
            Ok(id) => id,
            Err(err) => return bridge_err_resp(BridgeError::InvalidId(err)),
        };
        let now_str = format!("epoch-{}", self.clock_secs);
        match self
            .connected_care
            .verify_dob(&body.credential, &bridge_acct, dob, now_str)
        {
            Ok(outcome) => MobileHttpResponse::json(200, &outcome),
            Err(err) => bridge_err_resp(err),
        }
    }

    fn handle_connected_care_share(&mut self, req: &MobileHttpRequest) -> MobileHttpResponse {
        let account_id = match self.authenticate(req) {
            Ok(id) => id,
            Err(resp) => return resp,
        };
        let body: ConnectedCareShareBody = match serde_json::from_slice(&req.body) {
            Ok(b) => b,
            Err(err) => return MobileHttpResponse::bad_request(err.to_string()),
        };
        let bridge_acct = match BridgeMobileAccountId::new(account_id.as_str()) {
            Ok(id) => id,
            Err(err) => return bridge_err_resp(BridgeError::InvalidId(err)),
        };
        let now_str = format!("epoch-{}", self.clock_secs);
        match self
            .connected_care
            .confirm_share(&body.credential, &bridge_acct, now_str)
        {
            Ok(standing) => MobileHttpResponse::json(200, &standing),
            Err(err) => bridge_err_resp(err),
        }
    }

    fn handle_connected_care_transfer(&mut self, req: &MobileHttpRequest) -> MobileHttpResponse {
        let account_id = match self.authenticate(req) {
            Ok(id) => id,
            Err(resp) => return resp,
        };
        let body: ConnectedCareTransferBody = match serde_json::from_slice(&req.body) {
            Ok(b) => b,
            Err(err) => return MobileHttpResponse::bad_request(err.to_string()),
        };
        let bridge_acct = match BridgeMobileAccountId::new(account_id.as_str()) {
            Ok(id) => id,
            Err(err) => return bridge_err_resp(BridgeError::InvalidId(err)),
        };
        let submission = TransferSubmission {
            idempotency_key: body.idempotency_key,
            connection_id: body.connection_id,
            sender_account: bridge_acct,
            sent_at: format!("epoch-{}", self.clock_secs),
            items: body.items,
            part2_consent: body.part2_consent,
        };
        match self.connected_care.submit_transfer(submission) {
            Ok(envelope) => MobileHttpResponse::json(200, &envelope),
            Err(err) => bridge_err_resp(err),
        }
    }

    fn handle_connected_care_disconnect(&mut self, req: &MobileHttpRequest) -> MobileHttpResponse {
        let account_id = match self.authenticate(req) {
            Ok(id) => id,
            Err(resp) => return resp,
        };
        let body: ConnectedCareDisconnectBody = match serde_json::from_slice(&req.body) {
            Ok(b) => b,
            Err(err) => return MobileHttpResponse::bad_request(err.to_string()),
        };
        let bridge_acct = match BridgeMobileAccountId::new(account_id.as_str()) {
            Ok(id) => id,
            Err(err) => return bridge_err_resp(BridgeError::InvalidId(err)),
        };
        let now_str = format!("epoch-{}", self.clock_secs);
        match self.connected_care.end_connection(
            &body.connection_id,
            DisconnectActor::Patient {
                account_id: bridge_acct,
            },
            now_str,
        ) {
            Ok(standing) => MobileHttpResponse::json(200, &standing),
            Err(err) => bridge_err_resp(err),
        }
    }

    fn handle_connected_care_status(
        &self,
        req: &MobileHttpRequest,
        query: Option<&str>,
    ) -> MobileHttpResponse {
        let account_id = match self.authenticate(req) {
            Ok(id) => id,
            Err(resp) => return resp,
        };
        if let Some(qs) = query {
            let mut practice_raw = None;
            let mut patient_raw = None;
            for pair in qs.split('&') {
                if let Some((k, v)) = pair.split_once('=') {
                    match k {
                        "practice_id" => practice_raw = Some(v),
                        "patient_id" => patient_raw = Some(v),
                        _ => {}
                    }
                }
            }
            if let (Some(pr), Some(pa)) = (practice_raw, patient_raw) {
                let practice_id = match PracticeId::new(pr) {
                    Ok(id) => id,
                    Err(err) => return MobileHttpResponse::bad_request(err.to_string()),
                };
                let patient_id = match PatientId::new(pa) {
                    Ok(id) => id,
                    Err(err) => return MobileHttpResponse::bad_request(err.to_string()),
                };
                let standing = self
                    .connected_care
                    .mobile_standing(&PairKey::new(practice_id, patient_id));
                return MobileHttpResponse::json(200, &standing);
            }
        }
        let bridge_acct = match BridgeMobileAccountId::new(account_id.as_str()) {
            Ok(id) => id,
            Err(err) => return bridge_err_resp(BridgeError::InvalidId(err)),
        };
        let active = self
            .connected_care
            .active_connections_for_account(&bridge_acct);
        MobileHttpResponse::json(200, &serde_json::json!({ "connections": active }))
    }
}

fn bridge_err_resp(err: BridgeError) -> MobileHttpResponse {
    let status = match &err {
        BridgeError::InvalidId(_)
        | BridgeError::InvalidDateOfBirth { .. }
        | BridgeError::InvalidAccountIncarnation(_)
        | BridgeError::EmptyField { .. }
        | BridgeError::EmptyTransferEnvelope => 400,
        BridgeError::UnauthorizedSender { .. }
        | BridgeError::UnauthorizedPractice { .. }
        | BridgeError::DobVerificationRequired { .. }
        | BridgeError::Part2WrittenConsentRequired
        | BridgeError::Part2ConsentScopeMismatch
        | BridgeError::Part2VerbatimNoticeMissing { .. }
        | BridgeError::AccountDeleted { .. } => 403,
        BridgeError::NoOpenInvitation { .. }
        | BridgeError::InvalidOrRevokedCredential { .. }
        | BridgeError::TransferNotFound { .. } => 404,
        BridgeError::CannotInviteInState { .. }
        | BridgeError::InvitationLocked { .. }
        | BridgeError::InvitationCancelled { .. }
        | BridgeError::ConnectionNotActive { .. }
        | BridgeError::InvalidTransferTransition { .. } => 409,
    };
    MobileHttpResponse::json(
        status,
        &serde_json::json!({
            "error": "connected_care_error",
            "detail": err.to_string(),
        }),
    )
}

fn auth_err_resp(err: MobileAuthError) -> MobileHttpResponse {
    let status = match err {
        MobileAuthError::AccountDeleted => 403,
        MobileAuthError::InvalidEmail | MobileAuthError::InvalidIdentifier { .. } => 400,
        _ => 401,
    };
    MobileHttpResponse::json(
        status,
        &serde_json::json!({
            "error": "mobile_auth_error",
            "detail": err.to_string(),
        }),
    )
}
