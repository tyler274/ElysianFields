//! Validation and idempotent runtime cohort loader with indexed queries.

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use crate::fhir::{
    AttachmentPageExtraction, BinaryAttachmentKind, ComplexityTier, FhirAppointment,
    FhirResourceFamily, RenderedPatientBundle, RenderedRuntimeBank, ResultAvailability,
    extract_pdf_pages,
};
use crate::generator::{
    COHORT_COMPLEX_COUNT, COHORT_FLAGSHIP_COUNT, COHORT_LIGHT_COUNT, COHORT_MODERATE_COUNT,
    COHORT_TOTAL_PATIENTS, sha256_hex,
};

/// Verified summary returned by [`validate_rendered_bank`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankValidationSummary {
    pub total_patients: usize,
    pub light_count: usize,
    pub moderate_count: usize,
    pub complex_count: usize,
    pub flagship_count: usize,
    pub calendar_appointments: usize,
    pub pending_post_as_of_observations: usize,
    pub flagship_min_note_bytes: usize,
}

/// Validation errors reported when a synthetic cohort bank violates M3 §4.2 invariants.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BankValidationError {
    #[error("unexpected cohort patient count: expected {expected}, got {actual}")]
    WrongPatientCount { expected: usize, actual: usize },
    #[error(
        "unexpected complexity tier counts (light={light}, moderate={moderate}, complex={complex}, flagship={flagship})"
    )]
    WrongTierCounts {
        light: usize,
        moderate: usize,
        complex: usize,
        flagship: usize,
    },
    #[error("patient '{patient_id}' is missing clinical FHIR family {family:?}")]
    MissingClinicalFamily {
        patient_id: String,
        family: FhirResourceFamily,
    },
    #[error("physiological invariant violated for '{patient_id}': {detail}")]
    PhysiologicalViolation { patient_id: String, detail: String },
    #[error("flagship patient '{patient_id}' progress note is {bytes} bytes (< 8192 bytes)")]
    FlagshipNoteTooSmall { patient_id: String, bytes: usize },
    #[error("flagship patient '{patient_id}' corrupted PDF isolation check failed")]
    CorruptedPdfIsolationFailed { patient_id: String },
    #[error("duplicate >=40-char narrative boilerplate detected across charts: '{line}'")]
    DuplicateNarrativeBoilerplate { line: String },
    #[error("manifest SHA-256 mismatch for '{path}'")]
    ManifestDigestMismatch { path: String },
}

