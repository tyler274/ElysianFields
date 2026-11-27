//! Physician invite-only authentication, session lifetime & sliding renewal,
//! structured refusals, multi-tab session coordination, and all 11 formal sign-in invariants.

use std::collections::{BTreeMap, VecDeque};

use ef_core::{
    IdError, PhysicianId, PracticeId, RefusalCode, SessionId, StructuredRefusal, TabId, WorkspaceId,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Single-use sign-in verification link TTL: 24 hours (`86,400 s`).
pub const VERIFICATION_TOKEN_TTL_SECS: u64 = 86_400;

/// Colleague invitation token TTL: 14 days (`1,209,600 s`).
pub const INVITATION_TOKEN_TTL_SECS: u64 = 1_209_600;

/// Session sliding idle timeout: 12 hours (`43,200 s`).
pub const IDLE_LIMIT_SECS: u64 = 43_200;

/// Session absolute lifetime cap: 7 days (`604,800 s`).
pub const ABSOLUTE_CAP_SECS: u64 = 604_800;

/// Activity renewal write-coalescing quantum: 15 minutes (`900 s`).
pub const ACTIVITY_QUANTUM_SECS: u64 = 900;

/// Canonical hosted anti-enumeration message returned for active, deactivated, and unknown emails.
pub const HOSTED_ANTI_ENUMERATION_MESSAGE: &str =
    "If that address has an account, a sign-in link is on its way.";

/// Normalized trimmed ASCII lowercase email address.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EmailAddress(String);

impl EmailAddress {
    /// Normalizes an input email (trim + ASCII lowercase) and rejects blank or invalid values.
    pub fn parse(raw: &str) -> Result<Self, AuthError> {
        let normalized = raw.trim().to_ascii_lowercase();
        if normalized.is_empty()
            || !normalized.contains('@')
            || normalized
                .chars()
                .any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(AuthError::BlankOrMalformedCredentials);
        }
        Ok(Self(normalized))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Account standing checked live on every verification and authenticated request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountStanding {
    Active,
    Deactivated,
}

/// Physician role inside a practice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhysicianRole {
    Standard,
    Admin,
}

/// Invited physician account persisted in practice authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicianAccount {
    pub physician_id: PhysicianId,
    pub practice_id: PracticeId,
    pub email: EmailAddress,
    pub display_name: String,
    pub role: PhysicianRole,
    pub standing: AccountStanding,
    /// Optional revocation fence: sessions created before this timestamp are rejected.
    pub sessions_ended_before: Option<u64>,
}

/// Sign-in initiation deployment mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInMode {
    Hosted,
    LocalProof,
}

/// Simulated outbound mail transport status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailTransportStatus {
    Healthy,
    DeliveryFailed,
}

/// Purpose and TTL class for a single-use verification token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationTokenKind {
    SignInLink,
    ColleagueInvite,
}

impl VerificationTokenKind {
    #[must_use]
    pub const fn ttl_secs(self) -> u64 {
        match self {
            Self::SignInLink => VERIFICATION_TOKEN_TTL_SECS,
            Self::ColleagueInvite => INVITATION_TOKEN_TTL_SECS,
        }
    }
}

/// Result of `POST /auth/sign-in`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SignInInitiationResponse {
    /// Hosted mode: identical payload for active, deactivated, and unknown emails.
    Unavailable {
        token: Option<String>,
        message: String,
    },
    /// Local proof mode only: exposes deterministic one-time token for local testing.
    TokenIssued { token: String, message: String },
}

/// Result of `POST /auth/verify` on success.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedSessionPayload {
    pub session: SessionId,
    pub physician_id: PhysicianId,
    pub practice_id: PracticeId,
}

/// Result of `GET /auth/whoami` on success.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhoAmIPayload {
    pub physician_id: PhysicianId,
    pub practice_id: PracticeId,
    pub email: EmailAddress,
    pub display_name: String,
    pub admin: bool,
    pub renewal: SessionActivityDecision,
}

/// Typed errors returned by authentication and session authority endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AuthError {
    #[error("sign-in email could not be delivered right now")]
    MailDeliveryFailed,
    #[error("blank or malformed credentials")]
    BlankOrMalformedCredentials,
    #[error("invalid session identifier: {0}")]
    InvalidSessionId(#[from] IdError),
    #[error("authentication refused ({reason:?}): {detail}", reason = .0.reason, detail = .0.detail)]
    Refused(StructuredRefusal),
}

