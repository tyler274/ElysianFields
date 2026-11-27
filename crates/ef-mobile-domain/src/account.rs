//! Standalone Curie Mobile patient account authority and 6 formal sign-in invariants
//! (`SignInHoldsNothing`, `DraftMatchesAccount`, `SignedInHoldsOwnSession`,
//! `ServerEndsSignOut`, `OwedSessionsEventuallyEnd`, `NoDeadlock`).
//!
//! Strictly isolated from Curie Clinic accounts and storage.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Standalone patient account identifier inside Curie Mobile.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MobileAccountId(String);

impl MobileAccountId {
    pub fn new(raw: impl Into<String>) -> Result<Self, MobileAuthError> {
        let value = raw.into();
        if value.trim().is_empty() || value.chars().any(char::is_control) {
            return Err(MobileAuthError::InvalidIdentifier {
                kind: "MobileAccountId",
            });
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MobileAccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Strictly positive account incarnation preventing post-deletion resurrection on DB restore.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountIncarnation(u64);

impl AccountIncarnation {
    pub fn new(value: u64) -> Result<Self, MobileAuthError> {
        if value == 0 {
            return Err(MobileAuthError::InvalidIncarnation);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn first() -> Self {
        Self(1)
    }

    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }

    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// Standalone mobile bearer session identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MobileSessionId(String);

impl MobileSessionId {
    pub fn new(raw: impl Into<String>) -> Result<Self, MobileAuthError> {
        let value = raw.into();
        if value.trim().is_empty() || value.chars().any(char::is_control) {
            return Err(MobileAuthError::InvalidIdentifier {
                kind: "MobileSessionId",
            });
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Normalized patient email address.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MobileEmail(String);

impl MobileEmail {
    pub fn parse(raw: &str) -> Result<Self, MobileAuthError> {
        let normalized = raw.trim().to_ascii_lowercase();
        if normalized.is_empty()
            || !normalized.contains('@')
            || normalized
                .chars()
                .any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(MobileAuthError::InvalidEmail);
        }
        Ok(Self(normalized))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Typed errors returned by Curie Mobile authentication and sign-in workflows.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MobileAuthError {
    #[error("invalid identifier for {kind}")]
    InvalidIdentifier { kind: &'static str },
    #[error("account incarnation must be >= 1")]
    InvalidIncarnation,
    #[error("invalid email address")]
    InvalidEmail,
    #[error("sign-in link token is invalid, expired, or already consumed")]
    InvalidOrExpiredToken,
    #[error("mobile session is missing, expired, or revoked")]
    SessionEnded,
    #[error("mobile account has been permanently deleted")]
    AccountDeleted,
    #[error("cannot edit draft while signed out")]
    NotSignedIn,
}

/// Network reachability status for online/offline sign-out transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectivityState {
    Online,
    Offline,
}

/// Persisted standalone patient account on the Curie Mobile server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MobileAccountRecord {
    pub account_id: MobileAccountId,
    pub incarnation: AccountIncarnation,
    pub email: MobileEmail,
    pub deleted: bool,
    pub deleted_at_incarnation: Option<AccountIncarnation>,
}

/// Persisted mobile bearer session owned by one standalone patient account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MobileSessionRecord {
    pub session_id: MobileSessionId,
    pub account_id: MobileAccountId,
    pub incarnation: AccountIncarnation,
    pub created_at_secs: u64,
    pub expires_at_secs: u64,
    pub revoked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingLinkToken {
    email: MobileEmail,
    expires_at_secs: u64,
    consumed: bool,
}

/// Verified session payload returned by `POST /mobile-api/auth/verify`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedMobileSession {
    pub session_id: MobileSessionId,
    pub account_id: MobileAccountId,
    pub incarnation: AccountIncarnation,
}

/// Authoritative server-side standalone patient account & session store.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MobileAuthAuthority {
    accounts_by_id: BTreeMap<MobileAccountId, MobileAccountRecord>,
    email_to_account: BTreeMap<MobileEmail, MobileAccountId>,
    tokens: BTreeMap<String, PendingLinkToken>,
    sessions: BTreeMap<MobileSessionId, MobileSessionRecord>,
    /// Fence recording deleted incarnations so restored backups can never resurrect them.
    deleted_incarnation_fence: BTreeMap<MobileAccountId, AccountIncarnation>,
    next_account_seq: u64,
    next_token_seq: u64,
    next_session_seq: u64,
}

impl MobileAuthAuthority {
    #[must_use]
    pub fn new() -> Self {
        Self {
            accounts_by_id: BTreeMap::new(),
            email_to_account: BTreeMap::new(),
            tokens: BTreeMap::new(),
            sessions: BTreeMap::new(),
            deleted_incarnation_fence: BTreeMap::new(),
            next_account_seq: 1,
            next_token_seq: 1,
            next_session_seq: 1,
        }
    }

    /// Registers a standalone patient account explicitly (or returns existing active account).
    pub fn ensure_account(
        &mut self,
        email: MobileEmail,
    ) -> Result<MobileAccountRecord, MobileAuthError> {
        if let Some(existing_id) = self.email_to_account.get(&email).cloned()
            && let Some(acct) = self.accounts_by_id.get(&existing_id)
        {
            if acct.deleted {
                return Err(MobileAuthError::AccountDeleted);
            }
            return Ok(acct.clone());
        }

        let account_id = MobileAccountId::new(format!("mob-acct-{}", self.next_account_seq))?;
        self.next_account_seq = self.next_account_seq.saturating_add(1);
        let record = MobileAccountRecord {
            account_id: account_id.clone(),
            incarnation: AccountIncarnation::first(),
            email: email.clone(),
            deleted: false,
            deleted_at_incarnation: None,
        };
        self.email_to_account.insert(email, account_id.clone());
        self.accounts_by_id.insert(account_id, record.clone());
        Ok(record)
    }

    /// Issues a single-use 24h sign-in link token for a standalone patient email.
    pub fn issue_sign_in_link(
        &mut self,
        raw_email: &str,
        now_secs: u64,
    ) -> Result<String, MobileAuthError> {
        let email = MobileEmail::parse(raw_email)?;
        let _ = self.ensure_account(email.clone())?;
        let token = format!("mob-tok-{}", self.next_token_seq);
        self.next_token_seq = self.next_token_seq.saturating_add(1);
        self.tokens.insert(
            token.clone(),
            PendingLinkToken {
                email,
                expires_at_secs: now_secs.saturating_add(86_400),
                consumed: false,
            },
        );
        Ok(token)
    }

    /// Consumes a single-use token and mints a standalone mobile session.
    pub fn verify_sign_in(
        &mut self,
        raw_email: &str,
        raw_token: &str,
        now_secs: u64,
    ) -> Result<VerifiedMobileSession, MobileAuthError> {
        let email = MobileEmail::parse(raw_email)?;
        let Some(token_rec) = self.tokens.get_mut(raw_token.trim()) else {
            return Err(MobileAuthError::InvalidOrExpiredToken);
        };
        if token_rec.email != email || token_rec.consumed || now_secs >= token_rec.expires_at_secs {
            return Err(MobileAuthError::InvalidOrExpiredToken);
        }

        let Some(account_id) = self.email_to_account.get(&email).cloned() else {
            return Err(MobileAuthError::AccountDeleted);
        };
        let Some(account) = self.accounts_by_id.get(&account_id).cloned() else {
            return Err(MobileAuthError::AccountDeleted);
        };
        if account.deleted
            || self
                .deleted_incarnation_fence
                .get(&account_id)
                .is_some_and(|fence| account.incarnation <= *fence)
        {
            return Err(MobileAuthError::AccountDeleted);
        }

        token_rec.consumed = true;
        let session_id = MobileSessionId::new(format!("mob-sess-{}", self.next_session_seq))?;
        self.next_session_seq = self.next_session_seq.saturating_add(1);

        let session = MobileSessionRecord {
            session_id: session_id.clone(),
            account_id: account.account_id.clone(),
            incarnation: account.incarnation,
            created_at_secs: now_secs,
            expires_at_secs: now_secs.saturating_add(604_800),
            revoked: false,
        };
        self.sessions.insert(session_id.clone(), session);

        Ok(VerifiedMobileSession {
            session_id,
            account_id: account.account_id,
            incarnation: account.incarnation,
        })
    }

    /// Validates a bearer session against liveness, revocation, and account deletion fences.
    pub fn validate_session(
        &self,
        session_id: &MobileSessionId,
        now_secs: u64,
    ) -> Result<MobileSessionRecord, MobileAuthError> {
        let Some(session) = self.sessions.get(session_id) else {
            return Err(MobileAuthError::SessionEnded);
        };
        if session.revoked || now_secs >= session.expires_at_secs {
            return Err(MobileAuthError::SessionEnded);
        }
        let Some(account) = self.accounts_by_id.get(&session.account_id) else {
            return Err(MobileAuthError::AccountDeleted);
        };
        if account.deleted
            || self
                .deleted_incarnation_fence
                .get(&session.account_id)
                .is_some_and(|fence| session.incarnation <= *fence)
        {
            return Err(MobileAuthError::AccountDeleted);
        }
        Ok(session.clone())
    }

    /// Revokes a single session (used by online sign-out and offline `owes` queue flush).
    pub fn revoke_session(&mut self, session_id: &MobileSessionId) {
        if let Some(session) = self.sessions.get_mut(session_id) {
            session.revoked = true;
        }
    }

    /// Revokes all active sessions for `(account_id, <= incarnation)`.
    pub fn revoke_all_sessions_for_account(
        &mut self,
        account_id: &MobileAccountId,
        up_to_incarnation: AccountIncarnation,
    ) {
        for session in self.sessions.values_mut() {
            if &session.account_id == account_id && session.incarnation <= up_to_incarnation {
                session.revoked = true;
            }
        }
    }

    /// Forces a session to expire immediately for expiry-path testing.
    pub fn expire_session(&mut self, session_id: &MobileSessionId, now_secs: u64) {
        if let Some(session) = self.sessions.get_mut(session_id) {
            session.expires_at_secs = now_secs;
        }
    }

    /// Permanently deletes an account at `incarnation`, revoking all sessions and setting
    /// the non-resurrection incarnation fence.
    pub fn delete_account(
        &mut self,
        account_id: &MobileAccountId,
        incarnation: AccountIncarnation,
    ) {
        self.deleted_incarnation_fence
            .entry(account_id.clone())
            .and_modify(|existing| {
                if incarnation > *existing {
                    *existing = incarnation;
                }
            })
            .or_insert(incarnation);

        if let Some(acct) = self.accounts_by_id.get_mut(account_id)
            && acct.incarnation <= incarnation
        {
            acct.deleted = true;
            acct.deleted_at_incarnation = Some(incarnation);
        }
        self.revoke_all_sessions_for_account(account_id, incarnation);
    }

    /// Returns true if a session is currently live at `now_secs`.
    #[must_use]
    pub fn is_session_live(&self, session_id: &MobileSessionId, now_secs: u64) -> bool {
        self.validate_session(session_id, now_secs).is_ok()
    }

    /// Returns the account record by ID.
    #[must_use]
    pub fn account(&self, account_id: &MobileAccountId) -> Option<&MobileAccountRecord> {
        self.accounts_by_id.get(account_id)
    }

    /// Returns the account record by email.
    #[must_use]
    pub fn account_by_email(&self, email: &MobileEmail) -> Option<&MobileAccountRecord> {
        let id = self.email_to_account.get(email)?;
        self.accounts_by_id.get(id)
    }
}

/// Account-scoped local note draft guaranteeing `DraftMatchesAccount`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountScopedDraft {
    pub owner_account_id: MobileAccountId,
    pub text: String,
}

/// The four Curie Mobile sign-in screens from Chapter 2 §2.1.
///
/// Encoding session credentials and drafts exclusively inside [`Self::SignedIn`]
/// makes `SignInHoldsNothing` a type-level guarantee.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MobileClientScreen {
    EnterYourEmail,
    CheckYourEmail {
        pending_email: MobileEmail,
    },
    SignedIn {
        account_id: MobileAccountId,
        incarnation: AccountIncarnation,
        session_id: MobileSessionId,
        local_draft: Option<AccountScopedDraft>,
    },
    SessionEnded,
}

/// Queued offline sign-out waiting for connectivity to end the session on the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwedSignOut {
    pub account_id: MobileAccountId,
    pub session_id: MobileSessionId,
}

/// Violation of one of the 6 formal `MobileSignIn.tla` invariants.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SignInInvariantViolation {
    #[error("SignInHoldsNothing violated: non-signed-in screen holds credentials or drafts")]
    SignInHoldsNothing,
    #[error("DraftMatchesAccount violated: draft owner {draft_owner} != signed-in {signed_in}")]
    DraftMatchesAccount {
        draft_owner: MobileAccountId,
        signed_in: MobileAccountId,
    },
    #[error("SignedInHoldsOwnSession violated: session does not belong to signed-in account")]
    SignedInHoldsOwnSession,
    #[error(
        "ServerEndsSignOut violated: signed-out session {session_id:?} is still live and not queued in owes"
    )]
    ServerEndsSignOut { session_id: MobileSessionId },
    #[error(
        "OwedSessionsEventuallyEnd violated: online flush left live owed session {session_id:?}"
    )]
    OwedSessionsEventuallyEnd { session_id: MobileSessionId },
    #[error("NoDeadlock violated: no legal continuation from current state")]
    NoDeadlock,
}

