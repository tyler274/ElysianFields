//! Logical export/import rehearsal at full 468-patient synthetic cohort scale,
//! single-writer domain fencing, and mid-flight crash/resume idempotency.

use std::collections::{BTreeMap, BTreeSet};

use ef_clinic_domain::workspace::{
    CitationOrigin, CitedSourcePayload, MembershipTier, ReadablePart, WorkspaceEvidenceStore,
    classify_record_status,
};
use ef_connected_care::{
    ConnectedCareBridge, CreateInvitationRequest, DateOfBirth, InviteCredential, PairKey,
};
use ef_core::{Locale, PatientId, PhysicianId, PracticeId, SourceKey, WorkspaceId};
use ef_mobile_domain::{
    AccountIncarnation, ClinicalRecordBucket, ConfidentialityTag, ImportSourceKind,
    IncomingBatchItem, MobileAccountId, PatientRecord, PatientRecordStore, PortalVendor,
};
use ef_patient_bank::{
    COHORT_TOTAL_PATIENTS, DEFAULT_AS_OF_DATE, DEFAULT_COHORT_SEED, RenderedPatientBundle,
    generate_synthetic_cohort, sha256_hex,
};
use serde::{Deserialize, Serialize};

use crate::RehearsalError;

/// Storage domain protected by an exclusive single-writer lease fence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageDomain {
    CurieClinic,
    CurieMobile,
    ConnectedCare,
}

impl StorageDomain {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CurieClinic => "curie_clinic",
            Self::CurieMobile => "curie_mobile",
            Self::ConnectedCare => "connected_care",
        }
    }
}

/// Active writer lease token required before mutating a domain during migration/cutover.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriterLeaseToken {
    pub domain: StorageDomain,
    pub holder_id: String,
    pub epoch: u64,
}

/// Single-writer domain fence rejecting concurrent or stale split-brain writers.
#[derive(Debug, Clone, Default)]
pub struct DomainWriterFence {
    active: BTreeMap<StorageDomain, WriterLeaseToken>,
}

impl DomainWriterFence {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Transfers exclusive write ownership for `domain` to `holder_id`, incrementing epoch monotonically.
    pub fn acquire_exclusive(
        &mut self,
        domain: StorageDomain,
        holder_id: impl Into<String>,
    ) -> WriterLeaseToken {
        let next_epoch = self
            .active
            .get(&domain)
            .map_or(1, |t| t.epoch.saturating_add(1));
        let token = WriterLeaseToken {
            domain,
            holder_id: holder_id.into(),
            epoch: next_epoch,
        };
        self.active.insert(domain, token.clone());
        token
    }

    /// Validates that `caller` currently holds the active exclusive writer lease for `caller.domain`.
    pub fn verify_writer(&self, caller: &WriterLeaseToken) -> Result<(), RehearsalError> {
        match self.active.get(&caller.domain) {
            Some(active)
                if active.holder_id == caller.holder_id && active.epoch == caller.epoch =>
            {
                Ok(())
            }
            Some(active) => Err(RehearsalError::SplitBrainWriteRejected {
                domain: caller.domain.as_str(),
                active_holder: active.holder_id.clone(),
                active_epoch: active.epoch,
                caller_holder: caller.holder_id.clone(),
                caller_epoch: caller.epoch,
            }),
            None => Err(RehearsalError::SplitBrainWriteRejected {
                domain: caller.domain.as_str(),
                active_holder: "<unclaimed>".to_owned(),
                active_epoch: 0,
                caller_holder: caller.holder_id.clone(),
                caller_epoch: caller.epoch,
            }),
        }
    }
}

/// One exported patient entry spanning Clinic panel assignment, Mobile account, and Connected Care flag.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedCohortRecord {
    pub patient_key: String,
    pub practice_id: PracticeId,
    pub assigned_physician_id: PhysicianId,
    pub workspace_id: WorkspaceId,
    pub mobile_account_id: MobileAccountId,
    pub connected_care_linked: bool,
    pub canonical_bundle_json: Vec<u8>,
    pub bundle_sha256: String,
    pub resource_row_count: usize,
    pub birth_date: String,
    pub display_name: String,
}