impl AuthError {
    /// Maps this error to its required HTTP status code (`502`, `401`, or `403`).
    #[must_use]
    pub const fn http_status_code(&self) -> u16 {
        match self {
            Self::MailDeliveryFailed => 502,
            Self::BlankOrMalformedCredentials | Self::InvalidSessionId(_) => 401,
            Self::Refused(refusal) => refusal.reason.http_status_code(),
        }
    }

    /// Returns the structured refusal code when present.
    #[must_use]
    pub const fn refusal_code(&self) -> Option<RefusalCode> {
        match self {
            Self::Refused(refusal) => Some(refusal.reason),
            Self::MailDeliveryFailed
            | Self::BlankOrMalformedCredentials
            | Self::InvalidSessionId(_) => None,
        }
    }
}

/// Single-owner decision for session activity checks and sliding renewal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionActivityDecision {
    /// Active and within the 15-minute renewal quantum; no DB write required.
    Covered { idle_deadline: u64 },
    /// Active and idle deadline advanced (clamped to absolute cap / fixed end).
    Extended { new_idle_deadline: u64 },
    /// Expired (`now >= idle_deadline` or `now >= absolute_end`) or revoked.
    Ended,
}

/// Authoritative persisted session record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: SessionId,
    pub physician_id: PhysicianId,
    pub practice_id: PracticeId,
    pub created_at: u64,
    pub idle_deadline: u64,
    pub absolute_end: u64,
    pub revoked: bool,
}

impl SessionRecord {
    /// Mints a new session obeying `IDLE_LIMIT_SECS` (12h), `ABSOLUTE_CAP_SECS` (7d),
    /// and any explicit fixed hard end (`fixed_ends_at`).
    #[must_use]
    pub fn mint(
        id: SessionId,
        physician_id: PhysicianId,
        practice_id: PracticeId,
        created_at: u64,
        fixed_ends_at: Option<u64>,
    ) -> Self {
        let cap_end = created_at.saturating_add(ABSOLUTE_CAP_SECS);
        let absolute_end = match fixed_ends_at {
            Some(fixed) => fixed.min(cap_end),
            None => cap_end,
        };
        let idle_deadline = created_at.saturating_add(IDLE_LIMIT_SECS).min(absolute_end);
        Self {
            id,
            physician_id,
            practice_id,
            created_at,
            idle_deadline,
            absolute_end,
            revoked: false,
        }
    }

    /// Single owner for session validity and 15-minute sliding renewal.
    ///
    /// Enforces strict inequality: valid strictly while `now < idle_deadline` and
    /// `now < absolute_end`. Clamps persisted over-cap deadlines to `created_at + 7d`.
    pub fn evaluate_activity(&mut self, now_secs: u64) -> SessionActivityDecision {
        if self.revoked {
            return SessionActivityDecision::Ended;
        }
        let hard_cap = self.created_at.saturating_add(ABSOLUTE_CAP_SECS);
        let effective_abs_end = self.absolute_end.min(hard_cap);
        let effective_idle = self.idle_deadline.min(effective_abs_end);

        // Strict inequality: at `now == idle_deadline` the session is Ended.
        if now_secs >= effective_idle || now_secs >= effective_abs_end {
            return SessionActivityDecision::Ended;
        }

        let candidate_idle = now_secs
            .saturating_add(IDLE_LIMIT_SECS)
            .min(effective_abs_end);
        let needs_clamp_write = self.idle_deadline > effective_abs_end
            || self.absolute_end > hard_cap
            || (candidate_idle == effective_abs_end && effective_idle < effective_abs_end);

        if needs_clamp_write
            || candidate_idle >= effective_idle.saturating_add(ACTIVITY_QUANTUM_SECS)
        {
            self.absolute_end = effective_abs_end;
            self.idle_deadline = candidate_idle;
            SessionActivityDecision::Extended {
                new_idle_deadline: candidate_idle,
            }
        } else {
            SessionActivityDecision::Covered {
                idle_deadline: effective_idle,
            }
        }
    }

