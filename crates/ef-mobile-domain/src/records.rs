//! Health records, SMART on FHIR R4 portal connections (PKCE S256), SMART Health Links
//! (`shlink:/`), C-CDA R2.1 XML & Kipu imports, per-record savepoint isolation, and
//! 42 CFR Part 2 confidentiality boundaries.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::account::{AccountIncarnation, MobileAccountId};

/// Verbatim 42 CFR § 2.32(a) prohibition-on-redisclosure notice.
pub const PART2_REDISCLOSURE_NOTICE: &str = "42 CFR Part 2 prohibits unauthorized disclosure of these records without specific written consent of the person to whom it pertains, or as otherwise permitted by 42 CFR Part 2.";

/// Closed 16-bucket clinical record classification from `m3-boundaries-and-states.schema.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ClinicalRecordBucket {
    Patient,
    Problems,
    Medications,
    Allergies,
    Results,
    Vitals,
    Notes,
    Documents,
    Visits,
    Procedures,
    Immunizations,
    Orders,
    History,
    Forms,
    Messages,
    Other,
}

/// Closed 42 CFR Part 2 confidentiality tag from `m3-boundaries-and-states.schema.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConfidentialityTag {
    Standard,
    Part2Sud,
    Part2SudCounselingNotes,
}

/// Supported SMART on FHIR R4 patient portal vendors ("Doctors & hospitals").
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortalVendor {
    EpicMyChart,
    OracleHealth,
    Athenahealth,
    EClinicalWorksHealow,
    MeditechExpanse,
    NextGen,
    Veradigm,
    VaHealth,
    CmsBlueButton2,
    CustomSmartFhirR4,
}

/// Origin channel for an imported patient record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImportSourceKind {
    SmartFhirR4 { vendor: PortalVendor },
    SmartHealthLink,
    CcdaR21Xml,
    KipuBehavioralHealth,
}

/// OAuth2 PKCE `S256` handshake record ensuring single-use portal linking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PkceS256Challenge {
    pub state: String,
    pub code_challenge: String,
    pub method: String,
    pub scopes: Vec<String>,
    pub consumed: bool,
}

impl PkceS256Challenge {
    /// Creates a validated PKCE `S256` challenge.
    pub fn new(
        state: impl Into<String>,
        code_challenge: impl Into<String>,
        scopes: Vec<String>,
    ) -> Result<Self, RecordImportError> {
        let state = state.into();
        let code_challenge = code_challenge.into();
        if state.trim().is_empty() || code_challenge.len() < 32 {
            return Err(RecordImportError::InvalidPkceChallenge);
        }
        Ok(Self {
            state,
            code_challenge,
            method: "S256".to_owned(),
            scopes,
            consumed: false,
        })
    }
}

/// Connection standing for a patient portal ("Doctors & hospitals").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortalLinkState {
    Connected,
    Disconnected,
}

/// Connected patient portal metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortalConnection {
    pub portal_id: String,
    pub account_id: MobileAccountId,
    pub vendor: PortalVendor,
    pub display_name: String,
    pub state: PortalLinkState,
    pub last_refresh_secs: u64,
    pub daily_refresh_interval_secs: u64,
}

/// Parsed and validated SMART Health Link (`shlink:/`) manifest payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmartHealthLinkPayload {
    pub url: String,
    pub key: String,
    #[serde(default)]
    pub flag: Option<String>,
    #[serde(default)]
    pub exp: Option<u64>,
    #[serde(default)]
    pub label: Option<String>,
}

/// Errors returned when validating portal links, `shlink:/` URIs, or record batches.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RecordImportError {
    #[error("invalid PKCE S256 challenge or state")]
    InvalidPkceChallenge,
    #[error("PKCE state already consumed or unknown")]
    PkceStateReplayedOrUnknown,
    #[error("portal vendor {vendor:?} is paused by operator kill-switch")]
    VendorKillSwitchActive { vendor: PortalVendor },
    #[error("SMART Health Link must start with `shlink:/`")]
    InvalidShlScheme,
    #[error("SMART Health Link base64url payload is malformed")]
    MalformedShlPayload,
    #[error("SMART Health Link destination must be a public https:// URL")]
    InsecureShlDestination,
    #[error("SMART Health Link key must decode to exactly 32 bytes (got {actual_bytes})")]
    InvalidShlKeyLength { actual_bytes: usize },
    #[error("SMART Health Link expired at {expired_at_secs}")]
    ShlExpired { expired_at_secs: u64 },
    #[error("portal '{portal_id}' not found")]
    PortalNotFound { portal_id: String },
}