/// Validates every M3 §4.2 / `SCN-M3-0016` invariant across a [`RenderedRuntimeBank`].
pub fn validate_rendered_bank(
    bank: &RenderedRuntimeBank,
) -> Result<BankValidationSummary, Vec<BankValidationError>> {
    let mut errors = Vec::new();
    if bank.patients.len() != COHORT_TOTAL_PATIENTS {
        errors.push(BankValidationError::WrongPatientCount {
            expected: COHORT_TOTAL_PATIENTS,
            actual: bank.patients.len(),
        });
    }

    let mut light = 0usize;
    let mut moderate = 0usize;
    let mut complex = 0usize;
    let mut flagship = 0usize;
    let mut pending_post_as_of = 0usize;
    let mut flagship_min_note_bytes = usize::MAX;
    let mut seen_long_lines = BTreeSet::new();

    for (patient_id, bundle) in &bank.patients {
        match bundle.patient.tier {
            ComplexityTier::Light => light += 1,
            ComplexityTier::Moderate => moderate += 1,
            ComplexityTier::Complex => complex += 1,
        }
        if bundle.patient.flagship.is_some() {
            flagship += 1;
        }

        // Verify all 11 FHIR R4 clinical resource families exist.
        let families = bundle.populated_families();
        for required in FhirResourceFamily::CLINICAL_ELEVEN {
            if !families.contains(&required) {
                errors.push(BankValidationError::MissingClinicalFamily {
                    patient_id: patient_id.clone(),
                    family: required,
                });
            }
        }

        // Verify physiological lab consistency.
        let mut sbp = None;
        let mut dbp = None;
        let mut hba1c = None;
        for obs in &bundle.observations {
            if obs.availability == ResultAvailability::PendingPostAsOf {
                pending_post_as_of += 1;
            }
            match obs.loinc_code.as_str() {
                "8480-6" => sbp = Some(obs.value),
                "8462-4" => dbp = Some(obs.value),
                "4548-4" => hba1c = Some(obs.value),
                _ => {}
            }
        }
        if let (Some(s), Some(d)) = (sbp, dbp)
            && s <= d
        {
            errors.push(BankValidationError::PhysiologicalViolation {
                patient_id: patient_id.clone(),
                detail: format!("systolic ({s}) <= diastolic ({d})"),
            });
        }
        let has_diabetes = bundle
            .conditions
            .iter()
            .any(|c| c.icd10_code.starts_with("E11"));
        if let Some(a1c) = hba1c {
            if has_diabetes && a1c < 6.5 {
                errors.push(BankValidationError::PhysiologicalViolation {
                    patient_id: patient_id.clone(),
                    detail: format!("diabetic patient has HbA1c {a1c} < 6.5"),
                });
            }
            if !has_diabetes && a1c >= 6.5 {
                errors.push(BankValidationError::PhysiologicalViolation {
                    patient_id: patient_id.clone(),
                    detail: format!("non-diabetic patient has HbA1c {a1c} >= 6.5"),
                });
            }
        }

        // Verify zero repeated >= 40-char narrative lines and > 8 KB flagship notes + isolated corrupt PDFs.
        let mut total_note_bytes = 0usize;
        let mut found_isolated_corrupt_pdf = false;
        for doc in &bundle.documents {
            for sec in &doc.sections {
                total_note_bytes += sec.heading.len() + sec.body.len();
                for line in sec.body.lines() {
                    let trimmed = line.trim();
                    if trimmed.len() >= 40 && !seen_long_lines.insert(trimmed.to_owned()) {
                        errors.push(BankValidationError::DuplicateNarrativeBoilerplate {
                            line: trimmed.to_owned(),
                        });
                    }
                }
            }
            for att in &doc.attachments {
                if att.kind == BinaryAttachmentKind::CorruptedSinglePagePdf {
                    let pages = extract_pdf_pages(att);
                    let has_readable_sibling = pages
                        .iter()
                        .any(|p| matches!(p, AttachmentPageExtraction::Readable { .. }));
                    let has_corrupt_page = pages
                        .iter()
                        .any(|p| matches!(p, AttachmentPageExtraction::Corrupted { .. }));
                    if has_readable_sibling && has_corrupt_page {
                        found_isolated_corrupt_pdf = true;
                    }
                }
            }
        }

        if bundle.patient.flagship.is_some() {
            flagship_min_note_bytes = flagship_min_note_bytes.min(total_note_bytes);
            if total_note_bytes < 8_192 {
                errors.push(BankValidationError::FlagshipNoteTooSmall {
                    patient_id: patient_id.clone(),
                    bytes: total_note_bytes,
                });
            }
            if !found_isolated_corrupt_pdf {
                errors.push(BankValidationError::CorruptedPdfIsolationFailed {
                    patient_id: patient_id.clone(),
                });
            }
        }

        // Verify manifest file digest integrity.
        let expected_path = format!("patients/{patient_id}.json");
        let bytes = serde_json::to_vec(bundle).expect("serializable");
        let actual_digest = sha256_hex(&bytes);
        if bank.manifest.file_digests.get(&expected_path) != Some(&actual_digest) {
            errors.push(BankValidationError::ManifestDigestMismatch {
                path: expected_path,
            });
        }
    }

    if light != COHORT_LIGHT_COUNT
        || moderate != COHORT_MODERATE_COUNT
        || complex != COHORT_COMPLEX_COUNT
        || flagship != COHORT_FLAGSHIP_COUNT
    {
        errors.push(BankValidationError::WrongTierCounts {
            light,
            moderate,
            complex,
            flagship,
        });
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(BankValidationSummary {
        total_patients: bank.patients.len(),
        light_count: light,
        moderate_count: moderate,
        complex_count: complex,
        flagship_count: flagship,
        calendar_appointments: bank.calendar.len(),
        pending_post_as_of_observations: pending_post_as_of,
        flagship_min_note_bytes: if flagship_min_note_bytes == usize::MAX {
            0
        } else {
            flagship_min_note_bytes
        },
    })
}

/// Summary reported by [`RuntimeBankLoader::import_rendered_bank`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportReport {
    pub inserted_records: usize,
    pub changed_records: usize,
    pub unchanged_records: usize,
    pub appointments_loaded: usize,
}