    /// Read-only liveness check at `now_secs` without mutating renewal state.
    #[must_use]
    pub fn is_live_at(&self, now_secs: u64) -> bool {
        if self.revoked {
            return false;
        }
        let hard_cap = self.created_at.saturating_add(ABSOLUTE_CAP_SECS);
        let effective_abs_end = self.absolute_end.min(hard_cap);
        let effective_idle = self.idle_deadline.min(effective_abs_end);
        now_secs < effective_idle && now_secs < effective_abs_end
    }
}

#[derive(Debug, Clone)]
struct VerificationTokenRecord {
    email: EmailAddress,
    expires_at: u64,
    consumed: bool,
}

/// Outbound mail delivery record captured for deterministic testing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundSignInMail {
    pub recipient: EmailAddress,
    pub token: String,
    pub expires_at: u64,
}

/// Revocation event broadcast to close SSE streams across all server instances.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityRevocationBroadcast {
    pub session_id: SessionId,
    pub revoked_at: u64,
}

/// Reference in-memory authentication & session authority implementing Chapter 1 rules.
#[derive(Debug, Clone)]
pub struct AuthAuthority {
    mode: SignInMode,
    mail_status: MailTransportStatus,
    accounts: BTreeMap<EmailAddress, PhysicianAccount>,
    tokens: BTreeMap<String, VerificationTokenRecord>,
    sessions: BTreeMap<SessionId, SessionRecord>,
    sent_mail: Vec<OutboundSignInMail>,
    revocation_broadcasts: Vec<AuthorityRevocationBroadcast>,
    next_token_seq: u64,
    next_session_seq: u64,
}

impl AuthAuthority {
    /// Creates a new auth authority in `Hosted` or `LocalProof` mode.
    #[must_use]
    pub fn new(mode: SignInMode) -> Self {
        Self {
            mode,
            mail_status: MailTransportStatus::Healthy,
            accounts: BTreeMap::new(),
            tokens: BTreeMap::new(),
            sessions: BTreeMap::new(),
            sent_mail: Vec::new(),
            revocation_broadcasts: Vec::new(),
            next_token_seq: 1,
            next_session_seq: 1,
        }
    }

    /// Configures simulated outbound mail transport status.
    pub fn set_mail_status(&mut self, status: MailTransportStatus) {
        self.mail_status = status;
    }

    /// Registers or updates an invited physician account.
    pub fn upsert_account(&mut self, account: PhysicianAccount) {
        self.accounts.insert(account.email.clone(), account);
    }

    /// Deactivates a physician account (persists across subsequent requests).
    pub fn deactivate_account(&mut self, email: &EmailAddress) {
        if let Some(account) = self.accounts.get_mut(email) {
            account.standing = AccountStanding::Deactivated;
        }
    }

    /// Sets `sessions_ended_before` fence on an account so earlier sessions fail immediately.
    pub fn set_sessions_ended_before(&mut self, email: &EmailAddress, fence_secs: u64) {
        if let Some(account) = self.accounts.get_mut(email) {
            account.sessions_ended_before = Some(fence_secs);
        }
    }

    /// `POST /auth/sign-in` implementing invite-only anti-enumeration semantics.
    pub fn initiate_sign_in(
        &mut self,
        raw_email: &str,
        kind: VerificationTokenKind,
        now_secs: u64,
    ) -> Result<SignInInitiationResponse, AuthError> {
        let email = EmailAddress::parse(raw_email)?;
        let active_account = self
            .accounts
            .get(&email)
            .filter(|acct| acct.standing == AccountStanding::Active);

        if active_account.is_none() {
            // Unknown or deactivated: send ZERO mail, return identical anti-enumeration response.
            return Ok(SignInInitiationResponse::Unavailable {
                token: None,
                message: HOSTED_ANTI_ENUMERATION_MESSAGE.to_owned(),
            });
        }

        let token = format!("tok-{}", self.next_token_seq);
        self.next_token_seq = self.next_token_seq.saturating_add(1);
        let expires_at = now_secs.saturating_add(kind.ttl_secs());

        match self.mode {
            SignInMode::Hosted => {
                if self.mail_status == MailTransportStatus::DeliveryFailed {
                    return Err(AuthError::MailDeliveryFailed);
                }
                self.tokens.insert(
                    token.clone(),
                    VerificationTokenRecord {
                        email: email.clone(),
                        expires_at,
                        consumed: false,
                    },
                );
                self.sent_mail.push(OutboundSignInMail {
                    recipient: email,
                    token,
                    expires_at,
                });
                Ok(SignInInitiationResponse::Unavailable {
                    token: None,
                    message: HOSTED_ANTI_ENUMERATION_MESSAGE.to_owned(),
                })
            }
            SignInMode::LocalProof => {
                self.tokens.insert(
                    token.clone(),
                    VerificationTokenRecord {
                        email,
                        expires_at,
                        consumed: false,
                    },
                );
                Ok(SignInInitiationResponse::TokenIssued {
                    token,
                    message: "Sign-in token issued.".to_owned(),
                })
            }
        }
    }

