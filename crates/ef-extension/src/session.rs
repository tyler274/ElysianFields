//! Memory-only credential storage (`chrome.storage.session` contract).

use std::fmt;

use thiserror::Error;

/// Chrome extension storage areas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromeStorageArea {
    /// Volatile in-memory session storage (`chrome.storage.session`).
    SessionMemoryOnly,
    /// Persistent local disk storage (`chrome.storage.local`) — forbidden for credentials.
    LocalDisk,
    /// Cloud-synced storage (`chrome.storage.sync`) — forbidden for credentials.
    SyncCloud,
}

/// Pairing and session lifecycle state for the MV3 extension popup and worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairingState {
    Unpaired,
    Paired {
        pairing_id: String,
        expires_at_ms: u64,
    },
}

/// Active bearer and refresh tokens held strictly in memory.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionCredentials {
    pairing_id: String,
    bearer_token: String,
    refresh_token: String,
    expires_at_ms: u64,
}

impl SessionCredentials {
    /// Creates a validated credential pair for an active pairing.
    pub fn new(
        pairing_id: impl Into<String>,
        bearer_token: impl Into<String>,
        refresh_token: impl Into<String>,
        expires_at_ms: u64,
    ) -> Result<Self, SessionError> {
        let pairing_id = pairing_id.into();
        let bearer_token = bearer_token.into();
        let refresh_token = refresh_token.into();
        if pairing_id.trim().is_empty()
            || bearer_token.trim().is_empty()
            || refresh_token.trim().is_empty()
        {
            return Err(SessionError::EmptyCredentialField);
        }
        Ok(Self {
            pairing_id,
            bearer_token,
            refresh_token,
            expires_at_ms,
        })
    }

    #[must_use]
    pub fn pairing_id(&self) -> &str {
        &self.pairing_id
    }

    #[must_use]
    pub fn bearer_token(&self) -> &str {
        &self.bearer_token
    }

    #[must_use]
    pub fn refresh_token(&self) -> &str {
        &self.refresh_token
    }

    #[must_use]
    pub const fn expires_at_ms(&self) -> u64 {
        self.expires_at_ms
    }

    #[must_use]
    pub const fn is_expired(&self, now_ms: u64) -> bool {
        now_ms >= self.expires_at_ms
    }
}

impl fmt::Debug for SessionCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionCredentials")
            .field("pairing_id", &self.pairing_id)
            .field("bearer_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}

/// Errors produced by the memory-only credential store.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SessionError {
    #[error("pairing_id, bearer_token, and refresh_token must all be non-empty")]
    EmptyCredentialField,
    #[error("credentials are forbidden in {area:?}; only chrome.storage.session is permitted")]
    NonSessionStorageForbidden { area: ChromeStorageArea },
    #[error("extension is not paired or session credentials have expired")]
    NotPaired,
}

/// Models Chrome MV3 extension storage areas while guaranteeing credentials live solely in `SessionMemoryOnly`.
#[derive(Debug, Default)]
pub struct MemoryOnlySessionStore {
    session: Option<SessionCredentials>,
    local_disk_keys: Vec<String>,
    sync_cloud_keys: Vec<String>,
}

impl MemoryOnlySessionStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stores active credentials exclusively in `ChromeStorageArea::SessionMemoryOnly`.
    pub fn store_credentials(
        &mut self,
        area: ChromeStorageArea,
        creds: SessionCredentials,
    ) -> Result<(), SessionError> {
        if area != ChromeStorageArea::SessionMemoryOnly {
            return Err(SessionError::NonSessionStorageForbidden { area });
        }
        self.session = Some(creds);
        Ok(())
    }

    /// Returns active non-expired credentials, clearing expired tokens automatically.
    pub fn active_credentials(&mut self, now_ms: u64) -> Result<&SessionCredentials, SessionError> {
        let expired = self.session.as_ref().is_some_and(|c| c.is_expired(now_ms));
        if expired {
            self.session = None;
            return Err(SessionError::NotPaired);
        }
        self.session.as_ref().ok_or(SessionError::NotPaired)
    }

    /// Reports current pairing status for the MV3 popup without exposing token material.
    #[must_use]
    pub fn pairing_state(&self, now_ms: u64) -> PairingState {
        match &self.session {
            Some(creds) if !creds.is_expired(now_ms) => PairingState::Paired {
                pairing_id: creds.pairing_id.clone(),
                expires_at_ms: creds.expires_at_ms,
            },
            _ => PairingState::Unpaired,
        }
    }

    /// Clears expired credentials on service-worker suspension or alarm wakeup.
    pub fn sweep_expired(&mut self, now_ms: u64) -> bool {
        if self.session.as_ref().is_some_and(|c| c.is_expired(now_ms)) {
            self.session = None;
            return true;
        }
        false
    }

    /// Explicitly unpairs the extension and clears all memory-only credentials.
    pub fn clear_session(&mut self) {
        self.session = None;
    }

    /// Exposes keys written to `chrome.storage.local` for audit tests (always empty for credentials).
    #[must_use]
    pub fn local_disk_keys(&self) -> &[String] {
        &self.local_disk_keys
    }

    /// Exposes keys written to `chrome.storage.sync` for audit tests (always empty for credentials).
    #[must_use]
    pub fn sync_cloud_keys(&self) -> &[String] {
        &self.sync_cloud_keys
    }
}