impl SmartHealthLinkPayload {
    /// Strictly validates a `shlink:/<base64url-json>` URI before any network fetch.
    ///
    /// Enforces:
    /// - `shlink:/` scheme prefix
    /// - Valid JSON payload with public `https://` destination (rejects `http://`, localhost, loopback)
    /// - Exact 32-byte key (base64url-encoded 32 raw bytes)
    /// - Unexpired `exp` timestamp when present
    pub fn parse_uri(raw_uri: &str, now_secs: u64) -> Result<Self, RecordImportError> {
        let encoded = raw_uri
            .trim()
            .strip_prefix("shlink:/")
            .ok_or(RecordImportError::InvalidShlScheme)?;
        if encoded.is_empty() {
            return Err(RecordImportError::MalformedShlPayload);
        }
        let decoded = decode_base64url(encoded).ok_or(RecordImportError::MalformedShlPayload)?;
        let Ok(payload) = serde_json::from_slice::<Self>(&decoded) else {
            return Err(RecordImportError::MalformedShlPayload);
        };

        validate_public_https_url(&payload.url)?;

        let key_bytes = decode_base64url(&payload.key)
            .ok_or(RecordImportError::InvalidShlKeyLength { actual_bytes: 0 })?;
        if key_bytes.len() != 32 {
            return Err(RecordImportError::InvalidShlKeyLength {
                actual_bytes: key_bytes.len(),
            });
        }

        if let Some(exp) = payload.exp
            && now_secs >= exp
        {
            return Err(RecordImportError::ShlExpired {
                expired_at_secs: exp,
            });
        }

        Ok(payload)
    }

    /// Encodes a [`SmartHealthLinkPayload`] into canonical `shlink:/<base64url>` form.
    #[must_use]
    pub fn to_shl_uri(&self) -> String {
        let json = serde_json::to_vec(self).unwrap_or_default();
        format!("shlink:/{}", encode_base64url(&json))
    }
}

/// Patient-owned health record persisted inside Curie Mobile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatientRecord {
    pub record_id: String,
    pub version: u64,
    pub account_id: MobileAccountId,
    pub incarnation: AccountIncarnation,
    pub source_kind: ImportSourceKind,
    pub portal_id: Option<String>,
    pub bucket: ClinicalRecordBucket,
    pub confidentiality: ConfidentialityTag,
    pub part2_notice: Option<String>,
    pub original_date: String,
    pub title: String,
    pub summary: String,
    pub native_bytes: Vec<u8>,
    pub removed_upstream: bool,
}

impl PatientRecord {
    /// Returns true if this record is subject to 42 CFR Part 2 confidentiality rules.
    #[must_use]
    pub const fn is_part2(&self) -> bool {
        matches!(
            self.confidentiality,
            ConfidentialityTag::Part2Sud | ConfidentialityTag::Part2SudCounselingNotes
        )
    }

    /// Returns true if eligible for general connected-care sharing without Part 2 written consent.
    #[must_use]
    pub const fn is_general_share_eligible(&self) -> bool {
        matches!(self.confidentiality, ConfidentialityTag::Standard) && !self.removed_upstream
    }
}

/// One element in an incoming portal, SHL, C-CDA, or Kipu ingestion batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IncomingBatchItem {
    Valid(PatientRecord),
    Malformed { external_id: String, reason: String },
}

/// Per-record error captured without aborting valid siblings in the same batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailedImportItem {
    pub external_id: String,
    pub reason: String,
}

/// Result of per-record savepoint ingestion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavepointBatchReport {
    pub committed_record_ids: Vec<String>,
    pub failed_items: Vec<FailedImportItem>,
}