    /// `POST /auth/verify` with atomic single-use token consumption and session minting.
    pub fn verify_token(
        &mut self,
        raw_email: &str,
        raw_token: &str,
        now_secs: u64,
        fixed_ends_at: Option<u64>,
    ) -> Result<VerifiedSessionPayload, AuthError> {
        let email = EmailAddress::parse(raw_email)?;
        let token = raw_token.trim();
        if token.is_empty() {
            return Err(AuthError::BlankOrMalformedCredentials);
        }

        let Some(account) = self.accounts.get(&email).cloned() else {
            return Err(AuthError::Refused(StructuredRefusal::session_ended()));
        };

        if account.standing == AccountStanding::Deactivated {
            return Err(AuthError::Refused(StructuredRefusal::access_removed()));
        }

        let Some(record) = self.tokens.get_mut(token) else {
            return Err(AuthError::Refused(StructuredRefusal::session_ended()));
        };

        if record.email != email || record.consumed || now_secs >= record.expires_at {
            return Err(AuthError::Refused(StructuredRefusal::session_ended()));
        }

        // Atomic consumption + session minting.
        record.consumed = true;
        let session_id = SessionId::new(format!("sess-{}", self.next_session_seq))?;
        self.next_session_seq = self.next_session_seq.saturating_add(1);

        let session = SessionRecord::mint(
            session_id.clone(),
            account.physician_id.clone(),
            account.practice_id.clone(),
            now_secs,
            fixed_ends_at,
        );
        self.sessions.insert(session_id.clone(), session);

        Ok(VerifiedSessionPayload {
            session: session_id,
            physician_id: account.physician_id,
            practice_id: account.practice_id,
        })
    }

    /// `GET /auth/whoami`: checks live session & account standing and updates sliding idle renewal.
    pub fn whoami(
        &mut self,
        bearer: Option<&SessionId>,
        now_secs: u64,
    ) -> Result<WhoAmIPayload, AuthError> {
        let Some(session_id) = bearer else {
            return Err(AuthError::Refused(StructuredRefusal::no_session()));
        };
        let Some(session) = self.sessions.get_mut(session_id) else {
            return Err(AuthError::Refused(StructuredRefusal::session_ended()));
        };

        let physician_id = session.physician_id.clone();
        let created_at = session.created_at;

        let Some(account) = self
            .accounts
            .values()
            .find(|acct| acct.physician_id == physician_id)
            .cloned()
        else {
            return Err(AuthError::Refused(StructuredRefusal::access_removed()));
        };

        if account.standing == AccountStanding::Deactivated {
            return Err(AuthError::Refused(StructuredRefusal::access_removed()));
        }

        if let Some(fence) = account.sessions_ended_before
            && created_at < fence
        {
            return Err(AuthError::Refused(StructuredRefusal::session_ended()));
        }

        let decision = session.evaluate_activity(now_secs);
        if decision == SessionActivityDecision::Ended {
            return Err(AuthError::Refused(StructuredRefusal::session_ended()));
        }

        Ok(WhoAmIPayload {
            physician_id: account.physician_id,
            practice_id: account.practice_id,
            email: account.email,
            display_name: account.display_name,
            admin: account.role == PhysicianRole::Admin,
            renewal: decision,
        })
    }

    /// `POST /auth/sign-out`: revokes session immediately and broadcasts authority revocation.
    pub fn sign_out(&mut self, session_id: &SessionId, now_secs: u64) {
        if let Some(session) = self.sessions.get_mut(session_id) {
            session.revoked = true;
            self.revocation_broadcasts
                .push(AuthorityRevocationBroadcast {
                    session_id: session_id.clone(),
                    revoked_at: now_secs,
                });
        }
    }

    /// Forces a session to expire server-side immediately (for expiry-race testing).
    pub fn expire_session(&mut self, session_id: &SessionId, now_secs: u64) {
        if let Some(session) = self.sessions.get_mut(session_id) {
            session.idle_deadline = now_secs;
        }
    }

