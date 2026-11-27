//! Deterministic synthetic 468-patient FHIR R4 cohort bank (`EF-HARNESS-0004` / `SCN-M3-0016`).
//!
//! 100% synthetic with zero PHI, zero rendering dependencies (`bevy` forbidden),
//! strict authoring/runtime isolation, and idempotent indexed cohort loading.

pub mod fhir;
pub mod generator;
pub mod loader;

pub use fhir::{
    AttachmentPageExtraction, AuthoringLedger, BankManifest, BinaryAttachment,
    BinaryAttachmentKind, ComplexityTier, DocumentSection, FhirAllergyIntolerance, FhirAppointment,
    FhirCarePlan, FhirCondition, FhirDiagnosticReport, FhirDocumentReference, FhirEncounter,
    FhirImmunization, FhirMedicationRequest, FhirObservation, FhirPatient, FhirProcedure,
    FhirResourceFamily, FlagshipUnit, RenderedPatientBundle, RenderedRuntimeBank,
    ResultAvailability, extract_pdf_pages,
};
pub use generator::{
    COHORT_COMPLEX_COUNT, COHORT_FLAGSHIP_COUNT, COHORT_LIGHT_COUNT, COHORT_MODERATE_COUNT,
    COHORT_TOTAL_PATIENTS, DEFAULT_AS_OF_DATE, DEFAULT_COHORT_SEED, generate_synthetic_cohort,
    sha256_hex,
};
pub use loader::{
    BankValidationError, BankValidationSummary, ImportReport, RuntimeBankLoader,
    validate_rendered_bank,
};