/// Authoritative patient-owned record & portal store inside Curie Mobile.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PatientRecordStore {
    portals: BTreeMap<String, PortalConnection>,
    pkce_challenges: BTreeMap<String, PkceS256Challenge>,
    paused_vendors: BTreeSet<PortalVendor>,
    records: BTreeMap<(MobileAccountId, String), PatientRecord>,
}

impl PatientRecordStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Enables or clears an operator per-vendor kill-switch without breaking existing records.
    pub fn set_vendor_kill_switch(&mut self, vendor: PortalVendor, paused: bool) {
        if paused {
            self.paused_vendors.insert(vendor);
        } else {
            self.paused_vendors.remove(&vendor);
        }
    }

    /// Registers a single-use PKCE `S256` challenge before redirecting to a patient portal.
    pub fn begin_portal_oauth(
        &mut self,
        vendor: PortalVendor,
        challenge: PkceS256Challenge,
    ) -> Result<(), RecordImportError> {
        if self.paused_vendors.contains(&vendor) {
            return Err(RecordImportError::VendorKillSwitchActive { vendor });
        }
        self.pkce_challenges
            .insert(challenge.state.clone(), challenge);
        Ok(())
    }

    /// Completes single-use PKCE `S256` portal linking.
    pub fn complete_portal_oauth(
        &mut self,
        state: &str,
        portal_id: impl Into<String>,
        account_id: MobileAccountId,
        vendor: PortalVendor,
        display_name: impl Into<String>,
        now_secs: u64,
    ) -> Result<PortalConnection, RecordImportError> {
        if self.paused_vendors.contains(&vendor) {
            return Err(RecordImportError::VendorKillSwitchActive { vendor });
        }
        let Some(challenge) = self.pkce_challenges.get_mut(state) else {
            return Err(RecordImportError::PkceStateReplayedOrUnknown);
        };
        if challenge.consumed || challenge.method != "S256" {
            return Err(RecordImportError::PkceStateReplayedOrUnknown);
        }
        challenge.consumed = true;

        let portal_id = portal_id.into();
        let connection = PortalConnection {
            portal_id: portal_id.clone(),
            account_id,
            vendor,
            display_name: display_name.into(),
            state: PortalLinkState::Connected,
            last_refresh_secs: now_secs,
            daily_refresh_interval_secs: 86_400,
        };
        self.portals.insert(portal_id, connection.clone());
        Ok(connection)
    }

    /// Disconnects a portal while preserving 100% of previously imported patient history.
    pub fn disconnect_portal(&mut self, portal_id: &str) -> Result<(), RecordImportError> {
        let Some(conn) = self.portals.get_mut(portal_id) else {
            return Err(RecordImportError::PortalNotFound {
                portal_id: portal_id.to_owned(),
            });
        };
        conn.state = PortalLinkState::Disconnected;
        Ok(())
    }

    /// Ingests a mixed batch with **per-record savepoint isolation**:
    /// valid records commit independently with verbatim Part 2 notices attached where required,
    /// while malformed sibling items fail cleanly into `failed_items`.
    pub fn ingest_batch_with_savepoints(
        &mut self,
        items: Vec<IncomingBatchItem>,
    ) -> SavepointBatchReport {
        let mut committed_record_ids = Vec::new();
        let mut failed_items = Vec::new();

        for item in items {
            match item {
                IncomingBatchItem::Malformed {
                    external_id,
                    reason,
                } => {
                    failed_items.push(FailedImportItem {
                        external_id,
                        reason,
                    });
                }
                IncomingBatchItem::Valid(mut record) => {
                    if record.record_id.trim().is_empty() || record.native_bytes.is_empty() {
                        failed_items.push(FailedImportItem {
                            external_id: record.record_id,
                            reason: "record_id and native_bytes must be non-empty".to_owned(),
                        });
                        continue;
                    }
                    if record.is_part2() && record.part2_notice.is_none() {
                        record.part2_notice = Some(PART2_REDISCLOSURE_NOTICE.to_owned());
                    }
                    let id = record.record_id.clone();
                    let key = (record.account_id.clone(), id.clone());
                    self.records.insert(key, record);
                    committed_record_ids.push(id);
                }
            }
        }

        SavepointBatchReport {
            committed_record_ids,
            failed_items,
        }
    }

    /// Applies an upstream source correction, incrementing version and summary.
    pub fn apply_upstream_correction(
        &mut self,
        account_id: &MobileAccountId,
        record_id: &str,
        new_summary: impl Into<String>,
    ) -> Option<u64> {
        let rec = self
            .records
            .get_mut(&(account_id.clone(), record_id.to_owned()))?;
        rec.version = rec.version.saturating_add(1);
        rec.summary = new_summary.into();
        Some(rec.version)
    }

    /// Marks an upstream record removed so `For my doctor` clears its mark cleanly.
    pub fn mark_removed_upstream(&mut self, account_id: &MobileAccountId, record_id: &str) -> bool {
        if let Some(rec) = self
            .records
            .get_mut(&(account_id.clone(), record_id.to_owned()))
        {
            rec.removed_upstream = true;
            true
        } else {
            false
        }
    }

    /// Purges all records belonging to `(account_id, <= incarnation)` (used by account deletion & restore replay).
    pub fn purge_account_records(
        &mut self,
        account_id: &MobileAccountId,
        up_to_incarnation: AccountIncarnation,
    ) -> usize {
        let before = self.records.len();
        self.records
            .retain(|(acct, _), rec| !(acct == account_id && rec.incarnation <= up_to_incarnation));
        self.portals
            .retain(|_, conn| &conn.account_id != account_id);
        before.saturating_sub(self.records.len())
    }

    /// Lists active records owned by `account_id`.
    #[must_use]
    pub fn records_for_account(&self, account_id: &MobileAccountId) -> Vec<&PatientRecord> {
        self.records
            .values()
            .filter(|r| &r.account_id == account_id && !r.removed_upstream)
            .collect()
    }

    /// Looks up a single record owned by `account_id`.
    #[must_use]
    pub fn get_record(
        &self,
        account_id: &MobileAccountId,
        record_id: &str,
    ) -> Option<&PatientRecord> {
        self.records
            .get(&(account_id.clone(), record_id.to_owned()))
    }

    /// Looks up a portal connection by ID.
    #[must_use]
    pub fn portal(&self, portal_id: &str) -> Option<&PortalConnection> {
        self.portals.get(portal_id)
    }
}