    /// Checks whether a session is currently live on the server at `now_secs`.
    #[must_use]
    pub fn is_session_live(&self, session_id: &SessionId, now_secs: u64) -> bool {
        let Some(session) = self.sessions.get(session_id) else {
            return false;
        };
        if !session.is_live_at(now_secs) {
            return false;
        }
        let Some(account) = self
            .accounts
            .values()
            .find(|acct| acct.physician_id == session.physician_id)
        else {
            return false;
        };
        if account.standing == AccountStanding::Deactivated {
            return false;
        }
        if let Some(fence) = account.sessions_ended_before
            && session.created_at < fence
        {
            return false;
        }
        true
    }

    /// Captured outbound sign-in emails (strictly active accounts only).
    #[must_use]
    pub fn sent_mail(&self) -> &[OutboundSignInMail] {
        &self.sent_mail
    }

    /// Captured authority revocation broadcasts for cross-instance SSE termination.
    #[must_use]
    pub fn revocation_broadcasts(&self) -> &[AuthorityRevocationBroadcast] {
        &self.revocation_broadcasts
    }

    /// Inspects a session record by ID.
    #[must_use]
    pub fn session(&self, id: &SessionId) -> Option<&SessionRecord> {
        self.sessions.get(id)
    }
}

/// Target route preserved across unauthenticated boot (`wants`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLinkTarget {
    Today,
    LinkedPatient(WorkspaceId),
}

/// Active view shown inside an authenticated `Workspace` (`opens`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceOpenView {
    Today,
    LinkedPatient(WorkspaceId),
}

/// Tab view state machine from Chapter 1 §1.4.
///
/// Encoding held session and open view directly inside enum variants prevents
/// invalid states such as `SessionEndedPrompt` holding patient content or
/// `Workspace` lacking a concrete session token (`SignedIn`, `PromptIsEmpty`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabViewState {
    Opening {
        wants: DeepLinkTarget,
    },
    SignInPage {
        wants: DeepLinkTarget,
    },
    SignInLink {
        prior_held: Option<SessionId>,
    },
    Workspace {
        holds: SessionId,
        opens: WorkspaceOpenView,
        sse_connected: bool,
    },
    SessionEndedPrompt,
}

impl TabViewState {
    /// Returns the session currently held by this tab, if any.
    #[must_use]
    pub fn held_session(&self) -> Option<&SessionId> {
        match self {
            Self::Workspace { holds, .. } => Some(holds),
            Self::SignInLink { prior_held } => prior_held.as_ref(),
            Self::Opening { .. } | Self::SignInPage { .. } | Self::SessionEndedPrompt => None,
        }
    }

    /// Returns true if any protected patient content is currently rendered or retained.
    #[must_use]
    pub const fn shows_protected_content(&self) -> bool {
        matches!(self, Self::Workspace { .. })
    }
}

/// Cross-tab broadcast event announced over `BroadcastChannel` / `storage` events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrossTabSessionNotice {
    pub origin_tab: TabId,
    pub stored_session: Option<SessionId>,
}

/// Multi-tab browser coordinator modeling shared storage, peer notifications, and server checks.
#[derive(Debug, Clone)]
pub struct MultiTabBrowser {
    shared_storage: Option<SessionId>,
    tabs: BTreeMap<TabId, TabViewState>,
    pending_notices: VecDeque<CrossTabSessionNotice>,
    ever_signed_out_sessions: Vec<SessionId>,
}

impl MultiTabBrowser {
    /// Creates an empty browser profile with optional pre-existing stored session.
    #[must_use]
    pub fn new(initial_storage: Option<SessionId>) -> Self {
        Self {
            shared_storage: initial_storage,
            tabs: BTreeMap::new(),
            pending_notices: VecDeque::new(),
            ever_signed_out_sessions: Vec::new(),
        }
    }

    /// Opens a new tab in `Opening` with an optional `?patient=` deep-link target.
    pub fn open_tab(&mut self, tab_id: TabId, wants: DeepLinkTarget) {
        self.tabs.insert(tab_id, TabViewState::Opening { wants });
    }

