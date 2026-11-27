//! FHIR R4 resource families, deterministic binary attachments, and runtime bundle structures.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Complexity tier distribution mandated by M3 §4.2 (`60` Light + `122` Moderate + `286` Complex = `468`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComplexityTier {
    Light,
    Moderate,
    Complex,
}

/// Five flagship primary-care charts carrying >8 KB multi-section notes and binary attachments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlagshipUnit {
    Unit1,
    Unit2,
    Unit3,
    Unit4,
    Unit5,
}

/// All 11 FHIR R4 clinical resource families + future `Appointment` calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum FhirResourceFamily {
    Patient,
    Condition,
    MedicationRequest,
    Observation,
    AllergyIntolerance,
    Immunization,
    Procedure,
    Encounter,
    DiagnosticReport,
    DocumentReference,
    CarePlan,
    Appointment,
}

impl FhirResourceFamily {
    /// All 11 clinical FHIR R4 resource families inside patient bundles.
    pub const CLINICAL_ELEVEN: [Self; 11] = [
        Self::Patient,
        Self::Condition,
        Self::MedicationRequest,
        Self::Observation,
        Self::AllergyIntolerance,
        Self::Immunization,
        Self::Procedure,
        Self::Encounter,
        Self::DiagnosticReport,
        Self::DocumentReference,
        Self::CarePlan,
    ];
}

/// Visibility status relative to the bank's `as_of_date` cutoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultAvailability {
    AvailableAtAsOf,
    PendingPostAsOf,
}

/// Synthetic FHIR R4 `Patient` resource (strictly synthetic, zero PHI).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FhirPatient {
    pub id: String,
    pub synthetic_display_name: String,
    pub birth_date: String,
    pub sex_at_birth: String,
    pub tier: ComplexityTier,
    pub flagship: Option<FlagshipUnit>,
}

/// Synthetic FHIR R4 `Condition`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FhirCondition {
    pub id: String,
    pub icd10_code: String,
    pub display: String,
    pub clinical_status: String,
    pub onset_date: String,
}

/// Synthetic FHIR R4 `MedicationRequest`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FhirMedicationRequest {
    pub id: String,
    pub rxnorm_code: String,
    pub medication_name: String,
    pub dosage_instruction: String,
    pub status: String,
}

/// Synthetic FHIR R4 `Observation` (vitals and physiologically consistent lab markers).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FhirObservation {
    pub id: String,
    pub loinc_code: String,
    pub display: String,
    pub value: f64,
    pub unit: String,
    pub effective_date: String,
    pub availability: ResultAvailability,
}

/// Synthetic FHIR R4 `AllergyIntolerance`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FhirAllergyIntolerance {
    pub id: String,
    pub substance: String,
    pub reaction: String,
    pub clinical_status: String,
}

/// Synthetic FHIR R4 `Immunization`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FhirImmunization {
    pub id: String,
    pub vaccine_name: String,
    pub occurrence_date: String,
}

/// Synthetic FHIR R4 `Procedure`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FhirProcedure {
    pub id: String,
    pub cpt_code: String,
    pub display: String,
    pub performed_date: String,
}

/// Synthetic FHIR R4 `Encounter`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FhirEncounter {
    pub id: String,
    pub encounter_class: String,
    pub period_start: String,
    pub reason: String,
}

/// Synthetic FHIR R4 `DiagnosticReport`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FhirDiagnosticReport {
    pub id: String,
    pub title: String,
    pub issued_date: String,
    pub conclusion: String,
    pub availability: ResultAvailability,
}

/// Deterministic binary attachment kinds exercised by M3 §4.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BinaryAttachmentKind {
    ValidMultiPagePdf,
    CorruptedSinglePagePdf,
    PngScan,
    JpegFaxScan,
}

/// Deterministic synthetic binary attachment payload carried inside `DocumentReference`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinaryAttachment {
    pub attachment_id: String,
    pub content_type: String,
    pub kind: BinaryAttachmentKind,
    pub bytes: Vec<u8>,
}

/// Per-page decode outcome ensuring corrupted PDF pages fail in isolation without hiding siblings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachmentPageExtraction {
    Readable { page_number: u32, text: String },
    Corrupted { page_number: u32, reason: String },
}

/// Extracts pages from a deterministic synthetic PDF attachment, isolating corrupted pages.
#[must_use]
pub fn extract_pdf_pages(attachment: &BinaryAttachment) -> Vec<AttachmentPageExtraction> {
    let text = String::from_utf8_lossy(&attachment.bytes);
    let mut pages = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("%PAGE:")
            && let Some((num_str, body)) = rest.split_once(':')
            && let Ok(page_number) = num_str.parse::<u32>()
        {
            if let Some(reason) = body.strip_prefix("CORRUPTED|") {
                pages.push(AttachmentPageExtraction::Corrupted {
                    page_number,
                    reason: reason.to_owned(),
                });
            } else {
                pages.push(AttachmentPageExtraction::Readable {
                    page_number,
                    text: body.to_owned(),
                });
            }
        }
    }
    pages
}