/// Complete logical export bundle for the 468-patient synthetic cohort.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogicalExportBundle {
    pub export_id: String,
    pub cohort_seed: u64,
    pub as_of_date: String,
    pub records: Vec<ExportedCohortRecord>,
    pub total_resource_rows: usize,
    pub manifest_sha256: String,
}

/// Generates a deterministic 468-patient logical export bundle from `ef-patient-bank`.
pub fn build_468_patient_logical_export() -> Result<LogicalExportBundle, RehearsalError> {
    let (bank, _ledger) = generate_synthetic_cohort(DEFAULT_COHORT_SEED, DEFAULT_AS_OF_DATE);
    let prac_a = PracticeId::new("prac-alpha").map_err(|e| RehearsalError::DomainFailure {
        reason: e.to_string(),
    })?;
    let prac_b = PracticeId::new("prac-beta").map_err(|e| RehearsalError::DomainFailure {
        reason: e.to_string(),
    })?;
    let dr_a1 = PhysicianId::new("phys-alpha-1").map_err(|e| RehearsalError::DomainFailure {
        reason: e.to_string(),
    })?;
    let dr_a2 = PhysicianId::new("phys-alpha-2").map_err(|e| RehearsalError::DomainFailure {
        reason: e.to_string(),
    })?;
    let dr_b1 = PhysicianId::new("phys-beta-1").map_err(|e| RehearsalError::DomainFailure {
        reason: e.to_string(),
    })?;

    let mut records = Vec::with_capacity(bank.patients.len());
    let mut total_resource_rows = 0usize;
    let mut manifest_hash_input = Vec::new();

    for (idx, (patient_key, bundle)) in bank.patients.iter().enumerate() {
        let (practice_id, assigned_physician_id) = match idx % 3 {
            0 => (prac_a.clone(), dr_a1.clone()),
            1 => (prac_a.clone(), dr_a2.clone()),
            _ => (prac_b.clone(), dr_b1.clone()),
        };

        let workspace_id = WorkspaceId::new(format!("ws-{patient_key}")).map_err(|e| {
            RehearsalError::DomainFailure {
                reason: e.to_string(),
            }
        })?;
        let mobile_account_id =
            MobileAccountId::new(format!("mob-{patient_key}")).map_err(|e| {
                RehearsalError::DomainFailure {
                    reason: e.to_string(),
                }
            })?;

        let canonical_bundle_json =
            serde_json::to_vec(bundle).map_err(|e| RehearsalError::DomainFailure {
                reason: e.to_string(),
            })?;
        let bundle_sha256 = sha256_hex(&canonical_bundle_json);
        let row_count = count_bundle_rows(bundle);
        total_resource_rows = total_resource_rows.saturating_add(row_count);

        manifest_hash_input.extend_from_slice(patient_key.as_bytes());
        manifest_hash_input.push(b':');
        manifest_hash_input.extend_from_slice(bundle_sha256.as_bytes());
        manifest_hash_input.push(b'\n');

        records.push(ExportedCohortRecord {
            patient_key: patient_key.clone(),
            practice_id,
            assigned_physician_id,
            workspace_id,
            mobile_account_id,
            // Link 10% of synthetic cohort (47 patients) across Connected Care for bridge verification.
            connected_care_linked: idx.is_multiple_of(10),
            canonical_bundle_json,
            bundle_sha256,
            resource_row_count: row_count,
            birth_date: bundle.patient.birth_date.clone(),
            display_name: bundle.patient.synthetic_display_name.clone(),
        });
    }

    let manifest_sha256 = sha256_hex(&manifest_hash_input);
    Ok(LogicalExportBundle {
        export_id: "m4-cohort-468-v1".to_owned(),
        cohort_seed: DEFAULT_COHORT_SEED,
        as_of_date: DEFAULT_AS_OF_DATE.to_owned(),
        records,
        total_resource_rows,
        manifest_sha256,
    })
}

fn count_bundle_rows(bundle: &RenderedPatientBundle) -> usize {
    1 + bundle.conditions.len()
        + bundle.medications.len()
        + bundle.observations.len()
        + bundle.allergies.len()
        + bundle.immunizations.len()
        + bundle.procedures.len()
        + bundle.encounters.len()
        + bundle.diagnostic_reports.len()
        + bundle.documents.len()
        + bundle.care_plans.len()
}