    /// Starts verifying an emailed sign-in link in `tab_id` (always resets `wants` to `Today`).
    pub fn begin_email_sign_in_link(&mut self, tab_id: &TabId) {
        if let Some(state) = self.tabs.get_mut(tab_id) {
            let prior_held = state.held_session().cloned();
            *state = TabViewState::SignInLink { prior_held };
        }
    }

    /// Completes `Opening` boot verification against `AuthAuthority` (`Lands` / `SignedIn` / `NoDeadlock`).
    pub fn resolve_opening(
        &mut self,
        tab_id: &TabId,
        authority: &mut AuthAuthority,
        now_secs: u64,
    ) {
        let Some(TabViewState::Opening { wants }) = self.tabs.get(tab_id).cloned() else {
            return;
        };
        let Some(stored) = self.shared_storage.clone() else {
            self.tabs
                .insert(tab_id.clone(), TabViewState::SignInPage { wants });
            return;
        };

        if authority.whoami(Some(&stored), now_secs).is_ok() {
            let opens = match wants {
                DeepLinkTarget::Today => WorkspaceOpenView::Today,
                DeepLinkTarget::LinkedPatient(ws) => WorkspaceOpenView::LinkedPatient(ws),
            };
            self.tabs.insert(
                tab_id.clone(),
                TabViewState::Workspace {
                    holds: stored,
                    opens,
                    sse_connected: true,
                },
            );
        } else {
            self.shared_storage = None;
            self.tabs
                .insert(tab_id.clone(), TabViewState::SignInPage { wants });
        }
    }

    /// Completes sign-in (direct or emailed link) in `tab_id`, updates shared storage,
    /// and enqueues a cross-tab broadcast notice.
    ///
    /// - From `SignInLink` or `SessionEndedPrompt`: always lands on `Today` (`TodayFirst`).
    /// - From `Opening` / `SignInPage` with a deep link: lands on `LinkedPatient` (`Lands`).
    pub fn complete_sign_in(&mut self, tab_id: &TabId, new_session: SessionId) {
        self.shared_storage = Some(new_session.clone());
        if let Some(state) = self.tabs.get_mut(tab_id) {
            let opens = match state {
                TabViewState::Opening {
                    wants: DeepLinkTarget::LinkedPatient(ws),
                }
                | TabViewState::SignInPage {
                    wants: DeepLinkTarget::LinkedPatient(ws),
                } => WorkspaceOpenView::LinkedPatient(ws.clone()),
                TabViewState::Opening {
                    wants: DeepLinkTarget::Today,
                }
                | TabViewState::SignInPage {
                    wants: DeepLinkTarget::Today,
                }
                | TabViewState::SignInLink { .. }
                | TabViewState::SessionEndedPrompt
                | TabViewState::Workspace { .. } => WorkspaceOpenView::Today,
            };
            *state = TabViewState::Workspace {
                holds: new_session.clone(),
                opens,
                sse_connected: true,
            };
        }

        self.pending_notices.push_back(CrossTabSessionNotice {
            origin_tab: tab_id.clone(),
            stored_session: Some(new_session),
        });
    }

    /// Handles an HTTP `401 session_ended` refusal from an in-flight request carrying `failed_session`.
    ///
    /// Enforces `Expiry`, `PromptIsEmpty`, and `KeepsWork`:
    /// - If shared storage already holds a newer live session (`KeepsWork`), adopts the newer
    ///   live session and lands on `Today` (`TodayFirst`) instead of kicking the tab out.
    /// - Otherwise clears shared storage (if matching) and transitions `Workspace` -> `SessionEndedPrompt`
    ///   (zero patient content, zero held session) or `SignInLink` -> `SignInPage` (`NoDeadlock`).
    pub fn handle_session_ended_response(
        &mut self,
        tab_id: &TabId,
        failed_session: &SessionId,
        authority: &mut AuthAuthority,
        now_secs: u64,
    ) {
        if let Some(stored) = self.shared_storage.clone()
            && &stored != failed_session
            && authority.whoami(Some(&stored), now_secs).is_ok()
        {
            if let Some(state) = self.tabs.get_mut(tab_id) {
                let opens = match state {
                    TabViewState::Opening {
                        wants: DeepLinkTarget::LinkedPatient(ws),
                    }
                    | TabViewState::SignInPage {
                        wants: DeepLinkTarget::LinkedPatient(ws),
                    } => WorkspaceOpenView::LinkedPatient(ws.clone()),
                    _ => WorkspaceOpenView::Today,
                };
                *state = TabViewState::Workspace {
                    holds: stored,
                    opens,
                    sse_connected: true,
                };
            }
            return;
        }

        if self.shared_storage.as_ref() == Some(failed_session) {
            self.shared_storage = None;
            self.pending_notices.push_back(CrossTabSessionNotice {
                origin_tab: tab_id.clone(),
                stored_session: None,
            });
        }

        if let Some(state) = self.tabs.get_mut(tab_id) {
            *state = match state {
                TabViewState::Workspace { .. } => TabViewState::SessionEndedPrompt,
                TabViewState::Opening { wants } | TabViewState::SignInPage { wants } => {
                    TabViewState::SignInPage {
                        wants: wants.clone(),
                    }
                }
                TabViewState::SignInLink { .. } => TabViewState::SignInPage {
                    wants: DeepLinkTarget::Today,
                },
                TabViewState::SessionEndedPrompt => TabViewState::SessionEndedPrompt,
            };
        }
    }

