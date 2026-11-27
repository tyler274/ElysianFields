//! Typed Curie Mobile client transport layer.
//!
//! Enforces the Chapter 2 §2.3 UI isolation boundary: Bevy UI screens read
//! synced health trends and patient records strictly through this typed transport
//! and NEVER invoke native platform health bridges directly.

use ef_connected_care::{
    AccountIncarnation as BridgeAccountIncarnation, BridgeError, ConnectedCareBridge, ConnectionId,
    ConnectionStanding, DateOfBirth, DisconnectActor, DobVerificationOutcome, EnvelopeItem,
    InviteChallenge, InviteCredential, MobileAccountId as BridgeMobileAccountId, PairKey,
    Part2WrittenConsent, TransferEnvelope, TransferSubmission,
};
use ef_core::{IdempotencyKey, Subpath};
use ef_mobile_domain::{
    AccountFencedHealthBatch, HealthSampleEntry, HealthSyncError, HealthUploadReceipt,
    IncomingBatchItem, MobileAccountId, MobileAuthAuthority, MobileAuthError,
    MobileHealthIngestStore, MobileSessionId, PatientRecord, PatientRecordStore,
    SavepointBatchReport, VerifiedMobileSession,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Typed transport error surfaced cleanly to Curie Mobile UI state.
#[derive(Debug, Error)]
pub enum MobileTransportError {
    #[error("authentication error: {0}")]
    Auth(#[from] MobileAuthError),
    #[error("health sync error: {0}")]
    Health(#[from] HealthSyncError),
    #[error("connected care error: {0}")]
    Bridge(#[from] BridgeError),
    #[error("wire serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("HTTP {status} ({error}): {detail}")]
    Http {
        status: u16,
        error: String,
        detail: String,
    },
    #[error("transport unavailable: {message}")]
    Unavailable { message: String },
    #[error("device is offline")]
    Offline,
}

/// Receipt returned after permanent mobile account deletion commits to the privacy journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountDeleteReceipt {
    pub deleted: bool,
    pub receipt_event_id: String,
    pub scrubbed_staging_records: usize,
}

/// Typed result for Connected Care status queries (single pair vs all active account links).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectedCareStatusResult {
    Pair(ConnectionStanding),
    ActiveConnections(Vec<ConnectionStanding>),
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
    PushRegister,
    Export,
    DeleteAccount,
    ConnectedCareLookup,
    ConnectedCareVerifyDob,
    ConnectedCareShare,
    ConnectedCareTransfer,
    ConnectedCareDisconnect,
    ConnectedCareStatus,
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
            Self::PushRegister => "mobile-api/push/register",
            Self::Export => "mobile-api/export",
            Self::DeleteAccount => "mobile-api/account/delete",
            Self::ConnectedCareLookup => "mobile-api/connected-care/lookup",
            Self::ConnectedCareVerifyDob => "mobile-api/connected-care/verify-dob",
            Self::ConnectedCareShare => "mobile-api/connected-care/share",
            Self::ConnectedCareTransfer => "mobile-api/connected-care/transfer",
            Self::ConnectedCareDisconnect => "mobile-api/connected-care/disconnect",
            Self::ConnectedCareStatus => "mobile-api/connected-care/status",
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

    fn revoke_sessions(
        &mut self,
        active_session: Option<&MobileSessionId>,
        owed_sessions: &[MobileSessionId],
    ) -> Result<usize, MobileTransportError>;

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

    fn delete_account(
        &mut self,
        session_id: &MobileSessionId,
        event_id: &str,
        occurred_at: &str,
        now_secs: u64,
    ) -> Result<AccountDeleteReceipt, MobileTransportError>;

    fn connected_care_lookup(
        &self,
        session_id: &MobileSessionId,
        credential: InviteCredential,
        now_secs: u64,
    ) -> Result<InviteChallenge, MobileTransportError>;

    fn connected_care_verify_dob(
        &mut self,
        session_id: &MobileSessionId,
        credential: InviteCredential,
        dob: DateOfBirth,
        now_secs: u64,
    ) -> Result<DobVerificationOutcome, MobileTransportError>;

    fn connected_care_share(
        &mut self,
        session_id: &MobileSessionId,
        credential: InviteCredential,
        now_secs: u64,
    ) -> Result<ConnectionStanding, MobileTransportError>;

    fn connected_care_transfer(
        &mut self,
        session_id: &MobileSessionId,
        idempotency_key: IdempotencyKey,
        connection_id: ConnectionId,
        items: Vec<EnvelopeItem>,
        part2_consent: Option<Part2WrittenConsent>,
        now_secs: u64,
    ) -> Result<TransferEnvelope, MobileTransportError>;

    fn connected_care_disconnect(
        &mut self,
        session_id: &MobileSessionId,
        connection_id: ConnectionId,
        now_secs: u64,
    ) -> Result<ConnectionStanding, MobileTransportError>;

    fn connected_care_status(
        &self,
        session_id: &MobileSessionId,
        pair: Option<&PairKey>,
        now_secs: u64,
    ) -> Result<ConnectedCareStatusResult, MobileTransportError>;
}

/// Deterministic in-process transport backed by `ef-mobile-domain` for headless Bevy tests.
#[derive(Debug, Default)]
pub struct LocalMobileTransport {
    pub auth: MobileAuthAuthority,
    pub records: PatientRecordStore,
    pub health: MobileHealthIngestStore,
    pub connected_care: ConnectedCareBridge,
}

impl LocalMobileTransport {
    #[must_use]
    pub fn new() -> Self {
        Self {
            auth: MobileAuthAuthority::new(),
            records: PatientRecordStore::new(),
            health: MobileHealthIngestStore::new(),
            connected_care: ConnectedCareBridge::new(),
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

    fn revoke_sessions(
        &mut self,
        active_session: Option<&MobileSessionId>,
        owed_sessions: &[MobileSessionId],
    ) -> Result<usize, MobileTransportError> {
        let mut count = 0;
        if let Some(sess) = active_session {
            self.auth.revoke_session(sess);
            count += 1;
        }
        for sess in owed_sessions {
            self.auth.revoke_session(sess);
            count += 1;
        }
        Ok(count)
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

    fn delete_account(
        &mut self,
        session_id: &MobileSessionId,
        event_id: &str,
        occurred_at: &str,
        now_secs: u64,
    ) -> Result<AccountDeleteReceipt, MobileTransportError> {
        let acct_id = self.resolve_account(session_id, now_secs)?;
        let Some(acct) = self.auth.account(&acct_id).cloned() else {
            return Err(MobileTransportError::Auth(MobileAuthError::AccountDeleted));
        };
        self.auth.delete_account(&acct_id, acct.incarnation);
        let _ = self
            .records
            .purge_account_records(&acct_id, acct.incarnation);
        let _ = self.health.purge_account_samples(&acct_id);
        if let Ok(bridge_acct) = BridgeMobileAccountId::new(acct_id.as_str())
            && let Ok(bridge_inc) = BridgeAccountIncarnation::new(acct.incarnation.as_u64())
        {
            let _ = self.connected_care.terminate_account_links(
                &bridge_acct,
                bridge_inc,
                occurred_at.to_owned(),
            );
        }
        Ok(AccountDeleteReceipt {
            deleted: true,
            receipt_event_id: event_id.to_owned(),
            scrubbed_staging_records: 0,
        })
    }

    fn connected_care_lookup(
        &self,
        session_id: &MobileSessionId,
        credential: InviteCredential,
        now_secs: u64,
    ) -> Result<InviteChallenge, MobileTransportError> {
        let _ = self.resolve_account(session_id, now_secs)?;
        Ok(self.connected_care.lookup_challenge(&credential)?)
    }

    fn connected_care_verify_dob(
        &mut self,
        session_id: &MobileSessionId,
        credential: InviteCredential,
        dob: DateOfBirth,
        now_secs: u64,
    ) -> Result<DobVerificationOutcome, MobileTransportError> {
        let acct = self.resolve_account(session_id, now_secs)?;
        let bridge_acct =
            BridgeMobileAccountId::new(acct.as_str()).map_err(BridgeError::InvalidId)?;
        Ok(self.connected_care.verify_dob(
            &credential,
            &bridge_acct,
            dob,
            format!("epoch-{now_secs}"),
        )?)
    }

    fn connected_care_share(
        &mut self,
        session_id: &MobileSessionId,
        credential: InviteCredential,
        now_secs: u64,
    ) -> Result<ConnectionStanding, MobileTransportError> {
        let acct = self.resolve_account(session_id, now_secs)?;
        let bridge_acct =
            BridgeMobileAccountId::new(acct.as_str()).map_err(BridgeError::InvalidId)?;
        Ok(self.connected_care.confirm_share(
            &credential,
            &bridge_acct,
            format!("epoch-{now_secs}"),
        )?)
    }

    fn connected_care_transfer(
        &mut self,
        session_id: &MobileSessionId,
        idempotency_key: IdempotencyKey,
        connection_id: ConnectionId,
        items: Vec<EnvelopeItem>,
        part2_consent: Option<Part2WrittenConsent>,
        now_secs: u64,
    ) -> Result<TransferEnvelope, MobileTransportError> {
        let acct = self.resolve_account(session_id, now_secs)?;
        let bridge_acct =
            BridgeMobileAccountId::new(acct.as_str()).map_err(BridgeError::InvalidId)?;
        let submission = TransferSubmission {
            idempotency_key,
            connection_id,
            sender_account: bridge_acct,
            sent_at: format!("epoch-{now_secs}"),
            items,
            part2_consent,
        };
        Ok(self.connected_care.submit_transfer(submission)?)
    }

    fn connected_care_disconnect(
        &mut self,
        session_id: &MobileSessionId,
        connection_id: ConnectionId,
        now_secs: u64,
    ) -> Result<ConnectionStanding, MobileTransportError> {
        let acct = self.resolve_account(session_id, now_secs)?;
        let bridge_acct =
            BridgeMobileAccountId::new(acct.as_str()).map_err(BridgeError::InvalidId)?;
        Ok(self.connected_care.end_connection(
            &connection_id,
            DisconnectActor::Patient {
                account_id: bridge_acct,
            },
            format!("epoch-{now_secs}"),
        )?)
    }

    fn connected_care_status(
        &self,
        session_id: &MobileSessionId,
        pair: Option<&PairKey>,
        now_secs: u64,
    ) -> Result<ConnectedCareStatusResult, MobileTransportError> {
        let acct = self.resolve_account(session_id, now_secs)?;
        if let Some(pair_key) = pair {
            return Ok(ConnectedCareStatusResult::Pair(
                self.connected_care.mobile_standing(pair_key),
            ));
        }
        let bridge_acct =
            BridgeMobileAccountId::new(acct.as_str()).map_err(BridgeError::InvalidId)?;
        Ok(ConnectedCareStatusResult::ActiveConnections(
            self.connected_care
                .active_connections_for_account(&bridge_acct),
        ))
    }
}