/// Resumable checkpoint recording committed patients across interruptions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportCheckpoint {
    pub committed_patient_keys: BTreeSet<String>,
    pub committed_resource_rows: usize,
    pub connected_care_links_established: usize,
    pub duplicates_skipped: usize,
    pub resume_cycles: u32,
}

/// Target stores populated by the logical importer across Clinic, Mobile, and Connected Care.
#[derive(Debug, Default)]
pub struct MigrationTargetStores {
    pub clinic_store: WorkspaceEvidenceStore,
    pub mobile_records: PatientRecordStore,
    pub connected_care: ConnectedCareBridge,
    pub imported_checksums: BTreeMap<String, String>,
}

/// Final parity verification report proving zero row loss, zero duplicates, panel preservation, and SHA-256 parity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationParityReport {
    pub expected_patients: usize,
    pub imported_patients: usize,
    pub expected_resource_rows: usize,
    pub imported_resource_rows: usize,
    pub connected_care_links_verified: usize,
    pub panel_assignments_verified: usize,
    pub duplicates_skipped: usize,
    pub resume_cycles: u32,
    pub source_manifest_sha256: String,
    pub target_manifest_sha256: String,
    pub checksum_parity_matched: bool,
}

/// Bundled exclusive writer leases across all three migrated storage domains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainLeaseSet {
    pub clinic: WriterLeaseToken,
    pub mobile: WriterLeaseToken,
    pub connected_care: WriterLeaseToken,
}