    /// Executes explicit sign-out from `tab_id` (`OnceOut` + `ServerRefusesSignedOut`).
    ///
    /// Terminates BOTH the shared stored session AND the tab's own held session on the server
    /// (fallback sign-out even if another tab already cleared shared storage).
    pub fn sign_out(&mut self, tab_id: &TabId, authority: &mut AuthAuthority, now_secs: u64) {
        if let Some(stored) = self.shared_storage.take() {
            authority.sign_out(&stored, now_secs);
            self.ever_signed_out_sessions.push(stored);
        }
        if let Some(held) = self
            .tabs
            .get(tab_id)
            .and_then(TabViewState::held_session)
            .cloned()
        {
            authority.sign_out(&held, now_secs);
            if !self.ever_signed_out_sessions.contains(&held) {
                self.ever_signed_out_sessions.push(held);
            }
        }
        if let Some(state) = self.tabs.get_mut(tab_id) {
            *state = TabViewState::SignInPage {
                wants: DeepLinkTarget::Today,
            };
        }
        self.pending_notices.push_back(CrossTabSessionNotice {
            origin_tab: tab_id.clone(),
            stored_session: None,
        });
    }

    /// Delivers all queued cross-tab notices and reconciles every tab (`CatchesUp`, `OnceOut`, `Lands`).
    pub fn deliver_all_peer_notices(&mut self, authority: &mut AuthAuthority, now_secs: u64) {
        while let Some(notice) = self.pending_notices.pop_front() {
            let tab_ids: Vec<TabId> = self.tabs.keys().cloned().collect();
            for tab_id in tab_ids {
                if tab_id == notice.origin_tab {
                    continue;
                }
                self.reconcile_tab(&tab_id, authority, now_secs);
            }
        }
    }

    /// Reconciles one tab against current shared storage (focus/visibility/storage event).
    pub fn reconcile_tab(&mut self, tab_id: &TabId, authority: &mut AuthAuthority, now_secs: u64) {
        let Some(current) = self.tabs.get(tab_id).cloned() else {
            return;
        };
        let next = match (current, self.shared_storage.clone()) {
            (
                TabViewState::Opening { wants } | TabViewState::SignInPage { wants },
                Some(stored),
            ) if authority.whoami(Some(&stored), now_secs).is_ok() => {
                let opens = match wants {
                    DeepLinkTarget::Today => WorkspaceOpenView::Today,
                    DeepLinkTarget::LinkedPatient(ws) => WorkspaceOpenView::LinkedPatient(ws),
                };
                TabViewState::Workspace {
                    holds: stored,
                    opens,
                    sse_connected: true,
                }
            }
            (TabViewState::SessionEndedPrompt, Some(stored))
                if authority.whoami(Some(&stored), now_secs).is_ok() =>
            {
                TabViewState::Workspace {
                    holds: stored,
                    opens: WorkspaceOpenView::Today,
                    sse_connected: true,
                }
            }
            (TabViewState::Workspace { holds, opens, .. }, Some(stored)) => {
                if holds == stored && authority.is_session_live(&holds, now_secs) {
                    TabViewState::Workspace {
                        holds,
                        opens,
                        sse_connected: true,
                    }
                } else if authority.whoami(Some(&stored), now_secs).is_ok() {
                    TabViewState::Workspace {
                        holds: stored,
                        opens: WorkspaceOpenView::Today,
                        sse_connected: true,
                    }
                } else {
                    TabViewState::SessionEndedPrompt
                }
            }
            (TabViewState::Workspace { .. }, None) => TabViewState::SignInPage {
                wants: DeepLinkTarget::Today,
            },
            (TabViewState::Opening { wants }, None) => TabViewState::SignInPage { wants },
            (other, _) => other,
        };
        self.tabs.insert(tab_id.clone(), next);
    }

