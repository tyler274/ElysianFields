//! On-device HealthKit / Health Connect acquisition, `(account_id, account_incarnation)`-fenced
//! transactional outbox, idempotent upload receiver, and APNs/FCM push token lifecycle.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::account::{AccountIncarnation, MobileAccountId};

/// Underlying native mobile health acquisition framework.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthAdapterPlatform {
    IosHealthKit,
    AndroidHealthConnect,
}

/// Granular HealthKit / Health Connect read metric categories from Chapter 2 §2.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthMetricKind {
    Steps,
    ActiveCalories,
    Exercise,
    Distance,
    Nutrition,
    Hydration,
    RestingHeartRate,
    Weight,
    Sleep,
    BloodPressure,
    BodyFat,
    LeanBodyMass,
    Height,
    BloodGlucose,
    BodyTemperature,
    BasalBodyTemperature,
    OxygenSaturation,
    Vo2Max,
}

/// Single synthetic or device-acquired health measurement sample.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthSampleEntry {
    pub sample_id: String,
    pub metric: HealthMetricKind,
    /// Fixed-point value in thousandths (e.g., `72_000` for `72.0 bpm`) for exact determinism.
    pub value_milli: i64,
    pub unit: String,
    pub recorded_at_secs: u64,
    pub platform: HealthAdapterPlatform,
}

/// Transactional outbox batch strictly fenced to `(account_id, account_incarnation)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountFencedHealthBatch {
    pub account_id: MobileAccountId,
    pub account_incarnation: AccountIncarnation,
    pub idempotency_key: String,
    pub samples: Vec<HealthSampleEntry>,
}

/// Errors produced during health outbox or upload processing.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HealthSyncError {
    #[error("health batch idempotency_key must not be empty")]
    EmptyIdempotencyKey,
    #[error("health batch must contain at least one sample")]
    EmptySamples,
    #[error("health batch account fence mismatch: expected {expected}, got {actual}")]
    AccountFenceMismatch {
        expected: MobileAccountId,
        actual: MobileAccountId,
    },
}

/// Durable on-device transactional outbox preventing cross-account sample leakage or silent drops.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HealthTransactionalOutbox {
    pending: Vec<AccountFencedHealthBatch>,
    uploaded_keys: BTreeSet<(MobileAccountId, String)>,
}

impl HealthTransactionalOutbox {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Commits a batch into the local outbox fenced strictly to `(account_id, account_incarnation)`.
    pub fn enqueue(&mut self, batch: AccountFencedHealthBatch) -> Result<bool, HealthSyncError> {
        if batch.idempotency_key.trim().is_empty() {
            return Err(HealthSyncError::EmptyIdempotencyKey);
        }
        if batch.samples.is_empty() {
            return Err(HealthSyncError::EmptySamples);
        }
        let dedup_key = (batch.account_id.clone(), batch.idempotency_key.clone());
        if self.uploaded_keys.contains(&dedup_key)
            || self.pending.iter().any(|b| {
                b.account_id == batch.account_id && b.idempotency_key == batch.idempotency_key
            })
        {
            return Ok(false);
        }
        self.pending.push(batch);
        Ok(true)
    }

    /// Returns ONLY pending batches fenced to `(account_id, incarnation)`.
    ///
    /// Guarantees Account B can NEVER observe or upload Account A's offline outbox entries.
    #[must_use]
    pub fn pending_for_account(
        &self,
        account_id: &MobileAccountId,
        incarnation: AccountIncarnation,
    ) -> Vec<AccountFencedHealthBatch> {
        self.pending
            .iter()
            .filter(|b| &b.account_id == account_id && b.account_incarnation == incarnation)
            .cloned()
            .collect()
    }

    /// Marks an account-fenced batch uploaded once server acknowledges persistence.
    pub fn mark_uploaded(&mut self, account_id: &MobileAccountId, idempotency_key: &str) {
        self.pending
            .retain(|b| !(&b.account_id == account_id && b.idempotency_key == idempotency_key));
        self.uploaded_keys
            .insert((account_id.clone(), idempotency_key.to_owned()));
    }

    /// Total pending outbox batches across all accounts.
    #[must_use]
    pub fn total_pending(&self) -> usize {
        self.pending.len()
    }
}