/// Executes or resumes a fenced logical import into `target`, optionally simulating a mid-flight crash
/// after committing `crash_after_new_commits` patients to rehearse checkpoint resume and idempotency.
pub fn run_fenced_logical_import(
    bundle: &LogicalExportBundle,
    fence: &DomainWriterFence,
    leases: &DomainLeaseSet,
    target: &mut MigrationTargetStores,
    checkpoint: &mut ImportCheckpoint,
    crash_after_new_commits: Option<usize>,
) -> Result<(), RehearsalError> {
    fence.verify_writer(&leases.clinic)?;
    fence.verify_writer(&leases.mobile)?;
    fence.verify_writer(&leases.connected_care)?;

    checkpoint.resume_cycles = checkpoint.resume_cycles.saturating_add(1);
    let mut newly_committed = 0usize;

    for rec in &bundle.records {
        // Verify source integrity before importing.
        let computed = sha256_hex(&rec.canonical_bundle_json);
        if computed != rec.bundle_sha256 {
            return Err(RehearsalError::ChecksumMismatch {
                subject: rec.patient_key.clone(),
                expected: rec.bundle_sha256.clone(),
                actual: computed,
            });
        }

        // Idempotent resume: already-committed patients are skipped cleanly with zero duplication.
        if checkpoint.committed_patient_keys.contains(&rec.patient_key) {
            checkpoint.duplicates_skipped = checkpoint.duplicates_skipped.saturating_add(1);
            continue;
        }

        if let Some(limit) = crash_after_new_commits
            && newly_committed >= limit
        {
            return Err(RehearsalError::SimulatedMidFlightCrash {
                committed_count: checkpoint.committed_patient_keys.len(),
            });
        }

        // 1. Clinic domain workspace & panel registration + lossless source record.
        target.clinic_store.register_workspace(
            rec.workspace_id.clone(),
            rec.practice_id.clone(),
            rec.display_name.clone(),
            vec![rec.assigned_physician_id.clone()],
        );
        let source_key =
            SourceKey::new(format!("src-bundle-{}", rec.patient_key)).map_err(|e| {
                RehearsalError::DomainFailure {
                    reason: e.to_string(),
                }
            })?;
        target.clinic_store.insert_source(CitedSourcePayload {
            source_key,
            workspace_id: rec.workspace_id.clone(),
            number: 1,
            source_label: format!("Migrated FHIR Bundle ({})", rec.patient_key),
            title: format!("Logical Export Bundle — {}", rec.display_name),
            record_type: "fhir_r4_bundle".to_owned(),
            recorded_at: bundle.as_of_date.clone(),
            record_status: "final".to_owned(),
            status_class: classify_record_status("final"),
            origin: CitationOrigin::SourceRecord,
            original_language: Locale::En,
            quote: format!("sha256:{}", rec.bundle_sha256),
            body: format!("{} clinical resource rows", rec.resource_row_count),
            readable_parts: vec![ReadablePart {
                label: "Manifest Digest".to_owned(),
                text: rec.bundle_sha256.clone(),
                unreadable_notice: None,
            }],
            fields: Vec::new(),
            raw_content: None,
            external_url: None,
        });

        // 2. Curie Mobile patient-owned record admission via per-record savepoints.
        let batch_report =
            target
                .mobile_records
                .ingest_batch_with_savepoints(vec![IncomingBatchItem::Valid(PatientRecord {
                    record_id: format!("mob-rec-{}", rec.patient_key),
                    version: 1,
                    account_id: rec.mobile_account_id.clone(),
                    incarnation: AccountIncarnation::first(),
                    source_kind: ImportSourceKind::SmartFhirR4 {
                        vendor: PortalVendor::CustomSmartFhirR4,
                    },
                    portal_id: None,
                    bucket: ClinicalRecordBucket::Patient,
                    confidentiality: ConfidentialityTag::Standard,
                    part2_notice: None,
                    original_date: bundle.as_of_date.clone(),
                    title: rec.display_name.clone(),
                    summary: format!("{} migrated FHIR rows", rec.resource_row_count),
                    native_bytes: rec.canonical_bundle_json.clone(),
                    removed_upstream: false,
                })]);
        if !batch_report.failed_items.is_empty() {
            return Err(RehearsalError::DomainFailure {
                reason: format!("mobile savepoint import failed for {}", rec.patient_key),
            });
        }

        // 3. Connected Care active link recreation where applicable.
        if rec.connected_care_linked {
            let patient_id = PatientId::new(rec.patient_key.clone()).map_err(|e| {
                RehearsalError::DomainFailure {
                    reason: e.to_string(),
                }
            })?;
            let pair = PairKey {
                practice_id: rec.practice_id.clone(),
                practice_patient_id: patient_id,
            };
            let dob = parse_yyyy_mm_dd(&rec.birth_date)?;
            let issued = target
                .connected_care
                .create_invitation(CreateInvitationRequest {
                    pair,
                    issued_by: rec.assigned_physician_id.clone(),
                    practice_name: format!("Practice {}", rec.practice_id.as_str()),
                    physician_name: format!("Dr. {}", rec.assigned_physician_id.as_str()),
                    expected_dob: dob,
                    created_at: "2026-06-15T00:00:00Z".to_owned(),
                })
                .map_err(|e| RehearsalError::DomainFailure {
                    reason: e.to_string(),
                })?;
            let cred = InviteCredential::LinkToken(issued.link_token);
            let bridge_account = ef_connected_care::MobileAccountId::new(
                rec.mobile_account_id.as_str(),
            )
            .map_err(|e| RehearsalError::DomainFailure {
                reason: e.to_string(),
            })?;
            let _ = target
                .connected_care
                .verify_dob(&cred, &bridge_account, dob, "2026-06-15T00:01:00Z")
                .map_err(|e| RehearsalError::DomainFailure {
                    reason: e.to_string(),
                })?;
            let _ = target
                .connected_care
                .confirm_share(&cred, &bridge_account, "2026-06-15T00:02:00Z")
                .map_err(|e| RehearsalError::DomainFailure {
                    reason: e.to_string(),
                })?;
            checkpoint.connected_care_links_established = checkpoint
                .connected_care_links_established
                .saturating_add(1);
        }

        target
            .imported_checksums
            .insert(rec.patient_key.clone(), computed);
        checkpoint
            .committed_patient_keys
            .insert(rec.patient_key.clone());
        checkpoint.committed_resource_rows = checkpoint
            .committed_resource_rows
            .saturating_add(rec.resource_row_count);
        newly_committed = newly_committed.saturating_add(1);
    }

    Ok(())
}