    /// Attempts a clinical read/mutation from `tab_id` (`ActsAsShown`).
    ///
    /// Executes ONLY when the tab is in `Workspace`, holds a session matching shared
    /// storage, and the session is live on `authority`.
    pub fn attempt_workspace_action(
        &self,
        tab_id: &TabId,
        authority: &AuthAuthority,
        now_secs: u64,
    ) -> Result<SessionId, AuthError> {
        let Some(TabViewState::Workspace { holds, .. }) = self.tabs.get(tab_id) else {
            return Err(AuthError::Refused(StructuredRefusal::no_session()));
        };
        if self.shared_storage.as_ref() != Some(holds)
            || !authority.is_session_live(holds, now_secs)
        {
            return Err(AuthError::Refused(StructuredRefusal::session_ended()));
        }
        Ok(holds.clone())
    }

    /// Current shared browser storage session.
    #[must_use]
    pub fn shared_storage(&self) -> Option<&SessionId> {
        self.shared_storage.as_ref()
    }

    /// Inspects a tab's current view state.
    #[must_use]
    pub fn tab_state(&self, tab_id: &TabId) -> Option<&TabViewState> {
        self.tabs.get(tab_id)
    }

    /// Verifies static & post-convergence safety invariants across all tabs and the server:
    /// `OnceOut`, `SignedIn`, `ActsAsShown`, `PromptIsEmpty`, `ServerRefusesSignedOut`, `NoDeadlock`.
    pub fn check_safety_invariants(
        &self,
        authority: &AuthAuthority,
        now_secs: u64,
    ) -> Result<(), String> {
        // ServerRefusesSignedOut: every signed-out session is rejected by authority.
        for dead in &self.ever_signed_out_sessions {
            if authority.is_session_live(dead, now_secs) {
                return Err(format!(
                    "ServerRefusesSignedOut violated: session {} is still live",
                    dead.as_str()
                ));
            }
        }

        let live_stored = self
            .shared_storage
            .as_ref()
            .filter(|s| authority.is_session_live(s, now_secs));

        for (tab_id, state) in &self.tabs {
            match state {
                TabViewState::Workspace { holds, .. } => {
                    // OnceOut + SignedIn + ActsAsShown.
                    if live_stored.is_none() {
                        return Err(format!(
                            "OnceOut violated: tab {} in Workspace while storage has no live session",
                            tab_id.as_str()
                        ));
                    }
                    if !authority.is_session_live(holds, now_secs) {
                        return Err(format!(
                            "SignedIn violated: tab {} holds dead session {}",
                            tab_id.as_str(),
                            holds.as_str()
                        ));
                    }
                    if self.shared_storage.as_ref() != Some(holds) {
                        return Err(format!(
                            "ActsAsShown violated: tab {} holds {} != stored {:?}",
                            tab_id.as_str(),
                            holds.as_str(),
                            self.shared_storage
                        ));
                    }
                }
                TabViewState::SessionEndedPrompt => {
                    // PromptIsEmpty guaranteed structurally; check SignedIn when live storage exists.
                    if state.shows_protected_content() || state.held_session().is_some() {
                        return Err("PromptIsEmpty violated".to_owned());
                    }
                    if live_stored.is_some() {
                        return Err(format!(
                            "SignedIn violated: tab {} stuck on SessionEndedPrompt with live storage",
                            tab_id.as_str()
                        ));
                    }
                }
                TabViewState::SignInPage { .. } => {
                    if live_stored.is_some() {
                        return Err(format!(
                            "SignedIn violated: tab {} stuck on SignInPage with live storage",
                            tab_id.as_str()
                        ));
                    }
                }
                TabViewState::Opening { .. } | TabViewState::SignInLink { .. } => {
                    return Err(format!(
                        "NoDeadlock violated: tab {} stalled in transient state {:?}",
                        tab_id.as_str(),
                        state
                    ));
                }
            }
        }
        Ok(())
    }
}
