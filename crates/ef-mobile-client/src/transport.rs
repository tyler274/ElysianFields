//! Typed Curie Mobile client transport layer.
//!
//! Enforces the Chapter 2 §2.3 UI isolation boundary: Bevy UI screens read
//! synced health trends and patient records strictly through this typed transport
//! and NEVER invoke native HealthKit / Health Connect platform bridges directly.

use ef_core::Subpath;
use ef_mobile_domain::{
    AccountFencedHealthBatch, HealthSampleEntry, HealthSyncError, HealthUploadReceipt,
    IncomingBatchItem, MobileAccountId, MobileAuthAuthority, MobileAuthError,
    MobileHealthIngestStore, MobileSessionId, PatientRecord, PatientRecordStore,
    SavepointBatchReport, VerifiedMobileSession,
};
use thiserror::Error;

/// Typed transport error surfaced cleanly to Curie Mobile UI state.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MobileTransportError {
    #[error("authentication error: {0}")]
    Auth(#[from] MobileAuthError),
    #[error("health sync error: {0}")]
    Health(#[from] HealthSyncError),
    #[error("device is offline")]
    Offline,
}

/// Canonical `/elysianfields/mobile-api/...` endpoint resolver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobileApiEndpoint {
    RequestLink,
    Verify,
    SignOut,
    Session,
    IngestRecords,
    ListRecords,
    UploadHealth,
    HealthTrends,
    Export,
    DeleteAccount,
    Readyz,
}

impl MobileApiEndpoint {
    #[must_use]
    pub fn path(self) -> String {
        let rel = match self {
            Self::RequestLink => "mobile-api/auth/request-link",
            Self::Verify => "mobile-api/auth/verify",
            Self::SignOut => "mobile-api/auth/sign-out",
            Self::Session => "mobile-api/auth/session",
            Self::IngestRecords => "mobile-api/records/ingest",
            Self::ListRecords => "mobile-api/records",
            Self::UploadHealth => "mobile-api/health/upload",
            Self::HealthTrends => "mobile-api/health/trends",
            Self::Export => "mobile-api/export",
            Self::DeleteAccount => "mobile-api/account/delete",
            Self::Readyz => "mobile-api/readyz",
        };
        format!("{}/{rel}", Subpath::CANONICAL_PREFIX)
    }
}

/// Typed client transport boundary bridging Curie Mobile UI to the standalone mobile backend.
pub trait MobileClientTransport: Send + Sync {
    fn request_sign_in_link(
        &mut self,
        email: &str,
        now_secs: u64,
    ) -> Result<String, MobileTransportError>;

    fn verify_sign_in(
        &mut self,
        email: &str,
        token: &str,
        now_secs: u64,
    ) -> Result<VerifiedMobileSession, MobileTransportError>;

    fn revoke_session(&mut self, session_id: &MobileSessionId) -> Result<(), MobileTransportError>;

    fn ingest_records(
        &mut self,
        session_id: &MobileSessionId,
        items: Vec<IncomingBatchItem>,
        now_secs: u64,
    ) -> Result<SavepointBatchReport, MobileTransportError>;

    fn fetch_records(
        &self,
        session_id: &MobileSessionId,
        now_secs: u64,
    ) -> Result<Vec<PatientRecord>, MobileTransportError>;

    fn upload_health_batch(
        &mut self,
        session_id: &MobileSessionId,
        batch: AccountFencedHealthBatch,
        now_secs: u64,
    ) -> Result<HealthUploadReceipt, MobileTransportError>;

    fn fetch_health_trends(
        &self,
        session_id: &MobileSessionId,
        now_secs: u64,
    ) -> Result<Vec<HealthSampleEntry>, MobileTransportError>;

    fn is_session_live(&self, session_id: &MobileSessionId, now_secs: u64) -> bool;
}

/// Deterministic in-process transport backed by `ef-mobile-domain` for headless Bevy tests.
#[derive(Debug, Default)]
pub struct LocalMobileTransport {
    pub auth: MobileAuthAuthority,
    pub records: PatientRecordStore,
    pub health: MobileHealthIngestStore,
}

impl LocalMobileTransport {
    #[must_use]
    pub fn new() -> Self {
        Self {
            auth: MobileAuthAuthority::new(),
            records: PatientRecordStore::new(),
            health: MobileHealthIngestStore::new(),
        }
    }

    fn resolve_account(
        &self,
        session_id: &MobileSessionId,
        now_secs: u64,
    ) -> Result<MobileAccountId, MobileTransportError> {
        Ok(self.auth.validate_session(session_id, now_secs)?.account_id)
    }
}

impl MobileClientTransport for LocalMobileTransport {
    fn request_sign_in_link(
        &mut self,
        email: &str,
        now_secs: u64,
    ) -> Result<String, MobileTransportError> {
        Ok(self.auth.issue_sign_in_link(email, now_secs)?)
    }

    fn verify_sign_in(
        &mut self,
        email: &str,
        token: &str,
        now_secs: u64,
    ) -> Result<VerifiedMobileSession, MobileTransportError> {
        Ok(self.auth.verify_sign_in(email, token, now_secs)?)
    }

    fn revoke_session(&mut self, session_id: &MobileSessionId) -> Result<(), MobileTransportError> {
        self.auth.revoke_session(session_id);
        Ok(())
    }

    fn ingest_records(
        &mut self,
        session_id: &MobileSessionId,
        items: Vec<IncomingBatchItem>,
        now_secs: u64,
    ) -> Result<SavepointBatchReport, MobileTransportError> {
        let _ = self.resolve_account(session_id, now_secs)?;
        Ok(self.records.ingest_batch_with_savepoints(items))
    }

    fn fetch_records(
        &self,
        session_id: &MobileSessionId,
        now_secs: u64,
    ) -> Result<Vec<PatientRecord>, MobileTransportError> {
        let acct = self.resolve_account(session_id, now_secs)?;
        Ok(self
            .records
            .records_for_account(&acct)
            .into_iter()
            .cloned()
            .collect())
    }

    fn upload_health_batch(
        &mut self,
        session_id: &MobileSessionId,
        batch: AccountFencedHealthBatch,
        now_secs: u64,
    ) -> Result<HealthUploadReceipt, MobileTransportError> {
        let acct = self.resolve_account(session_id, now_secs)?;
        Ok(self.health.ingest_batch(&acct, batch)?)
    }

    fn fetch_health_trends(
        &self,
        session_id: &MobileSessionId,
        now_secs: u64,
    ) -> Result<Vec<HealthSampleEntry>, MobileTransportError> {
        let acct = self.resolve_account(session_id, now_secs)?;
        Ok(self.health.samples_for_account(&acct).to_vec())
    }

    fn is_session_live(&self, session_id: &MobileSessionId, now_secs: u64) -> bool {
        self.auth.is_session_live(session_id, now_secs)
    }
}