/// Receipt returned by idempotent server health-batch upload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthUploadReceipt {
    pub idempotency_key: String,
    pub deduplicated: bool,
    pub stored_count: usize,
}

/// Server-side idempotent health sample store isolated strictly per patient account.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MobileHealthIngestStore {
    samples_by_account: BTreeMap<MobileAccountId, Vec<HealthSampleEntry>>,
    processed_keys: BTreeSet<(MobileAccountId, String)>,
}

impl MobileHealthIngestStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ingests an account-fenced health batch idempotently.
    pub fn ingest_batch(
        &mut self,
        authenticated_account: &MobileAccountId,
        batch: AccountFencedHealthBatch,
    ) -> Result<HealthUploadReceipt, HealthSyncError> {
        if &batch.account_id != authenticated_account {
            return Err(HealthSyncError::AccountFenceMismatch {
                expected: authenticated_account.clone(),
                actual: batch.account_id,
            });
        }
        if batch.idempotency_key.trim().is_empty() {
            return Err(HealthSyncError::EmptyIdempotencyKey);
        }
        if batch.samples.is_empty() {
            return Err(HealthSyncError::EmptySamples);
        }

        let key = (authenticated_account.clone(), batch.idempotency_key.clone());
        if self.processed_keys.contains(&key) {
            return Ok(HealthUploadReceipt {
                idempotency_key: batch.idempotency_key,
                deduplicated: true,
                stored_count: 0,
            });
        }

        let stored_count = batch.samples.len();
        self.samples_by_account
            .entry(authenticated_account.clone())
            .or_default()
            .extend(batch.samples);
        self.processed_keys.insert(key);

        Ok(HealthUploadReceipt {
            idempotency_key: batch.idempotency_key,
            deduplicated: false,
            stored_count,
        })
    }

    /// Reads synced health trends/samples strictly for `account_id`.
    #[must_use]
    pub fn samples_for_account(&self, account_id: &MobileAccountId) -> &[HealthSampleEntry] {
        self.samples_by_account
            .get(account_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Purges all health samples for `account_id` on account deletion / `records_withdraw`.
    pub fn purge_account_samples(&mut self, account_id: &MobileAccountId) -> usize {
        self.processed_keys.retain(|(acct, _)| acct != account_id);
        self.samples_by_account
            .remove(account_id)
            .map_or(0, |v| v.len())
    }
}

/// Push notification delivery platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PushPlatform {
    IosApns,
    AndroidFcm,
}

/// Registered device push token bound to one patient account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevicePushRegistration {
    pub device_id: String,
    pub account_id: MobileAccountId,
    pub platform: PushPlatform,
    pub token: String,
    pub proactive_enabled: bool,
}

/// Registry managing APNs/FCM token registration, rotation, sign-out revocation,
/// and privacy withdrawal (`device_withdraw` / `proactive_withdraw`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PushTokenRegistry {
    registrations: BTreeMap<String, DevicePushRegistration>,
}

impl PushTokenRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers or rotates a device push token.
    pub fn register_or_rotate(&mut self, reg: DevicePushRegistration) {
        self.registrations.insert(reg.device_id.clone(), reg);
    }

    /// Revokes a specific device's push token (sign-out or `device_withdraw`).
    pub fn revoke_device(&mut self, device_id: &str) -> bool {
        self.registrations.remove(device_id).is_some()
    }

    /// Revokes all push tokens for an account (account deletion or full `device_withdraw`).
    pub fn revoke_all_for_account(&mut self, account_id: &MobileAccountId) -> usize {
        let before = self.registrations.len();
        self.registrations
            .retain(|_, r| &r.account_id != account_id);
        before.saturating_sub(self.registrations.len())
    }

    /// Disables proactive push notifications for an account (`proactive_withdraw`).
    pub fn disable_proactive_for_account(&mut self, account_id: &MobileAccountId) {
        for reg in self.registrations.values_mut() {
            if &reg.account_id == account_id {
                reg.proactive_enabled = false;
            }
        }
    }

    /// Lists active push registrations for `account_id`.
    #[must_use]
    pub fn active_for_account(&self, account_id: &MobileAccountId) -> Vec<&DevicePushRegistration> {
        self.registrations
            .values()
            .filter(|r| &r.account_id == account_id)
            .collect()
    }
}
