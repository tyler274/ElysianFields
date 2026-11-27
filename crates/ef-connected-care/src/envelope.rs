//! Consent-gated cross-product data envelope and 42 CFR Part 2 enforcement.

use ef_core::{IdempotencyKey, PatientId, PracticeId};
use serde::{Deserialize, Serialize};

use crate::error::BridgeError;
use crate::types::{
    ConfidentialityTag, ConnectionId, ConsentRecordId, MobileAccountId, TransferId,
};

/// Verbatim 42 CFR § 2.32(a) statutory prohibition on redisclosure required on
/// every Part 2 SUD record or SUD counseling note crossing Connected Care.
pub const VERBATIM_PART2_REDISCLOSURE_NOTICE: &str = "This record which has been disclosed to you is protected by federal confidentiality rules (42 CFR part 2). These rules prohibit you from using or disclosing this record, or testimony that describes the information contained in this record, in any civil, criminal, administrative, or legislative proceedings by any federal, state, or local authority, against the patient, unless authorized by the consent of the patient, except as provided at 42 CFR 2.12(c)(5) or as authorized by a court in accordance with 42 CFR 2.64 or 2.65. In addition, the federal rules prohibit you from making any other use or disclosure of this record unless at least one of the following applies: (i) Further use or disclosure is expressly permitted by the written consent of the individual whose information is being disclosed in this record or as otherwise permitted by 42 CFR part 2. (ii) A general authorization for the release of medical or other information is NOT sufficient to permit further use or disclosure for any purpose.";

/// One immutable curated item inside a confirmed **For my doctor** send envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvelopeItem {
    pub item_id: String,
    pub source_version: u64,
    pub confidentiality: ConfidentialityTag,
    pub payload_sha256: String,
    pub verbatim_payload: String,
    pub part2_notice: Option<String>,
}

/// Explicit written consent artifact required for any Part 2 transfer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Part2WrittenConsent {
    pub consent_id: ConsentRecordId,
    pub patient_account: MobileAccountId,
    pub practice_id: PracticeId,
    pub granted_at: String,
    pub scope_description: String,
    pub notice_acknowledged: ConsentAcknowledgement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConsentAcknowledgement {
    Acknowledged,
    NotAcknowledged,
}

/// Request submitted by Curie Mobile upon explicit patient confirmation of a send.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferSubmission {
    pub idempotency_key: IdempotencyKey,
    pub connection_id: ConnectionId,
    pub sender_account: MobileAccountId,
    pub sent_at: String,
    pub items: Vec<EnvelopeItem>,
    pub part2_consent: Option<Part2WrittenConsent>,
}

/// Lifecycle status of a cross-product transfer envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferStatus {
    Sending,
    Uncertain { reason: String },
    Delivered { received_at: String },
    HaltedOnConnectionEnd { ended_at: String },
    ScrubbedOnAccountDeletion { scrubbed_at: String },
}

impl TransferStatus {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Sending => "Sending",
            Self::Uncertain { .. } => "Uncertain",
            Self::Delivered { .. } => "Delivered",
            Self::HaltedOnConnectionEnd { .. } => "HaltedOnConnectionEnd",
            Self::ScrubbedOnAccountDeletion { .. } => "ScrubbedOnAccountDeletion",
        }
    }

    #[must_use]
    pub const fn is_in_flight(&self) -> bool {
        matches!(self, Self::Sending | Self::Uncertain { .. })
    }
}

/// Authoritative cross-product data envelope tracked by the Connected Care bridge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferEnvelope {
    pub transfer_id: TransferId,
    pub idempotency_key: IdempotencyKey,
    pub connection_id: ConnectionId,
    pub practice_id: PracticeId,
    pub practice_patient_id: PatientId,
    pub sender_account: MobileAccountId,
    pub sent_at: String,
    pub items: Vec<EnvelopeItem>,
    pub part2_consent_id: Option<ConsentRecordId>,
    pub status: TransferStatus,
}

/// Single-owner validator enforcing explicit item presence, 42 CFR Part 2
/// written consent, and verbatim § 2.32(a) notices before any transfer is staged.
pub fn validate_transfer_consent(
    submission: &TransferSubmission,
    expected_practice: &PracticeId,
) -> Result<Option<ConsentRecordId>, BridgeError> {
    if submission.items.is_empty() {
        return Err(BridgeError::EmptyTransferEnvelope);
    }

    let mut requires_part2 = false;
    for item in &submission.items {
        if item.item_id.trim().is_empty() {
            return Err(BridgeError::EmptyField { field: "item_id" });
        }
        if item.confidentiality.requires_part2_written_consent() {
            requires_part2 = true;
            let has_verbatim = item
                .part2_notice
                .as_deref()
                .is_some_and(|notice| notice == VERBATIM_PART2_REDISCLOSURE_NOTICE);
            if !has_verbatim {
                return Err(BridgeError::Part2VerbatimNoticeMissing {
                    item_id: item.item_id.clone(),
                });
            }
        }
    }

    if !requires_part2 {
        return Ok(None);
    }

    let Some(consent) = &submission.part2_consent else {
        return Err(BridgeError::Part2WrittenConsentRequired);
    };

    if !matches!(
        consent.notice_acknowledged,
        ConsentAcknowledgement::Acknowledged
    ) {
        return Err(BridgeError::Part2WrittenConsentRequired);
    }

    if consent.patient_account != submission.sender_account
        || &consent.practice_id != expected_practice
    {
        return Err(BridgeError::Part2ConsentScopeMismatch);
    }

    if consent.scope_description.trim().is_empty() {
        return Err(BridgeError::EmptyField {
            field: "scope_description",
        });
    }

    Ok(Some(consent.consent_id.clone()))
}
