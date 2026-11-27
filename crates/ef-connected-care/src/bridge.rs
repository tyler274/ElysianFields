//! Single-owner Connected Care state machine enforcing all 11 formal
//! `Invitation` invariants, multi-practice independence, consent-gated
//! transfers, disconnection, rejoining, and idempotent account deletion.

use std::collections::{BTreeMap, HashMap, HashSet};

use ef_core::{IdempotencyKey, PhysicianId};
use serde::{Deserialize, Serialize};

use crate::envelope::{
    TransferEnvelope, TransferStatus, TransferSubmission, validate_transfer_consent,
};
use crate::error::{BridgeError, InvariantViolation};
use crate::types::{
    AccountIncarnation, ConnectedCareState, ConnectionId, ConnectionStanding, ConsentRecordId,
    DateOfBirth, DisconnectActor, DobVerificationOutcome, InvitationId, InviteChallenge,
    InviteCredential, InviteShortCode, InviteToken, MailDeliveryOutcome, MobileAccountId, PairKey,
    PracticePreview, TransferId,
};

/// Maximum wrong date-of-birth attempts before permanent invitation lockout (`LockedAfterFiveWrongDob`).
pub const MAX_WRONG_DOB_ATTEMPTS: u8 = 5;

/// Request issued by an authorized Curie Clinic practice member to invite a patient.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateInvitationRequest {
    pub pair: PairKey,
    pub issued_by: PhysicianId,
    pub practice_name: String,
    pub physician_name: String,
    pub expected_dob: DateOfBirth,
    pub created_at: String,
}

/// Issued invitation credentials returned for outbound email delivery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssuedInvitation {
    pub invitation_id: InvitationId,
    pub link_token: InviteToken,
    pub short_code: InviteShortCode,
}

/// Metadata-only audit event (zero clinical prose, zero DOB values, zero tokens).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub sequence: u64,
    pub occurred_at: String,
    pub kind: AuditEventKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditEventKind {
    InvitationCreated {
        pair: PairKey,
        invitation_id: InvitationId,
        issued_by: PhysicianId,
    },
    InvitationResent {
        pair: PairKey,
        invitation_id: InvitationId,
        resend_count: u32,
    },
    MailDeliveryRecorded {
        invitation_id: InvitationId,
        delivered: bool,
    },
    DobAttemptRejected {
        invitation_id: InvitationId,
        attempts: u8,
        locked: bool,
    },
    DobVerified {
        invitation_id: InvitationId,
        account_id: MobileAccountId,
    },
    PreviewExitedWithoutShare {
        invitation_id: InvitationId,
        account_id: MobileAccountId,
    },
    InvitationCancelled {
        pair: PairKey,
        invitation_id: InvitationId,
    },
    ConnectionEstablished {
        pair: PairKey,
        invitation_id: InvitationId,
        connection_id: ConnectionId,
        account_id: MobileAccountId,
    },
    TransferStaged {
        connection_id: ConnectionId,
        transfer_id: TransferId,
        item_count: usize,
        part2_consent_id: Option<ConsentRecordId>,
    },
    TransferRejectedPart2 {
        connection_id: ConnectionId,
        sender_account: MobileAccountId,
    },
    TransferMarkedUncertain {
        connection_id: ConnectionId,
        transfer_id: TransferId,
    },
    TransferDelivered {
        connection_id: ConnectionId,
        transfer_id: TransferId,
    },
    TransferHaltedOnDisconnect {
        connection_id: ConnectionId,
        transfer_id: TransferId,
    },
    ConnectionEnded {
        pair: PairKey,
        connection_id: ConnectionId,
        actor: DisconnectActor,
    },
    AccountDeletionReplayed {
        account_id: MobileAccountId,
        account_incarnation: AccountIncarnation,
        connections_ended: usize,
        invitations_cancelled: usize,
        transfers_scrubbed: usize,
    },
}

/// Summary returned by [`ConnectedCareBridge::terminate_account_links`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountTerminationSummary {
    pub connections_ended: usize,
    pub invitations_cancelled: usize,
    pub transfers_scrubbed: usize,
}

/// Verification summary returned by [`ConnectedCareBridge::audit_all_invariants`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvariantAuditReport {
    pub tracked_pairs: usize,
    pub open_invitations: usize,
    pub active_connections: usize,
    pub audited_transfers: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum InvitationStatus {
    OpenUnlocked,
    OpenLocked,
    Cancelled,
    Consumed { connection_id: ConnectionId },
}

#[derive(Debug, Clone)]
struct InvitationRecord {
    id: InvitationId,
    pair: PairKey,
    practice_name: String,
    physician_name: String,
    expected_dob: DateOfBirth,
    status: InvitationStatus,
    wrong_dob_attempts: u8,
    verified_accounts: HashSet<MobileAccountId>,
    link_tokens: Vec<InviteToken>,
    short_codes: Vec<InviteShortCode>,
    resend_count: u32,
    mail_history: Vec<MailDeliveryOutcome>,
}

impl InvitationRecord {
    fn is_open(&self) -> bool {
        matches!(
            self.status,
            InvitationStatus::OpenUnlocked | InvitationStatus::OpenLocked
        )
    }