/// Verifies complete 468-patient parity: zero row loss, SHA-256 manifest match, panel preservation,
/// and Connected Care invariant audit.
pub fn verify_migration_parity(
    bundle: &LogicalExportBundle,
    target: &MigrationTargetStores,
    checkpoint: &ImportCheckpoint,
) -> Result<MigrationParityReport, RehearsalError> {
    let mut target_hash_input = Vec::new();
    let mut panel_assignments_verified = 0usize;
    let mut connected_care_links_verified = 0usize;

    for rec in &bundle.records {
        // Verify Clinic panel assignment preserved strictly as `Allowed`.
        let tier = target
            .clinic_store
            .membership_for(
                &rec.practice_id,
                &rec.assigned_physician_id,
                &rec.workspace_id,
            )
            .map_err(|e| RehearsalError::DomainFailure {
                reason: e.to_string(),
            })?;
        if tier != MembershipTier::Allowed {
            return Err(RehearsalError::DomainFailure {
                reason: format!(
                    "panel assignment lost for {}: expected Allowed, got {tier:?}",
                    rec.patient_key
                ),
            });
        }
        panel_assignments_verified = panel_assignments_verified.saturating_add(1);

        // Verify Mobile native bytes SHA-256 parity.
        let mobile_recs = target
            .mobile_records
            .records_for_account(&rec.mobile_account_id);
        if mobile_recs.len() != 1 {
            return Err(RehearsalError::DomainFailure {
                reason: format!(
                    "expected exactly 1 migrated mobile record for {}, found {}",
                    rec.patient_key,
                    mobile_recs.len()
                ),
            });
        }
        let actual_sha = sha256_hex(&mobile_recs[0].native_bytes);
        if actual_sha != rec.bundle_sha256 {
            return Err(RehearsalError::ChecksumMismatch {
                subject: rec.patient_key.clone(),
                expected: rec.bundle_sha256.clone(),
                actual: actual_sha,
            });
        }

        if rec.connected_care_linked {
            let patient_id = PatientId::new(rec.patient_key.clone()).map_err(|e| {
                RehearsalError::DomainFailure {
                    reason: e.to_string(),
                }
            })?;
            let pair = PairKey {
                practice_id: rec.practice_id.clone(),
                practice_patient_id: patient_id,
            };
            let standing = target.connected_care.clinic_standing(&pair);
            if standing.state != ef_connected_care::ConnectedCareState::Connected {
                return Err(RehearsalError::DomainFailure {
                    reason: format!("connected care link missing for {}", rec.patient_key),
                });
            }
            connected_care_links_verified = connected_care_links_verified.saturating_add(1);
        }

        target_hash_input.extend_from_slice(rec.patient_key.as_bytes());
        target_hash_input.push(b':');
        target_hash_input.extend_from_slice(actual_sha.as_bytes());
        target_hash_input.push(b'\n');
    }

    // Verify all 11 Connected Care bridge invariants hold on the imported cohort.
    let _audit = target
        .connected_care
        .audit_all_invariants()
        .map_err(|violations| RehearsalError::DomainFailure {
            reason: format!("connected care invariant violations: {violations:?}"),
        })?;

    let target_manifest_sha256 = sha256_hex(&target_hash_input);
    let checksum_parity_matched = target_manifest_sha256 == bundle.manifest_sha256
        && target.imported_checksums.len() == COHORT_TOTAL_PATIENTS;

    if !checksum_parity_matched {
        return Err(RehearsalError::ChecksumMismatch {
            subject: bundle.export_id.clone(),
            expected: bundle.manifest_sha256.clone(),
            actual: target_manifest_sha256,
        });
    }

    Ok(MigrationParityReport {
        expected_patients: bundle.records.len(),
        imported_patients: target.imported_checksums.len(),
        expected_resource_rows: bundle.total_resource_rows,
        imported_resource_rows: checkpoint.committed_resource_rows,
        connected_care_links_verified,
        panel_assignments_verified,
        duplicates_skipped: checkpoint.duplicates_skipped,
        resume_cycles: checkpoint.resume_cycles,
        source_manifest_sha256: bundle.manifest_sha256.clone(),
        target_manifest_sha256,
        checksum_parity_matched,
    })
}

fn parse_yyyy_mm_dd(raw: &str) -> Result<DateOfBirth, RehearsalError> {
    let mut parts = raw.split('-');
    let year: u16 =
        parts
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| RehearsalError::DomainFailure {
                reason: format!("invalid birth_date year in '{raw}'"),
            })?;
    let month: u8 =
        parts
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| RehearsalError::DomainFailure {
                reason: format!("invalid birth_date month in '{raw}'"),
            })?;
    let day: u8 =
        parts
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| RehearsalError::DomainFailure {
                reason: format!("invalid birth_date day in '{raw}'"),
            })?;
    DateOfBirth::new(year, month, day).map_err(|e| RehearsalError::DomainFailure {
        reason: e.to_string(),
    })
}