/// Synthetic FHIR R4 `DocumentReference` (progress notes + deterministic binary attachments).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FhirDocumentReference {
    pub id: String,
    pub title: String,
    pub date: String,
    pub sections: Vec<DocumentSection>,
    pub attachments: Vec<BinaryAttachment>,
}

/// One section of a multi-section progress note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentSection {
    pub heading: String,
    pub body: String,
}

/// Synthetic FHIR R4 `CarePlan`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FhirCarePlan {
    pub id: String,
    pub title: String,
    pub status: String,
    pub goals: Vec<String>,
}

/// Synthetic FHIR R4 `Appointment` stored in the separate `calendar/` directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FhirAppointment {
    pub id: String,
    pub patient_id: String,
    pub scheduled_date: String,
    pub visit_type: String,
    pub status: String,
}

/// Complete rendered FHIR R4 clinical chart bundle for one synthetic patient (`patients/<id>.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderedPatientBundle {
    pub patient: FhirPatient,
    pub conditions: Vec<FhirCondition>,
    pub medications: Vec<FhirMedicationRequest>,
    pub observations: Vec<FhirObservation>,
    pub allergies: Vec<FhirAllergyIntolerance>,
    pub immunizations: Vec<FhirImmunization>,
    pub procedures: Vec<FhirProcedure>,
    pub encounters: Vec<FhirEncounter>,
    pub diagnostic_reports: Vec<FhirDiagnosticReport>,
    pub documents: Vec<FhirDocumentReference>,
    pub care_plans: Vec<FhirCarePlan>,
}

impl RenderedPatientBundle {
    /// Returns all clinical resource families populated inside this patient bundle.
    #[must_use]
    pub fn populated_families(&self) -> Vec<FhirResourceFamily> {
        let mut out = vec![FhirResourceFamily::Patient];
        if !self.conditions.is_empty() {
            out.push(FhirResourceFamily::Condition);
        }
        if !self.medications.is_empty() {
            out.push(FhirResourceFamily::MedicationRequest);
        }
        if !self.observations.is_empty() {
            out.push(FhirResourceFamily::Observation);
        }
        if !self.allergies.is_empty() {
            out.push(FhirResourceFamily::AllergyIntolerance);
        }
        if !self.immunizations.is_empty() {
            out.push(FhirResourceFamily::Immunization);
        }
        if !self.procedures.is_empty() {
            out.push(FhirResourceFamily::Procedure);
        }
        if !self.encounters.is_empty() {
            out.push(FhirResourceFamily::Encounter);
        }
        if !self.diagnostic_reports.is_empty() {
            out.push(FhirResourceFamily::DiagnosticReport);
        }
        if !self.documents.is_empty() {
            out.push(FhirResourceFamily::DocumentReference);
        }
        if !self.care_plans.is_empty() {
            out.push(FhirResourceFamily::CarePlan);
        }
        out
    }

    /// Returns ONLY observations available on or before `as_of_date`, withholding post-`as_of` results.
    #[must_use]
    pub fn observations_visible_at(&self, as_of_date: &str) -> Vec<&FhirObservation> {
        self.observations
            .iter()
            .filter(|obs| {
                obs.availability == ResultAvailability::AvailableAtAsOf
                    && obs.effective_date.as_str() <= as_of_date
            })
            .collect()
    }
}

/// Runtime manifest (`manifest.json`) tracking per-file SHA-256 digests and overall cohort digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankManifest {
    pub schema_version: u32,
    pub seed: u64,
    pub as_of_date: String,
    pub total_patients: usize,
    pub light_count: usize,
    pub moderate_count: usize,
    pub complex_count: usize,
    pub flagship_count: usize,
    pub file_digests: BTreeMap<String, String>,
    pub bank_sha256: String,
}

/// Strictly runtime-only rendered bank (`patients/`, `calendar/`, `manifest.json`).
///
/// By type construction, contains ZERO authoring ledgers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderedRuntimeBank {
    pub patients: BTreeMap<String, RenderedPatientBundle>,
    pub calendar: Vec<FhirAppointment>,
    pub manifest: BankManifest,
}

/// Authoring-only provenance ledger strictly separated from [`RenderedRuntimeBank`].
///
/// Never loaded into runtime servers or container images.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoringLedger {
    pub seed: u64,
    pub generator_version: String,
    pub per_patient_recipes: BTreeMap<String, String>,
}