    fn build_preview(&self) -> PracticePreview {
        PracticePreview {
            invitation_id: self.id.clone(),
            practice_id: self.pair.practice_id.clone(),
            practice_name: self.practice_name.clone(),
            physician_name: self.physician_name.clone(),
            sharing_disclosure: format!(
                "Connect your Curie Mobile account with {} ({}) to share curated items you explicitly choose to send.",
                self.practice_name, self.physician_name
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConnectionStatus {
    Connected,
    Ended { ended_at: String },
}

#[derive(Debug, Clone)]
struct ConnectionRecord {
    id: ConnectionId,
    pair: PairKey,
    account_id: MobileAccountId,
    status: ConnectionStatus,
}

#[derive(Debug, Clone, Default)]
struct PairRecord {
    open_invitation: Option<InvitationId>,
    active_connection: Option<ConnectionId>,
    has_prior_ended_connection: bool,
}

impl PairRecord {
    fn standing(
        &self,
        invitations: &HashMap<InvitationId, InvitationRecord>,
    ) -> ConnectedCareState {
        if self.active_connection.is_some() {
            return ConnectedCareState::Connected;
        }
        if let Some(inv_id) = &self.open_invitation
            && let Some(inv) = invitations.get(inv_id)
        {
            return match inv.status {
                InvitationStatus::OpenUnlocked => ConnectedCareState::Invited,
                InvitationStatus::OpenLocked => ConnectedCareState::InvitedLocked,
                InvitationStatus::Cancelled | InvitationStatus::Consumed { .. } => {
                    if self.has_prior_ended_connection {
                        ConnectedCareState::Ended
                    } else {
                        ConnectedCareState::NotInvited
                    }
                }
            };
        }
        if self.has_prior_ended_connection {
            ConnectedCareState::Ended
        } else {
            ConnectedCareState::NotInvited
        }
    }
}

/// Single-owner Connected Care bridge service/store shared conceptually as the
/// sole connection authority read by both Curie Clinic and Curie Mobile.
#[derive(Debug, Default)]
pub struct ConnectedCareBridge {
    next_seq: u64,
    pairs: BTreeMap<PairKey, PairRecord>,
    invitations: HashMap<InvitationId, InvitationRecord>,
    token_index: HashMap<InviteToken, InvitationId>,
    code_index: HashMap<InviteShortCode, InvitationId>,
    connections: HashMap<ConnectionId, ConnectionRecord>,
    transfers: HashMap<TransferId, TransferEnvelope>,
    transfer_idempotency: HashMap<(ConnectionId, IdempotencyKey), TransferId>,
    deleted_accounts: HashMap<MobileAccountId, AccountIncarnation>,
    audit_log: Vec<AuditEvent>,
}

impl ConnectedCareBridge {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Single-owner connection standing read by Curie Clinic (`SameConnectionState`).
    #[must_use]
    pub fn clinic_standing(&self, pair: &PairKey) -> ConnectionStanding {
        self.standing_for_pair(pair)
    }

    /// Single-owner connection standing read by Curie Mobile (`SameConnectionState`).
    #[must_use]
    pub fn mobile_standing(&self, pair: &PairKey) -> ConnectionStanding {
        self.standing_for_pair(pair)
    }

    /// Lists all active connections for a standalone Curie Mobile account across practices.
    #[must_use]
    pub fn active_connections_for_account(
        &self,
        account_id: &MobileAccountId,
    ) -> Vec<ConnectionStanding> {
        self.connections
            .values()
            .filter(|conn| {
                &conn.account_id == account_id && matches!(conn.status, ConnectionStatus::Connected)
            })
            .map(|conn| self.standing_for_pair(&conn.pair))
            .collect()
    }

    /// Issues a single open invitation for `(practice_id, practice_patient_id)` (`AtMostOneOpen`).
    pub fn create_invitation(
        &mut self,
        request: CreateInvitationRequest,
    ) -> Result<IssuedInvitation, BridgeError> {
        if request.practice_name.trim().is_empty() {
            return Err(BridgeError::EmptyField {
                field: "practice_name",
            });
        }
        if request.physician_name.trim().is_empty() {
            return Err(BridgeError::EmptyField {
                field: "physician_name",
            });
        }

        let current_state = self.standing_for_pair(&request.pair).state;
        if !matches!(
            current_state,
            ConnectedCareState::NotInvited | ConnectedCareState::Ended
        ) {
            return Err(BridgeError::CannotInviteInState {
                pair: request.pair,
                state: current_state,
            });
        }

        let seq = self.next_id();
        let invitation_id = InvitationId::new(format!("inv-{seq}"))?;
        let link_token = InviteToken::new(format!("tok-{seq}-{}", request.pair.practice_id))?;
        let short_code = InviteShortCode::new(format!("EF-{seq:06}"))?;

        let record = InvitationRecord {
            id: invitation_id.clone(),
            pair: request.pair.clone(),
            practice_name: request.practice_name,
            physician_name: request.physician_name,
            expected_dob: request.expected_dob,
            status: InvitationStatus::OpenUnlocked,
            wrong_dob_attempts: 0,
            verified_accounts: HashSet::new(),
            link_tokens: vec![link_token.clone()],
            short_codes: vec![short_code.clone()],
            resend_count: 0,
            mail_history: Vec::new(),
        };

        let _ = self.invitations.insert(invitation_id.clone(), record);
        let _ = self
            .token_index
            .insert(link_token.clone(), invitation_id.clone());
        let _ = self
            .code_index
            .insert(short_code.clone(), invitation_id.clone());

        let pair_entry = self.pairs.entry(request.pair.clone()).or_default();
        pair_entry.open_invitation = Some(invitation_id.clone());

        self.push_audit(
            request.created_at,
            AuditEventKind::InvitationCreated {
                pair: request.pair,
                invitation_id: invitation_id.clone(),
                issued_by: request.issued_by,
            },
        );

        Ok(IssuedInvitation {
            invitation_id,
            link_token,
            short_code,
        })
    }

    /// Resends an open **unlocked** invitation without creating a second invite,
    /// unlocking a locked invite, or invalidating previously emailed links/codes
    /// (`ResendNeverSplitsOrUnlocks`, `AtMostOneOpen`).
    pub fn resend_invitation(
        &mut self,
        pair: &PairKey,
        resent_at: impl Into<String>,
    ) -> Result<IssuedInvitation, BridgeError> {
        let inv_id = self
            .pairs
            .get(pair)
            .and_then(|p| p.open_invitation.clone())
            .ok_or_else(|| BridgeError::NoOpenInvitation { pair: pair.clone() })?;

        let status = self
            .invitations
            .get(&inv_id)
            .map(|inv| inv.status.clone())
            .ok_or_else(|| BridgeError::NoOpenInvitation { pair: pair.clone() })?;

        match status {
            InvitationStatus::OpenUnlocked => {}
            InvitationStatus::OpenLocked => {
                return Err(BridgeError::InvitationLocked {
                    invitation_id: inv_id,
                });
            }
            InvitationStatus::Cancelled => {
                return Err(BridgeError::InvitationCancelled {
                    invitation_id: inv_id,
                });
            }
            InvitationStatus::Consumed { .. } => {
                return Err(BridgeError::NoOpenInvitation { pair: pair.clone() });
            }
        }

        let seq = self.next_id();
        let new_token = InviteToken::new(format!("tok-{seq}-{}", pair.practice_id))?;
        let new_code = InviteShortCode::new(format!("EF-{seq:06}"))?;

        let resend_count = {
            let inv = self
                .invitations
                .get_mut(&inv_id)
                .ok_or_else(|| BridgeError::NoOpenInvitation { pair: pair.clone() })?;
            inv.resend_count += 1;
            inv.link_tokens.push(new_token.clone());
            inv.short_codes.push(new_code.clone());
            inv.resend_count
        };

        let _ = self.token_index.insert(new_token.clone(), inv_id.clone());
        let _ = self.code_index.insert(new_code.clone(), inv_id.clone());

        self.push_audit(
            resent_at.into(),
            AuditEventKind::InvitationResent {
                pair: pair.clone(),
                invitation_id: inv_id.clone(),
                resend_count,
            },
        );

        Ok(IssuedInvitation {
            invitation_id: inv_id,
            link_token: new_token,
            short_code: new_code,
        })
    }

    /// Records outbound email delivery success or failure. Delivery failure
    /// leaves the invitation open (`FailedMailLeavesOpen`).
    pub fn record_mail_delivery(
        &mut self,
        invitation_id: &InvitationId,
        outcome: MailDeliveryOutcome,
    ) -> Result<(), BridgeError> {
        let (delivered, timestamp) = match &outcome {
            MailDeliveryOutcome::Delivered { delivered_at } => (true, delivered_at.clone()),
            MailDeliveryOutcome::Failed { failed_at, .. } => (false, failed_at.clone()),
        };

        let inv = self.invitations.get_mut(invitation_id).ok_or_else(|| {
            BridgeError::InvalidOrRevokedCredential {
                invitation_id: invitation_id.as_str().to_owned(),
            }
        })?;
        inv.mail_history.push(outcome);

        self.push_audit(
            timestamp,
            AuditEventKind::MailDeliveryRecorded {
                invitation_id: invitation_id.clone(),
                delivered,
            },
        );
        Ok(())
    }

    /// Cancels an open invitation (unlocked OR locked), immediately invalidating
    /// every previously mailed link token and short code (`CancelledNeverConnects`).
    pub fn cancel_invitation(
        &mut self,
        pair: &PairKey,
        cancelled_at: impl Into<String>,
    ) -> Result<InvitationId, BridgeError> {
        let inv_id = self
            .pairs
            .get(pair)
            .and_then(|p| p.open_invitation.clone())
            .ok_or_else(|| BridgeError::NoOpenInvitation { pair: pair.clone() })?;

        let (tokens, codes) = {
            let inv = self
                .invitations
                .get_mut(&inv_id)
                .ok_or_else(|| BridgeError::NoOpenInvitation { pair: pair.clone() })?;
            if !inv.is_open() {
                return Err(BridgeError::NoOpenInvitation { pair: pair.clone() });
            }
            inv.status = InvitationStatus::Cancelled;
            inv.verified_accounts.clear();
            (inv.link_tokens.clone(), inv.short_codes.clone())
        };

        for tok in tokens {
            let _ = self.token_index.remove(&tok);
        }
        for code in codes {
            let _ = self.code_index.remove(&code);
        }

        if let Some(pair_record) = self.pairs.get_mut(pair) {
            pair_record.open_invitation = None;
        }

        self.push_audit(
            cancelled_at.into(),
            AuditEventKind::InvitationCancelled {
                pair: pair.clone(),
                invitation_id: inv_id.clone(),
            },
        );

        Ok(inv_id)
    }

    /// Looks up an invitation by link token or short code BEFORE date-of-birth
    /// verification. Returns zero practice or physician details (`PreviewOnlyAfterDob`).
    pub fn lookup_challenge(
        &self,
        credential: &InviteCredential,
    ) -> Result<InviteChallenge, BridgeError> {
        let inv = self.resolve_active_invitation(credential)?;
        match inv.status {
            InvitationStatus::OpenUnlocked => Ok(InviteChallenge {
                invitation_id: inv.id.clone(),
                remaining_attempts: MAX_WRONG_DOB_ATTEMPTS.saturating_sub(inv.wrong_dob_attempts),
            }),
            InvitationStatus::OpenLocked => Err(BridgeError::InvitationLocked {
                invitation_id: inv.id.clone(),
            }),
            InvitationStatus::Cancelled => Err(BridgeError::InvitationCancelled {
                invitation_id: inv.id.clone(),
            }),
            InvitationStatus::Consumed { .. } => Err(BridgeError::InvalidOrRevokedCredential {
                invitation_id: inv.id.as_str().to_owned(),
            }),
        }
    }

    /// Verifies the patient's date of birth against the practice's invited record.
    ///
    /// - Grants ZERO sharing consent (`OnlyShareGrantsConsent`).
    /// - Reveals `PracticePreview` ONLY upon exact DOB match (`DobVerifiedOnlyAfterMatch`).
    /// - Locks permanently on the 5th wrong guess (`LockedAfterFiveWrongDob`).
    pub fn verify_dob(
        &mut self,
        credential: &InviteCredential,
        account_id: &MobileAccountId,
        dob_guess: DateOfBirth,
        attempted_at: impl Into<String>,
    ) -> Result<DobVerificationOutcome, BridgeError> {
        self.ensure_account_not_deleted(account_id)?;
        let inv_id = self.resolve_active_invitation(credential)?.id.clone();

        let (status, expected_dob) = {
            let inv = self.invitations.get(&inv_id).ok_or_else(|| {
                BridgeError::InvalidOrRevokedCredential {
                    invitation_id: inv_id.as_str().to_owned(),
                }
            })?;
            (inv.status.clone(), inv.expected_dob)
        };

        match status {
            InvitationStatus::OpenUnlocked => {}
            InvitationStatus::OpenLocked => {
                return Err(BridgeError::InvitationLocked {
                    invitation_id: inv_id,
                });
            }
            InvitationStatus::Cancelled => {
                return Err(BridgeError::InvitationCancelled {
                    invitation_id: inv_id,
                });
            }
            InvitationStatus::Consumed { .. } => {
                return Err(BridgeError::InvalidOrRevokedCredential {
                    invitation_id: inv_id.as_str().to_owned(),
                });
            }
        }

        if dob_guess == expected_dob {
            let preview = {
                let inv = self.invitations.get_mut(&inv_id).ok_or_else(|| {
                    BridgeError::InvalidOrRevokedCredential {
                        invitation_id: inv_id.as_str().to_owned(),
                    }
                })?;
                let _ = inv.verified_accounts.insert(account_id.clone());
                inv.build_preview()
            };
            self.push_audit(
                attempted_at.into(),
                AuditEventKind::DobVerified {
                    invitation_id: inv_id,
                    account_id: account_id.clone(),
                },
            );
            return Ok(DobVerificationOutcome::Verified { preview });
        }

        let (attempts, locked_now) = {
            let inv = self.invitations.get_mut(&inv_id).ok_or_else(|| {
                BridgeError::InvalidOrRevokedCredential {
                    invitation_id: inv_id.as_str().to_owned(),
                }
            })?;
            inv.wrong_dob_attempts = inv.wrong_dob_attempts.saturating_add(1);
            let locked = inv.wrong_dob_attempts >= MAX_WRONG_DOB_ATTEMPTS;
            if locked {
                inv.status = InvitationStatus::OpenLocked;
                inv.verified_accounts.clear();
            }
            (inv.wrong_dob_attempts, locked)
        };

        self.push_audit(
            attempted_at.into(),
            AuditEventKind::DobAttemptRejected {
                invitation_id: inv_id,
                attempts,
                locked: locked_now,
            },
        );

        if locked_now {
            Ok(DobVerificationOutcome::LockedOut)
        } else {
            Ok(DobVerificationOutcome::Mismatch {
                remaining_attempts: MAX_WRONG_DOB_ATTEMPTS.saturating_sub(attempts),
            })
        }
    }

    /// Records the patient leaving the preview screen without tapping **Share**.
    /// Keeps the invitation open and grants zero sharing consent (`OnlyShareGrantsConsent`).
    pub fn leave_preview_without_sharing(
        &mut self,
        credential: &InviteCredential,
        account_id: &MobileAccountId,
        occurred_at: impl Into<String>,
    ) -> Result<(), BridgeError> {
        let inv_id = self.resolve_active_invitation(credential)?.id.clone();
        self.push_audit(
            occurred_at.into(),
            AuditEventKind::PreviewExitedWithoutShare {
                invitation_id: inv_id,
                account_id: account_id.clone(),
            },
        );
        Ok(())
    }

    /// Explicitly confirms **Share** from the signed-in Curie Mobile account,
    /// transitioning `Invited -> Connected` (`ConnectsOnlyOnOpenShare`,
    /// `ShareConnectsSignedInAccount`). Historical `To send` items never transfer
    /// automatically upon connecting.
    pub fn confirm_share(
        &mut self,
        credential: &InviteCredential,
        signed_in_account: &MobileAccountId,
        confirmed_at: impl Into<String>,
    ) -> Result<ConnectionStanding, BridgeError> {
        self.ensure_account_not_deleted(signed_in_account)?;
        let inv_id = self.resolve_active_invitation(credential)?.id.clone();

        let (pair, tokens, codes) = {
            let inv = self.invitations.get(&inv_id).ok_or_else(|| {
                BridgeError::InvalidOrRevokedCredential {
                    invitation_id: inv_id.as_str().to_owned(),
                }
            })?;
            match inv.status {
                InvitationStatus::OpenUnlocked => {}
                InvitationStatus::OpenLocked => {
                    return Err(BridgeError::InvitationLocked {
                        invitation_id: inv_id,
                    });
                }
                InvitationStatus::Cancelled => {
                    return Err(BridgeError::InvitationCancelled {
                        invitation_id: inv_id,
                    });
                }
                InvitationStatus::Consumed { .. } => {
                    return Err(BridgeError::InvalidOrRevokedCredential {
                        invitation_id: inv_id.as_str().to_owned(),
                    });
                }
            }
            if !inv.verified_accounts.contains(signed_in_account) {
                return Err(BridgeError::DobVerificationRequired {
                    account_id: signed_in_account.clone(),
                });
            }
            (
                inv.pair.clone(),
                inv.link_tokens.clone(),
                inv.short_codes.clone(),
            )
        };

        let seq = self.next_id();
        let connection_id = ConnectionId::new(format!("conn-{seq}"))?;

        if let Some(inv) = self.invitations.get_mut(&inv_id) {
            inv.status = InvitationStatus::Consumed {
                connection_id: connection_id.clone(),
            };
        }
        for tok in tokens {
            let _ = self.token_index.remove(&tok);
        }
        for code in codes {
            let _ = self.code_index.remove(&code);
        }

        let _ = self.connections.insert(
            connection_id.clone(),
            ConnectionRecord {
                id: connection_id.clone(),
                pair: pair.clone(),
                account_id: signed_in_account.clone(),
                status: ConnectionStatus::Connected,
            },
        );

        let pair_entry = self.pairs.entry(pair.clone()).or_default();
        pair_entry.open_invitation = None;
        pair_entry.active_connection = Some(connection_id.clone());

        self.push_audit(
            confirmed_at.into(),
            AuditEventKind::ConnectionEstablished {
                pair: pair.clone(),
                invitation_id: inv_id,
                connection_id,
                account_id: signed_in_account.clone(),
            },
        );

        Ok(self.standing_for_pair(&pair))
    }

    /// Stages an explicitly confirmed **For my doctor** cross-product transfer
    /// envelope, enforcing active `Connected` status, idempotency on
    /// `IdempotencyKey`, and 42 CFR Part 2 written consent + verbatim notice.
    pub fn submit_transfer(
        &mut self,
        submission: TransferSubmission,
    ) -> Result<TransferEnvelope, BridgeError> {
        self.ensure_account_not_deleted(&submission.sender_account)?;

        let conn = self
            .connections
            .get(&submission.connection_id)
            .ok_or_else(|| BridgeError::ConnectionNotActive {
                connection_id: submission.connection_id.clone(),
                state: ConnectedCareState::NotInvited,
            })?;

        if !matches!(conn.status, ConnectionStatus::Connected) {
            return Err(BridgeError::ConnectionNotActive {
                connection_id: submission.connection_id,
                state: ConnectedCareState::Ended,
            });
        }
        if conn.account_id != submission.sender_account {
            return Err(BridgeError::UnauthorizedSender {
                connection_id: submission.connection_id,
                sender: submission.sender_account,
            });
        }

        let pair = conn.pair.clone();

        // Idempotent retry reuse: if the same idempotency key already exists on
        // this active connection, return the exact frozen envelope without duplicating!
        let idem_key = (
            submission.connection_id.clone(),
            submission.idempotency_key.clone(),
        );
        if let Some(existing_id) = self.transfer_idempotency.get(&idem_key)
            && let Some(existing) = self.transfers.get(existing_id)
        {
            return Ok(existing.clone());
        }

        let part2_consent_id = match validate_transfer_consent(&submission, &pair.practice_id) {
            Ok(consent_id) => consent_id,
            Err(err) => {
                if matches!(
                    err,
                    BridgeError::Part2WrittenConsentRequired
                        | BridgeError::Part2ConsentScopeMismatch
                        | BridgeError::Part2VerbatimNoticeMissing { .. }
                ) {
                    self.push_audit(
                        submission.sent_at.clone(),
                        AuditEventKind::TransferRejectedPart2 {
                            connection_id: submission.connection_id,
                            sender_account: submission.sender_account,
                        },
                    );
                }
                return Err(err);
            }
        };

        let seq = self.next_id();
        let transfer_id = TransferId::new(format!("xfer-{seq}"))?;
        let item_count = submission.items.len();

        let envelope = TransferEnvelope {
            transfer_id: transfer_id.clone(),
            idempotency_key: submission.idempotency_key,
            connection_id: submission.connection_id.clone(),
            practice_id: pair.practice_id,
            practice_patient_id: pair.practice_patient_id,
            sender_account: submission.sender_account,
            sent_at: submission.sent_at.clone(),
            items: submission.items,
            part2_consent_id: part2_consent_id.clone(),
            status: TransferStatus::Sending,
        };

        let _ = self.transfers.insert(transfer_id.clone(), envelope.clone());
        let _ = self
            .transfer_idempotency
            .insert(idem_key, transfer_id.clone());

        self.push_audit(
            submission.sent_at,
            AuditEventKind::TransferStaged {
                connection_id: submission.connection_id,
                transfer_id,
                item_count,
                part2_consent_id,
            },
        );

        Ok(envelope)
    }

    /// Marks an in-flight transfer `Uncertain` (e.g., network timeout before practice ack).
    pub fn mark_transfer_uncertain(
        &mut self,
        transfer_id: &TransferId,
        reason: impl Into<String>,
        occurred_at: impl Into<String>,
    ) -> Result<TransferEnvelope, BridgeError> {
        let (connection_id, updated) = {
            let xfer = self.transfers.get_mut(transfer_id).ok_or_else(|| {
                BridgeError::TransferNotFound {
                    transfer_id: transfer_id.clone(),
                }
            })?;
            if !matches!(xfer.status, TransferStatus::Sending) {
                return Err(BridgeError::InvalidTransferTransition {
                    transfer_id: transfer_id.clone(),
                    from: xfer.status.label(),
                    to: "Uncertain",
                });
            }
            xfer.status = TransferStatus::Uncertain {
                reason: reason.into(),
            };
            (xfer.connection_id.clone(), xfer.clone())
        };

        self.push_audit(
            occurred_at.into(),
            AuditEventKind::TransferMarkedUncertain {
                connection_id,
                transfer_id: transfer_id.clone(),
            },
        );

        Ok(updated)
    }

    /// Confirms practice chart receipt, transitioning `Sending | Uncertain -> Delivered`.
    /// Fails closed if the connection has already ended.
    pub fn confirm_practice_receipt(
        &mut self,
        transfer_id: &TransferId,
        received_at: impl Into<String>,
    ) -> Result<TransferEnvelope, BridgeError> {
        let received_at = received_at.into();
        let connection_id = self
            .transfers
            .get(transfer_id)
            .map(|t| t.connection_id.clone())
            .ok_or_else(|| BridgeError::TransferNotFound {
                transfer_id: transfer_id.clone(),
            })?;

        let conn_active = self
            .connections
            .get(&connection_id)
            .is_some_and(|c| matches!(c.status, ConnectionStatus::Connected));
        if !conn_active {
            return Err(BridgeError::ConnectionNotActive {
                connection_id,
                state: ConnectedCareState::Ended,
            });
        }

        let updated = {
            let xfer = self.transfers.get_mut(transfer_id).ok_or_else(|| {
                BridgeError::TransferNotFound {
                    transfer_id: transfer_id.clone(),
                }
            })?;
            if !xfer.status.is_in_flight() {
                return Err(BridgeError::InvalidTransferTransition {
                    transfer_id: transfer_id.clone(),
                    from: xfer.status.label(),
                    to: "Delivered",
                });
            }
            xfer.status = TransferStatus::Delivered {
                received_at: received_at.clone(),
            };
            xfer.clone()
        };

        self.push_audit(
            received_at,
            AuditEventKind::TransferDelivered {
                connection_id,
                transfer_id: transfer_id.clone(),
            },
        );

        Ok(updated)
    }

    /// Ends an active connection (`Connected -> Ended`) idempotently.
    ///
    /// Immediately stops all future transfers and halts any `Sending` or
    /// `Uncertain` sends without deleting earlier `Delivered` history or
    /// affecting other practices (`ConnectionEndStopsUncertain`,
    /// `PracticesStayIndependent`).
    pub fn end_connection(
        &mut self,
        connection_id: &ConnectionId,
        actor: DisconnectActor,
        ended_at: impl Into<String>,
    ) -> Result<ConnectionStanding, BridgeError> {
        let ended_at = ended_at.into();
        let (pair, already_ended) = {
            let conn = self.connections.get(connection_id).ok_or_else(|| {
                BridgeError::ConnectionNotActive {
                    connection_id: connection_id.clone(),
                    state: ConnectedCareState::NotInvited,
                }
            })?;

            // Authorize the actor against this connection.
            match &actor {
                DisconnectActor::Patient { account_id } if &conn.account_id != account_id => {
                    return Err(BridgeError::UnauthorizedSender {
                        connection_id: connection_id.clone(),
                        sender: account_id.clone(),
                    });
                }
                DisconnectActor::PracticeMember { practice_id, .. }
                    if &conn.pair.practice_id != practice_id =>
                {
                    return Err(BridgeError::UnauthorizedPractice {
                        connection_id: connection_id.clone(),
                        practice_id: practice_id.as_str().to_owned(),
                    });
                }
                _ => {}
            }

            (
                conn.pair.clone(),
                matches!(conn.status, ConnectionStatus::Ended { .. }),
            )
        };

        if already_ended {
            return Ok(self.standing_for_pair(&pair));
        }

        if let Some(conn) = self.connections.get_mut(connection_id) {
            conn.status = ConnectionStatus::Ended {
                ended_at: ended_at.clone(),
            };
        }

        if let Some(pair_entry) = self.pairs.get_mut(&pair) {
            if pair_entry.active_connection.as_ref() == Some(connection_id) {
                pair_entry.active_connection = None;
            }
            pair_entry.has_prior_ended_connection = true;
        }

        let halted_ids: Vec<TransferId> = self
            .transfers
            .values_mut()
            .filter(|xfer| &xfer.connection_id == connection_id && xfer.status.is_in_flight())
            .map(|xfer| {
                xfer.status = TransferStatus::HaltedOnConnectionEnd {
                    ended_at: ended_at.clone(),
                };
                xfer.transfer_id.clone()
            })
            .collect();

        for transfer_id in halted_ids {
            self.push_audit(
                ended_at.clone(),
                AuditEventKind::TransferHaltedOnDisconnect {
                    connection_id: connection_id.clone(),
                    transfer_id,
                },
            );
        }

        self.push_audit(
            ended_at,
            AuditEventKind::ConnectionEnded {
                pair: pair.clone(),
                connection_id: connection_id.clone(),
                actor,
            },
        );

        Ok(self.standing_for_pair(&pair))
    }

    /// Idempotently terminates every connected-care link and scrubs staged
    /// transfer records for a deleted Curie Mobile account (safe for repeated
    /// privacy receipt journal replay after a DB restore).
    pub fn terminate_account_links(
        &mut self,
        account_id: &MobileAccountId,
        account_incarnation: AccountIncarnation,
        occurred_at: impl Into<String>,
    ) -> AccountTerminationSummary {
        let occurred_at = occurred_at.into();

        // Record monotonic highest deleted incarnation fence so replay cannot resurrect.
        let prev = self.deleted_accounts.get(account_id).copied();
        if prev.is_none_or(|p| account_incarnation > p) {
            let _ = self
                .deleted_accounts
                .insert(account_id.clone(), account_incarnation);
        }

        let active_conn_ids: Vec<ConnectionId> = self
            .connections
            .values()
            .filter(|c| {
                &c.account_id == account_id && matches!(c.status, ConnectionStatus::Connected)
            })
            .map(|c| c.id.clone())
            .collect();

        let mut connections_ended = 0;
        for conn_id in active_conn_ids {
            if self
                .end_connection(
                    &conn_id,
                    DisconnectActor::AccountDeletionReplay {
                        account_id: account_id.clone(),
                        account_incarnation,
                    },
                    occurred_at.clone(),
                )
                .is_ok()
            {
                connections_ended += 1;
            }
        }

        // Cancel any open invitations where this account had verified DOB.
        let open_pairs_for_account: Vec<PairKey> = self
            .invitations
            .values()
            .filter(|inv| inv.is_open() && inv.verified_accounts.contains(account_id))
            .map(|inv| inv.pair.clone())
            .collect();

        let mut invitations_cancelled = 0;
        for pair in open_pairs_for_account {
            if self.cancel_invitation(&pair, occurred_at.clone()).is_ok() {
                invitations_cancelled += 1;
            }
        }

        // Scrub all non-delivered / staged transfer payloads for this account.
        let mut transfers_scrubbed = 0;
        for xfer in self.transfers.values_mut() {
            if &xfer.sender_account == account_id
                && !matches!(
                    xfer.status,
                    TransferStatus::Delivered { .. }
                        | TransferStatus::ScrubbedOnAccountDeletion { .. }
                )
            {
                xfer.items.clear();
                xfer.status = TransferStatus::ScrubbedOnAccountDeletion {
                    scrubbed_at: occurred_at.clone(),
                };
                transfers_scrubbed += 1;
            }
        }

        self.push_audit(
            occurred_at,
            AuditEventKind::AccountDeletionReplayed {
                account_id: account_id.clone(),
                account_incarnation,
                connections_ended,
                invitations_cancelled,
                transfers_scrubbed,
            },
        );

        AccountTerminationSummary {
            connections_ended,
            invitations_cancelled,
            transfers_scrubbed,
        }
    }

    #[must_use]
    pub fn transfer(&self, transfer_id: &TransferId) -> Option<&TransferEnvelope> {
        self.transfers.get(transfer_id)
    }

    /// Returns all delivered (confirmed) transfer envelopes for a practice-patient pair.
    #[must_use]
    pub fn delivered_transfers_for_pair(&self, pair: &PairKey) -> Vec<&TransferEnvelope> {
        self.transfers
            .values()
            .filter(|xfer| {
                xfer.practice_id == pair.practice_id
                    && xfer.practice_patient_id == pair.practice_patient_id
                    && matches!(xfer.status, TransferStatus::Delivered { .. })
            })
            .collect()
    }

    /// Returns the content-free metadata-only audit trail.
    #[must_use]
    pub fn audit_log(&self) -> &[AuditEvent] {
        &self.audit_log
    }

    /// Audits all structural invariants across the entire bridge store.
    pub fn audit_all_invariants(&self) -> Result<InvariantAuditReport, Vec<InvariantViolation>> {
        let mut violations = Vec::new();
        let mut open_counts: HashMap<&PairKey, usize> = HashMap::new();

        for inv in self.invitations.values() {
            if inv.is_open() {
                *open_counts.entry(&inv.pair).or_insert(0) += 1;
            }

            // LockedAfterFiveWrongDob consistency.
            let is_locked = matches!(inv.status, InvitationStatus::OpenLocked);
            if inv.is_open() && ((inv.wrong_dob_attempts >= MAX_WRONG_DOB_ATTEMPTS) != is_locked) {
                violations.push(InvariantViolation::LockoutStateMismatch {
                    invitation_id: inv.id.clone(),
                    attempts: inv.wrong_dob_attempts,
                    is_locked,
                });
            }

            // CancelledNeverConnects credential revocation consistency.
            if matches!(inv.status, InvitationStatus::Cancelled) {
                let any_live = inv
                    .link_tokens
                    .iter()
                    .any(|t| self.token_index.contains_key(t))
                    || inv
                        .short_codes
                        .iter()
                        .any(|c| self.code_index.contains_key(c));
                if any_live {
                    violations.push(InvariantViolation::CancelledCredentialStillActive {
                        invitation_id: inv.id.clone(),
                    });
                }
            }
        }

        for (pair, count) in &open_counts {
            if *count > 1 {
                violations.push(InvariantViolation::MultipleOpenInvitations {
                    pair: (*pair).clone(),
                    open_count: *count,
                });
            }
        }

        // SameConnectionState consistency between clinic and mobile views.
        for pair in self.pairs.keys() {
            let clinic = self.clinic_standing(pair).state;
            let mobile = self.mobile_standing(pair).state;
            if clinic != mobile {
                violations.push(InvariantViolation::DriftedConnectionState {
                    pair: pair.clone(),
                    clinic,
                    mobile,
                });
            }
        }

        // ConnectionEndStopsUncertain consistency.
        for xfer in self.transfers.values() {
            if xfer.status.is_in_flight()
                && let Some(conn) = self.connections.get(&xfer.connection_id)
                && matches!(conn.status, ConnectionStatus::Ended { .. })
            {
                violations.push(InvariantViolation::LiveTransferAfterDisconnect {
                    connection_id: conn.id.clone(),
                    transfer_id: xfer.transfer_id.clone(),
                });
            }
        }

        if violations.is_empty() {
            let active_connections = self
                .connections
                .values()
                .filter(|c| matches!(c.status, ConnectionStatus::Connected))
                .count();
            Ok(InvariantAuditReport {
                tracked_pairs: self.pairs.len(),
                open_invitations: open_counts.values().sum(),
                active_connections,
                audited_transfers: self.transfers.len(),
            })
        } else {
            Err(violations)
        }
    }

    fn standing_for_pair(&self, pair: &PairKey) -> ConnectionStanding {
        let Some(record) = self.pairs.get(pair) else {
            return ConnectionStanding {
                pair: pair.clone(),
                state: ConnectedCareState::NotInvited,
                active_invitation_id: None,
                active_connection_id: None,
                connected_account: None,
            };
        };

        let state = record.standing(&self.invitations);
        let connected_account = record
            .active_connection
            .as_ref()
            .and_then(|id| self.connections.get(id))
            .map(|conn| conn.account_id.clone());

        ConnectionStanding {
            pair: pair.clone(),
            state,
            active_invitation_id: record.open_invitation.clone(),
            active_connection_id: record.active_connection.clone(),
            connected_account,
        }
    }

    fn resolve_active_invitation(
        &self,
        credential: &InviteCredential,
    ) -> Result<&InvitationRecord, BridgeError> {
        let inv_id = match credential {
            InviteCredential::LinkToken(token) => self.token_index.get(token).ok_or_else(|| {
                BridgeError::InvalidOrRevokedCredential {
                    invitation_id: token.as_str().to_owned(),
                }
            })?,
            InviteCredential::ShortCode(code) => self.code_index.get(code).ok_or_else(|| {
                BridgeError::InvalidOrRevokedCredential {
                    invitation_id: code.as_str().to_owned(),
                }
            })?,
        };
        self.invitations
            .get(inv_id)
            .ok_or_else(|| BridgeError::InvalidOrRevokedCredential {
                invitation_id: inv_id.as_str().to_owned(),
            })
    }

    fn ensure_account_not_deleted(&self, account_id: &MobileAccountId) -> Result<(), BridgeError> {
        if let Some(incarnation) = self.deleted_accounts.get(account_id) {
            return Err(BridgeError::AccountDeleted {
                account_id: account_id.clone(),
                deleted_incarnation: incarnation.as_u64(),
            });
        }
        Ok(())
    }

    fn next_id(&mut self) -> u64 {
        self.next_seq += 1;
        self.next_seq
    }

    fn push_audit(&mut self, occurred_at: String, kind: AuditEventKind) {
        let sequence = self.audit_log.len() as u64 + 1;
        self.audit_log.push(AuditEvent {
            sequence,
            occurred_at,
            kind,
        });
    }
}