/// Runtime-only FHIR cohort loader & indexed query store.
///
/// Accepts ONLY [`RenderedRuntimeBank`] (never [`crate::fhir::AuthoringLedger`]),
/// guarantees idempotent re-imports (`0` changed records on repeat import), and preserves
/// physician panel assignments across re-imports.
#[derive(Debug, Default)]
pub struct RuntimeBankLoader {
    bundles: BTreeMap<String, (String, RenderedPatientBundle)>,
    appointments: BTreeMap<String, FhirAppointment>,
    panel_assignments: BTreeMap<String, String>,
    tier_index: BTreeMap<ComplexityTier, Vec<String>>,
    condition_index: BTreeMap<String, Vec<String>>,
}

impl RuntimeBankLoader {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Assigns a patient to a clinician panel (preserved across idempotent bank re-imports).
    pub fn assign_panel(&mut self, patient_id: impl Into<String>, physician_id: impl Into<String>) {
        self.panel_assignments
            .insert(patient_id.into(), physician_id.into());
    }

    #[must_use]
    pub fn panel_assignment(&self, patient_id: &str) -> Option<&str> {
        self.panel_assignments.get(patient_id).map(String::as_str)
    }

    /// Idempotently loads a [`RenderedRuntimeBank`] into runtime storage.
    pub fn import_rendered_bank(&mut self, bank: &RenderedRuntimeBank) -> ImportReport {
        let mut inserted_records = 0usize;
        let mut changed_records = 0usize;
        let mut unchanged_records = 0usize;

        for (patient_id, bundle) in &bank.patients {
            let digest = bank
                .manifest
                .file_digests
                .get(&format!("patients/{patient_id}.json"))
                .cloned()
                .unwrap_or_else(|| sha256_hex(&serde_json::to_vec(bundle).expect("serializable")));

            match self.bundles.get(patient_id) {
                Some((existing_digest, _)) if existing_digest == &digest => {
                    unchanged_records += 1;
                }
                Some(_) => {
                    self.bundles
                        .insert(patient_id.clone(), (digest, bundle.clone()));
                    changed_records += 1;
                }
                None => {
                    self.bundles
                        .insert(patient_id.clone(), (digest, bundle.clone()));
                    inserted_records += 1;
                }
            }
        }

        for appt in &bank.calendar {
            self.appointments.insert(appt.id.clone(), appt.clone());
        }

        if inserted_records > 0 || changed_records > 0 {
            self.rebuild_indexes();
        }

        ImportReport {
            inserted_records,
            changed_records,
            unchanged_records,
            appointments_loaded: self.appointments.len(),
        }
    }

    #[must_use]
    pub fn patient(&self, patient_id: &str) -> Option<&RenderedPatientBundle> {
        self.bundles.get(patient_id).map(|(_, b)| b)
    }

    #[must_use]
    pub fn patients_by_tier(&self, tier: ComplexityTier) -> &[String] {
        self.tier_index.get(&tier).map_or(&[], Vec::as_slice)
    }

    #[must_use]
    pub fn patients_by_condition_code(&self, icd10_code: &str) -> &[String] {
        self.condition_index
            .get(icd10_code)
            .map_or(&[], Vec::as_slice)
    }

    /// Queries patients whose `as_of`-visible lab observation for `loinc_code` falls in `[min, max]`.
    #[must_use]
    pub fn patients_by_lab_range(
        &self,
        loinc_code: &str,
        min_inclusive: f64,
        max_inclusive: f64,
        as_of_date: &str,
    ) -> Vec<String> {
        let mut matches = Vec::new();
        for (patient_id, (_, bundle)) in &self.bundles {
            let hit = bundle
                .observations_visible_at(as_of_date)
                .iter()
                .any(|obs| {
                    obs.loinc_code == loinc_code
                        && obs.value >= min_inclusive
                        && obs.value <= max_inclusive
                });
            if hit {
                matches.push(patient_id.clone());
            }
        }
        matches
    }

    fn rebuild_indexes(&mut self) {
        self.tier_index.clear();
        self.condition_index.clear();
        for (patient_id, (_, bundle)) in &self.bundles {
            self.tier_index
                .entry(bundle.patient.tier)
                .or_default()
                .push(patient_id.clone());
            for cond in &bundle.conditions {
                self.condition_index
                    .entry(cond.icd10_code.clone())
                    .or_default()
                    .push(patient_id.clone());
            }
        }
    }
}