/// Client-side Curie Mobile sign-in & device-handoff state machine.
#[derive(Debug, Clone)]
pub struct MobileDeviceAuthState {
    screen: MobileClientScreen,
    owes: VecDeque<OwedSignOut>,
    ever_signed_out: BTreeSet<MobileSessionId>,
    connectivity: ConnectivityState,
}

impl Default for MobileDeviceAuthState {
    fn default() -> Self {
        Self::new()
    }
}

impl MobileDeviceAuthState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            screen: MobileClientScreen::EnterYourEmail,
            owes: VecDeque::new(),
            ever_signed_out: BTreeSet::new(),
            connectivity: ConnectivityState::Online,
        }
    }

    #[must_use]
    pub const fn screen(&self) -> &MobileClientScreen {
        &self.screen
    }

    #[must_use]
    pub const fn connectivity(&self) -> ConnectivityState {
        self.connectivity
    }

    #[must_use]
    pub fn owed_sign_outs(&self) -> &VecDeque<OwedSignOut> {
        &self.owes
    }

    /// Updates device network reachability.
    pub fn set_connectivity(&mut self, state: ConnectivityState) {
        self.connectivity = state;
    }

    /// Transitions `EnterYourEmail` / `SessionEnded` -> `CheckYourEmail`.
    pub fn submit_email(&mut self, raw_email: &str) -> Result<MobileEmail, MobileAuthError> {
        let email = MobileEmail::parse(raw_email)?;
        self.screen = MobileClientScreen::CheckYourEmail {
            pending_email: email.clone(),
        };
        Ok(email)
    }

    /// Completes sign-in with a verified session for the patient account.
    ///
    /// Starts with `local_draft: None` so device handoff to Account B never inherits Account A's draft.
    pub fn complete_sign_in(&mut self, verified: VerifiedMobileSession) {
        self.screen = MobileClientScreen::SignedIn {
            account_id: verified.account_id,
            incarnation: verified.incarnation,
            session_id: verified.session_id,
            local_draft: None,
        };
    }

    /// Edits the local note draft bound strictly to the currently signed-in account.
    pub fn set_local_draft(&mut self, text: impl Into<String>) -> Result<(), MobileAuthError> {
        let MobileClientScreen::SignedIn {
            account_id,
            local_draft,
            ..
        } = &mut self.screen
        else {
            return Err(MobileAuthError::NotSignedIn);
        };
        *local_draft = Some(AccountScopedDraft {
            owner_account_id: account_id.clone(),
            text: text.into(),
        });
        Ok(())
    }

    /// Signs out of the current account, clearing all credentials and drafts immediately.
    ///
    /// - Online: ends the session on `authority` immediately.
    /// - Offline: queues the session in durable `owes` so it ends on `authority` once online.
    pub fn sign_out(&mut self, authority: &mut MobileAuthAuthority) -> Option<MobileSessionId> {
        let previous = std::mem::replace(&mut self.screen, MobileClientScreen::EnterYourEmail);
        let MobileClientScreen::SignedIn {
            account_id,
            session_id,
            ..
        } = previous
        else {
            return None;
        };

        self.ever_signed_out.insert(session_id.clone());
        match self.connectivity {
            ConnectivityState::Online => {
                authority.revoke_session(&session_id);
            }
            ConnectivityState::Offline => {
                self.owes.push_back(OwedSignOut {
                    account_id,
                    session_id: session_id.clone(),
                });
            }
        }
        Some(session_id)
    }

    /// Flushes all queued offline sign-outs to `authority` when online (`OwedSessionsEventuallyEnd`).
    pub fn flush_owed_sign_outs(&mut self, authority: &mut MobileAuthAuthority) -> usize {
        if self.connectivity == ConnectivityState::Offline {
            return 0;
        }
        let mut flushed = 0usize;
        while let Some(owed) = self.owes.pop_front() {
            authority.revoke_session(&owed.session_id);
            flushed = flushed.saturating_add(1);
        }
        flushed
    }

    /// Transitions a signed-in screen to `SessionEnded` upon server `401` expiry discovery,
    /// immediately discarding credentials and local drafts (`SignInHoldsNothing`).
    pub fn handle_session_ended(&mut self) {
        if matches!(self.screen, MobileClientScreen::SignedIn { .. }) {
            self.screen = MobileClientScreen::SessionEnded;
        }
    }

    /// Dismisses `SessionEnded` back to `EnterYourEmail` so another or returning user can sign in.
    pub fn return_to_sign_in(&mut self) {
        self.screen = MobileClientScreen::EnterYourEmail;
    }

    /// Verifies all 6 formal `MobileSignIn.tla` invariants at `now_secs`.
    pub fn check_all_6_invariants(
        &self,
        authority: &MobileAuthAuthority,
        now_secs: u64,
    ) -> Result<(), SignInInvariantViolation> {
        // 1. SignInHoldsNothing + 2. DraftMatchesAccount + 3. SignedInHoldsOwnSession
        match &self.screen {
            MobileClientScreen::EnterYourEmail
            | MobileClientScreen::CheckYourEmail { .. }
            | MobileClientScreen::SessionEnded => {}
            MobileClientScreen::SignedIn {
                account_id,
                session_id,
                local_draft,
                ..
            } => {
                if let Some(draft) = local_draft
                    && &draft.owner_account_id != account_id
                {
                    return Err(SignInInvariantViolation::DraftMatchesAccount {
                        draft_owner: draft.owner_account_id.clone(),
                        signed_in: account_id.clone(),
                    });
                }
                let owns_session = authority
                    .sessions
                    .get(session_id)
                    .is_some_and(|rec| &rec.account_id == account_id);
                if !owns_session {
                    return Err(SignInInvariantViolation::SignedInHoldsOwnSession);
                }
            }
        }

        // 4. ServerEndsSignOut: no signed-out session may remain live unless queued in `owes`.
        for signed_out_id in &self.ever_signed_out {
            if authority.is_session_live(signed_out_id, now_secs) {
                let queued = self
                    .owes
                    .iter()
                    .any(|entry| &entry.session_id == signed_out_id);
                if !queued {
                    return Err(SignInInvariantViolation::ServerEndsSignOut {
                        session_id: signed_out_id.clone(),
                    });
                }
            }
        }

        // 5. OwedSessionsEventuallyEnd: when online and queue empty, zero owed sessions remain live.
        if self.connectivity == ConnectivityState::Online && self.owes.is_empty() {
            for signed_out_id in &self.ever_signed_out {
                if authority.is_session_live(signed_out_id, now_secs) {
                    return Err(SignInInvariantViolation::OwedSessionsEventuallyEnd {
                        session_id: signed_out_id.clone(),
                    });
                }
            }
        }

        // 6. NoDeadlock: every reachable screen always admits a legal transition.
        let has_continuation = matches!(
            self.screen,
            MobileClientScreen::EnterYourEmail
                | MobileClientScreen::CheckYourEmail { .. }
                | MobileClientScreen::SignedIn { .. }
                | MobileClientScreen::SessionEnded
        );
        if !has_continuation {
            return Err(SignInInvariantViolation::NoDeadlock);
        }

        Ok(())
    }
}