fn validate_public_https_url(url: &str) -> Result<(), RecordImportError> {
    let rest = url
        .strip_prefix("https://")
        .ok_or(RecordImportError::InsecureShlDestination)?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if host.is_empty()
        || host == "localhost"
        || host == "127.0.0.1"
        || host == "::1"
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || host.ends_with(".local")
        || !host.contains('.')
    {
        return Err(RecordImportError::InsecureShlDestination);
    }
    Ok(())
}

/// Encodes raw bytes as RFC 4648 URL-safe base64 without padding.
#[must_use]
pub fn encode_base64url(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let mut i = 0;
    while i < input.len() {
        let b0 = input[i];
        let b1 = if i + 1 < input.len() { input[i + 1] } else { 0 };
        let b2 = if i + 2 < input.len() { input[i + 2] } else { 0 };
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if i + 1 < input.len() {
            out.push(TABLE[(((b1 & 0x0F) << 2) | (b2 >> 6)) as usize] as char);
        }
        if i + 2 < input.len() {
            out.push(TABLE[(b2 & 0x3F) as usize] as char);
        }
        i += 3;
    }
    out
}

/// Decodes RFC 4648 URL-safe base64 (with or without `=` padding).
#[must_use]
pub fn decode_base64url(input: &str) -> Option<Vec<u8>> {
    let trimmed = input.trim_end_matches('=');
    let mut out = Vec::with_capacity((trimmed.len() * 3) / 4);
    let mut buf: u32 = 0;
    let mut bits: u8 = 0;
    for byte in trimmed.bytes() {
        let val = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        buf = (buf << 6) | u32::from(val);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
}
