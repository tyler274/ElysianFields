//! Multi-format EHR ingestion, lossless 16-bucket mapping with untouched native bytes + SHA-256,
//! mandatory patient-binding validation, 42 CFR Part 2 classification & notices, SMART on FHIR R4
//! + PKCE `S256`, KipuEMR, C-CDA R2.1 XML, HL7 v2.x, `shlink:/`, and Gmail/Spruce channel adapters.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::today::{ChannelHealthStatus, TodayChannelBanner, derive_channel_banner};

/// Verbatim 42 CFR § 2.32(a) statutory redisclosure prohibition notice.
pub const PART2_STATUTORY_REDISCLOSURE_NOTICE_2_32_A: &str = "This record which has been disclosed to you is protected by federal confidentiality rules (42 CFR part 2). These rules prohibit you from using or disclosing this record, or testimony that describes the information contained in this record, in any civil, criminal, administrative, or legislative proceedings by any federal, state, or local authority, against the patient, unless authorized by the consent of the patient, except as provided at 42 CFR 2.12(c)(5) and 2.65.";

/// Closed 16 clinical record buckets matching `contracts/m3-boundaries-and-states.schema.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

impl ClinicalRecordBucket {
    /// Complete closed set of all 16 clinical record buckets.
    pub const ALL: [Self; 16] = [
        Self::Patient,
        Self::Problems,
        Self::Medications,
        Self::Allergies,
        Self::Results,
        Self::Vitals,
        Self::Notes,
        Self::Documents,
        Self::Visits,
        Self::Procedures,
        Self::Immunizations,
        Self::Orders,
        Self::History,
        Self::Forms,
        Self::Messages,
        Self::Other,
    ];
}

/// Closed 42 CFR Part 2 confidentiality classification matching `contracts/m3-boundaries-and-states.schema.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConfidentialityTag {
    Standard,
    Part2Sud,
    Part2SudCounselingNotes,
}

impl ConfidentialityTag {
    /// Enforces export/redisclosure gating:
    /// - `Standard`: permitted in general clinical exports.
    /// - `Part2Sud`: requires explicit written 42 CFR Part 2 consent.
    /// - `Part2SudCounselingNotes`: requires BOTH Part 2 consent AND separate specific counseling-notes consent (42 CFR § 2.31(b)).
    #[must_use]
    pub const fn permitted_in_export(
        self,
        has_written_part2_consent: bool,
        has_separate_counseling_notes_consent: bool,
    ) -> bool {
        match self {
            Self::Standard => true,
            Self::Part2Sud => has_written_part2_consent,
            Self::Part2SudCounselingNotes => {
                has_written_part2_consent && has_separate_counseling_notes_consent
            }
        }
    }
}

/// Verbatim 42 CFR § 2.32(a)/(b) notice attached to every `Part2Sud` and `Part2SudCounselingNotes` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Part2DisclosureNotice {
    pub statutory_notice_2_32_a: String,
    pub consent_scope_2_32_b: String,
}

impl Part2DisclosureNotice {
    #[must_use]
    pub fn for_scope(consent_scope: impl Into<String>) -> Self {
        Self {
            statutory_notice_2_32_a: PART2_STATUTORY_REDISCLOSURE_NOTICE_2_32_A.to_owned(),
            consent_scope_2_32_b: consent_scope.into(),
        }
    }
}

/// Lossless clinical record preserving untouched native source bytes and SHA-256 digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredClinicalRecord {
    pub bucket: ClinicalRecordBucket,
    pub source_id: String,
    pub revision_id: String,
    pub acquired_at: String,
    pub clinical_date: String,
    pub patient_binding: String,
    pub native_bytes: Vec<u8>,
    pub sha256_hex: String,
    pub confidentiality: ConfidentialityTag,
    pub part2_notice: Option<Part2DisclosureNotice>,
    pub summary: String,
}

/// Honest partial-delivery error record for unreadable attachments or malformed sibling records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailedPart {
    pub source_id: String,
    pub reason: String,
    pub native_sha256_hex: String,
}

/// Result of ingesting a multi-record clinical batch with per-item failure isolation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IngestionBatchResult {
    pub admitted: Vec<StoredClinicalRecord>,
    pub failed_parts: Vec<FailedPart>,
}

/// Errors that reject a record or link prior to admission.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IngestionError {
    #[error("patient binding missing in source record '{source_id}'")]
    MissingPatientBinding { source_id: String },
    #[error("patient binding mismatch in '{source_id}': expected '{expected}', found '{actual}'")]
    PatientBindingMismatch {
        source_id: String,
        expected: String,
        actual: String,
    },
    #[error("malformed clinical payload: {reason}")]
    MalformedPayload { reason: String },
    #[error("SMART Health Link rejected: {reason}")]
    InvalidShlink { reason: String },
    #[error("SMART on FHIR configuration error: {reason}")]
    InvalidSmartConfig { reason: String },
}

/// Closed catalog of supported live SMART on FHIR R4 EHR vendors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SmartFhirVendor {
    EpicMyChart,
    OracleHealthCerner,
    Athenahealth,
    EClinicalWorksHealow,
    MeditechExpanse,
    NextGen,
    Veradigm,
    ElationHealth,
    CustomFhirR4,
}

/// EHRs lacking live FHIR APIs surface explicit export/assisted onboarding rather than fake live checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistedOnboardingVendor {
    SimplePractice,
}

impl AssistedOnboardingVendor {
    #[must_use]
    pub const fn onboarding_guidance(self) -> &'static str {
        match self {
            Self::SimplePractice => {
                "SimplePractice does not provide live SMART on FHIR R4 APIs; use guided chart export upload."
            }
        }
    }
}

/// SMART on FHIR scope version (`v1` `.read` vs `v2` CRUDS `.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmartScopeVersion {
    V1Read,
    V2Cruds,
}

/// Validated SMART on FHIR R4 authorization launch request using exact-base-URL `aud` and PKCE `S256`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmartAuthorizeRequest {
    pub vendor: SmartFhirVendor,
    pub authorize_endpoint: String,
    pub token_endpoint: String,
    pub aud_base_url: String,
    pub scope: String,
    pub code_challenge_s256: String,
    pub code_challenge_method: &'static str,
    pub uses_metadata_fallback: bool,
}

/// Builds a validated SMART on FHIR R4 PKCE `S256` authorize request with `.well-known/smart-configuration`
/// or `metadata` fallback and explicit US Core scopes.
pub fn build_smart_authorize_request(
    vendor: SmartFhirVendor,
    fhir_base_url: &str,
    well_known_endpoints: Option<(&str, &str)>,
    metadata_fallback_endpoints: Option<(&str, &str)>,
    scope_version: SmartScopeVersion,
    pkce_code_verifier: &str,
) -> Result<SmartAuthorizeRequest, IngestionError> {
    if !fhir_base_url.starts_with("https://") {
        return Err(IngestionError::InvalidSmartConfig {
            reason: "FHIR base URL must use https://".to_owned(),
        });
    }
    if pkce_code_verifier.len() < 43 {
        return Err(IngestionError::InvalidSmartConfig {
            reason: "PKCE code_verifier must be at least 43 characters".to_owned(),
        });
    }

    let (authorize_endpoint, token_endpoint, uses_metadata_fallback) = match well_known_endpoints {
        Some((auth, tok)) => (auth.to_owned(), tok.to_owned(), false),
        None => match metadata_fallback_endpoints {
            Some((auth, tok)) => (auth.to_owned(), tok.to_owned(), true),
            None => {
                return Err(IngestionError::InvalidSmartConfig {
                    reason: "neither .well-known/smart-configuration nor /metadata provided OAuth endpoints".to_owned(),
                });
            }
        },
    };

    let scope = match scope_version {
        SmartScopeVersion::V1Read => {
            "launch/patient openid fhirUser patient/Patient.read patient/Condition.read patient/MedicationRequest.read patient/AllergyIntolerance.read patient/Observation.read patient/DocumentReference.read"
        }
        SmartScopeVersion::V2Cruds => {
            "launch/patient openid fhirUser patient/Patient.rs patient/Condition.rs patient/MedicationRequest.rs patient/AllergyIntolerance.rs patient/Observation.rs patient/DocumentReference.rs"
        }
    };

    Ok(SmartAuthorizeRequest {
        vendor,
        authorize_endpoint,
        token_endpoint,
        aud_base_url: fhir_base_url.trim_end_matches('/').to_owned(),
        scope: scope.to_owned(),
        code_challenge_s256: pkce_s256_challenge(pkce_code_verifier),
        code_challenge_method: "S256",
        uses_metadata_fallback,
    })
}

/// Validates a pagination `next` URL strictly stays same-origin as `fhir_base_url`.
#[must_use]
pub fn is_same_origin_bundle_next(fhir_base_url: &str, next_url: &str) -> bool {
    fn https_origin(u: &str) -> Option<&str> {
        let rest = u.strip_prefix("https://")?;
        let end = rest.find('/').unwrap_or(rest.len());
        Some(&u[..("https://".len() + end)])
    }
    match (https_origin(fhir_base_url), https_origin(next_url)) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        _ => false,
    }
}

/// Maps a FHIR R4 `resourceType` (and optional `Observation.category`) into the closed 16 buckets.
#[must_use]
pub fn map_fhir_resource_bucket(
    resource_type: &str,
    observation_category: Option<&str>,
) -> ClinicalRecordBucket {
    match resource_type {
        "Patient" => ClinicalRecordBucket::Patient,
        "Condition" => ClinicalRecordBucket::Problems,
        "MedicationRequest" | "MedicationStatement" | "MedicationDispense" => {
            ClinicalRecordBucket::Medications
        }
        "AllergyIntolerance" => ClinicalRecordBucket::Allergies,
        "Observation" => match observation_category {
            Some("vital-signs") => ClinicalRecordBucket::Vitals,
            Some("social-history") => ClinicalRecordBucket::History,
            _ => ClinicalRecordBucket::Results,
        },
        "DiagnosticReport" => ClinicalRecordBucket::Results,
        "DocumentReference" => ClinicalRecordBucket::Documents,
        "Encounter" | "Appointment" => ClinicalRecordBucket::Visits,
        "Procedure" => ClinicalRecordBucket::Procedures,
        "Immunization" => ClinicalRecordBucket::Immunizations,
        "ServiceRequest" => ClinicalRecordBucket::Orders,
        "FamilyMemberHistory" => ClinicalRecordBucket::History,
        "QuestionnaireResponse" => ClinicalRecordBucket::Forms,
        "Communication" => ClinicalRecordBucket::Messages,
        _ => ClinicalRecordBucket::Other,
    }
}

/// Ingests a FHIR R4 JSON resource enforcing mandatory patient-binding validation (`Patient.id` or `subject.reference`)
/// and retaining untouched native bytes + SHA-256.
pub fn ingest_fhir_r4_resource(
    expected_patient_id: &str,
    native_json_bytes: &[u8],
    acquired_at: &str,
) -> Result<StoredClinicalRecord, IngestionError> {
    let value: serde_json::Value = serde_json::from_slice(native_json_bytes).map_err(|err| {
        IngestionError::MalformedPayload {
            reason: err.to_string(),
        }
    })?;

    let resource_type = value
        .get("resourceType")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| IngestionError::MalformedPayload {
            reason: "missing FHIR resourceType".to_owned(),
        })?;
    let source_id = value
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown-fhir-id")
        .to_owned();

    let binding = if resource_type == "Patient" {
        Some(source_id.clone())
    } else {
        value
            .get("subject")
            .or_else(|| value.get("patient"))
            .and_then(|s| s.get("reference"))
            .and_then(serde_json::Value::as_str)
            .map(|r| r.strip_prefix("Patient/").unwrap_or(r).to_owned())
    };

    let bound_patient = binding.ok_or_else(|| IngestionError::MissingPatientBinding {
        source_id: source_id.clone(),
    })?;
    if bound_patient != expected_patient_id {
        return Err(IngestionError::PatientBindingMismatch {
            source_id,
            expected: expected_patient_id.to_owned(),
            actual: bound_patient,
        });
    }

    let obs_cat = value
        .get("category")
        .and_then(serde_json::Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("coding"))
        .and_then(serde_json::Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("code"))
        .and_then(serde_json::Value::as_str);

    let bucket = map_fhir_resource_bucket(resource_type, obs_cat);
    let revision_id = value
        .get("meta")
        .and_then(|m| m.get("versionId"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("1")
        .to_owned();
    let clinical_date = value
        .get("effectiveDateTime")
        .or_else(|| value.get("recordedDate"))
        .or_else(|| value.get("date"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or(acquired_at)
        .to_owned();

    Ok(StoredClinicalRecord {
        bucket,
        source_id,
        revision_id,
        acquired_at: acquired_at.to_owned(),
        clinical_date,
        patient_binding: bound_patient,
        native_bytes: native_json_bytes.to_vec(),
        sha256_hex: sha256_hex(native_json_bytes),
        confidentiality: ConfidentialityTag::Standard,
        part2_notice: None,
        summary: format!("FHIR R4 {resource_type}"),
    })
}

/// KipuEMR authentication mode supporting API v3 HMAC and API v4 OAuth2 Bearer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KipuApiAuthMode {
    ApiV3HmacSha256,
    ApiV3HmacSha1,
    ApiV4OAuth2Bearer,
}

impl KipuApiAuthMode {
    #[must_use]
    pub const fn accept_header(self) -> &'static str {
        match self {
            Self::ApiV3HmacSha256 | Self::ApiV3HmacSha1 => {
                "application/vnd.kipusystems+json; version=3"
            }
            Self::ApiV4OAuth2Bearer => "application/json",
        }
    }
}

/// Ingests a KipuEMR behavioral health record with mandatory `casefile_id` binding and
/// 42 CFR Part 2 (`Standard` / `Part2Sud` / `Part2SudCounselingNotes`) classification + § 2.32 notice.
pub fn ingest_kipu_record(
    expected_casefile_id: &str,
    native_json_bytes: &[u8],
    acquired_at: &str,
    consent_scope_2_32_b: &str,
) -> Result<StoredClinicalRecord, IngestionError> {
    let value: serde_json::Value = serde_json::from_slice(native_json_bytes).map_err(|err| {
        IngestionError::MalformedPayload {
            reason: err.to_string(),
        }
    })?;
    let source_id = value
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("kipu-record")
        .to_owned();
    let casefile_id = value
        .get("casefile_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| IngestionError::MissingPatientBinding {
            source_id: source_id.clone(),
        })?;
    if casefile_id != expected_casefile_id {
        return Err(IngestionError::PatientBindingMismatch {
            source_id,
            expected: expected_casefile_id.to_owned(),
            actual: casefile_id.to_owned(),
        });
    }

    let kind = value
        .get("record_kind")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("note");
    let is_sud_program = value
        .get("part2_program")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let is_counseling_note = value
        .get("sud_counseling_note")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);

    let confidentiality = if is_counseling_note {
        ConfidentialityTag::Part2SudCounselingNotes
    } else if is_sud_program {
        ConfidentialityTag::Part2Sud
    } else {
        ConfidentialityTag::Standard
    };
    let part2_notice = match confidentiality {
        ConfidentialityTag::Standard => None,
        ConfidentialityTag::Part2Sud | ConfidentialityTag::Part2SudCounselingNotes => {
            Some(Part2DisclosureNotice::for_scope(consent_scope_2_32_b))
        }
    };

    let bucket = match kind {
        "evaluation" | "form" => ClinicalRecordBucket::Forms,
        "medication" => ClinicalRecordBucket::Medications,
        "vital" => ClinicalRecordBucket::Vitals,
        "problem" => ClinicalRecordBucket::Problems,
        _ => ClinicalRecordBucket::Notes,
    };
    let clinical_date = value
        .get("recorded_at")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(acquired_at)
        .to_owned();

    Ok(StoredClinicalRecord {
        bucket,
        source_id,
        revision_id: "kipu-v3".to_owned(),
        acquired_at: acquired_at.to_owned(),
        clinical_date,
        patient_binding: casefile_id.to_owned(),
        native_bytes: native_json_bytes.to_vec(),
        sha256_hex: sha256_hex(native_json_bytes),
        confidentiality,
        part2_notice,
        summary: format!("KipuEMR {kind}"),
    })
}

/// Ingests a C-CDA R2.1 XML document:
/// - Validates `<ClinicalDocument>` and `<recordTarget>`/`<patientRole>`/`<id extension="..."/>` binding
/// - Maps LOINC section codes (`2.16.840.1.11383.6.1`) into the 16 buckets with untouched XML slices + SHA-256
/// - Isolates `<nonXMLBody>` and `nullFlavor` sections into explicit [`FailedPart`] without dropping valid siblings
pub fn ingest_ccda_r21_xml(
    expected_patient_id: &str,
    xml_bytes: &[u8],
    acquired_at: &str,
) -> Result<IngestionBatchResult, IngestionError> {
    let xml = std::str::from_utf8(xml_bytes).map_err(|err| IngestionError::MalformedPayload {
        reason: err.to_string(),
    })?;
    if !xml.contains("<ClinicalDocument") {
        return Err(IngestionError::MalformedPayload {
            reason: "missing <ClinicalDocument root element".to_owned(),
        });
    }

    let patient_role =
        extract_between(xml, "<patientRole>", "</patientRole>").ok_or_else(|| {
            IngestionError::MissingPatientBinding {
                source_id: "ccda-document".to_owned(),
            }
        })?;
    let bound_id = extract_attr(patient_role, "extension").ok_or_else(|| {
        IngestionError::MissingPatientBinding {
            source_id: "ccda-document".to_owned(),
        }
    })?;
    if bound_id != expected_patient_id {
        return Err(IngestionError::PatientBindingMismatch {
            source_id: "ccda-document".to_owned(),
            expected: expected_patient_id.to_owned(),
            actual: bound_id.to_owned(),
        });
    }

    let mut result = IngestionBatchResult::default();

    // Isolate <nonXMLBody> into FailedPart while keeping valid structured sections.
    if let Some(non_xml) = extract_between(xml, "<nonXMLBody>", "</nonXMLBody>") {
        let slice = format!("<nonXMLBody>{non_xml}</nonXMLBody>");
        result.failed_parts.push(FailedPart {
            source_id: "ccda-nonXMLBody".to_owned(),
            reason: "binary nonXMLBody attachment isolated as unreadable".to_owned(),
            native_sha256_hex: sha256_hex(slice.as_bytes()),
        });
    }

    let mut cursor = xml;
    let mut section_idx = 0usize;
    while let Some(start) = cursor.find("<section") {
        let after_start = &cursor[start..];
        let Some(end_rel) = after_start.find("</section>") else {
            break;
        };
        let section_slice = &after_start[..end_rel + "</section>".len()];
        cursor = &after_start[end_rel + "</section>".len()..];
        section_idx += 1;
        let source_id = format!("ccda-section-{section_idx}");

        if section_slice.contains("nullFlavor=") {
            result.failed_parts.push(FailedPart {
                source_id,
                reason: "C-CDA section carries nullFlavor without readable clinical body"
                    .to_owned(),
                native_sha256_hex: sha256_hex(section_slice.as_bytes()),
            });
            continue;
        }

        let loinc_code = extract_attr(section_slice, "code").unwrap_or("");
        let bucket = map_ccda_loinc_bucket(loinc_code);
        result.admitted.push(StoredClinicalRecord {
            bucket,
            source_id,
            revision_id: "ccda-r2.1".to_owned(),
            acquired_at: acquired_at.to_owned(),
            clinical_date: acquired_at.to_owned(),
            patient_binding: bound_id.to_owned(),
            native_bytes: section_slice.as_bytes().to_vec(),
            sha256_hex: sha256_hex(section_slice.as_bytes()),
            confidentiality: ConfidentialityTag::Standard,
            part2_notice: None,
            summary: format!("C-CDA section LOINC {loinc_code}"),
        });
    }

    Ok(result)
}

fn map_ccda_loinc_bucket(loinc: &str) -> ClinicalRecordBucket {
    match loinc {
        "11450-4" => ClinicalRecordBucket::Problems,
        "10160-0" => ClinicalRecordBucket::Medications,
        "48765-2" => ClinicalRecordBucket::Allergies,
        "30954-2" => ClinicalRecordBucket::Results,
        "8716-3" => ClinicalRecordBucket::Vitals,
        "11506-3" => ClinicalRecordBucket::Notes,
        "46240-8" => ClinicalRecordBucket::Visits,
        "47519-4" => ClinicalRecordBucket::Procedures,
        "11369-6" => ClinicalRecordBucket::Immunizations,
        "29762-2" => ClinicalRecordBucket::History,
        _ => ClinicalRecordBucket::Other,
    }
}

/// Decodes HL7 v2.x escape sequences (`\F\`, `\S\`, `\T\`, `\R\`, `\E\`, `\.br\`) using dynamic separators.
#[must_use]
pub fn decode_hl7_escapes(
    raw: &str,
    field_sep: char,
    comp_sep: char,
    rep_sep: char,
    esc_char: char,
    subcomp_sep: char,
) -> String {
    let mut out = String::with_capacity(raw.len());
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == esc_char
            && let Some(end_rel) = chars[i + 1..].iter().position(|&c| c == esc_char)
        {
            let code: String = chars[i + 1..i + 1 + end_rel].iter().collect();
            match code.as_str() {
                "F" => out.push(field_sep),
                "S" => out.push(comp_sep),
                "T" => out.push(subcomp_sep),
                "R" => out.push(rep_sep),
                "E" => out.push(esc_char),
                ".br" => out.push('\n'),
                _ => {
                    out.push(esc_char);
                    out.push_str(&code);
                    out.push(esc_char);
                }
            }
            i += end_rel + 2;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Ingests an HL7 v2.x message (`ADT`, `ORU`, `MDM`, `SIU`):
/// - Parses dynamic `MSH-1` / `MSH-2` separators
/// - Validates `PID-3` patient binding before admission
/// - Maps `PID`/`PV1`/`SCH`/`DG1`/`AL1`/`RXE`/`OBR`/`OBX`/`TXA`/`NTE` into the 16 buckets
/// - Isolates `OBX` `ED` binary attachments into [`FailedPart`] without dropping valid siblings
pub fn ingest_hl7_v2_message(
    expected_patient_id: &str,
    raw_bytes: &[u8],
    acquired_at: &str,
) -> Result<IngestionBatchResult, IngestionError> {
    let text = std::str::from_utf8(raw_bytes).map_err(|err| IngestionError::MalformedPayload {
        reason: err.to_string(),
    })?;
    let segments: Vec<&str> = text
        .split(['\r', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let Some(msh) = segments.first().copied() else {
        return Err(IngestionError::MalformedPayload {
            reason: "empty HL7 v2 message".to_owned(),
        });
    };
    if !msh.starts_with("MSH") || msh.len() < 8 {
        return Err(IngestionError::MalformedPayload {
            reason: "HL7 v2 message must begin with MSH segment and encoding characters".to_owned(),
        });
    }
    let msh_chars: Vec<char> = msh.chars().collect();
    let field_sep = msh_chars[3];
    let comp_sep = msh_chars[4];
    let rep_sep = msh_chars[5];
    let esc_char = msh_chars[6];
    let subcomp_sep = msh_chars[7];

    // Locate PID-3 patient binding.
    let pid_id = segments.iter().find_map(|seg| {
        let fields: Vec<&str> = seg.split(field_sep).collect();
        if fields.first().copied() == Some("PID") {
            fields
                .get(3)
                .and_then(|f| f.split(comp_sep).next())
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
        } else {
            None
        }
    });

    let bound_patient = pid_id.ok_or_else(|| IngestionError::MissingPatientBinding {
        source_id: "hl7-pid-3".to_owned(),
    })?;
    if bound_patient != expected_patient_id {
        return Err(IngestionError::PatientBindingMismatch {
            source_id: "hl7-pid-3".to_owned(),
            expected: expected_patient_id.to_owned(),
            actual: bound_patient,
        });
    }

    let mut result = IngestionBatchResult::default();
    for (idx, seg) in segments.iter().enumerate() {
        let fields: Vec<&str> = seg.split(field_sep).collect();
        let Some(tag) = fields.first().copied() else {
            continue;
        };
        if tag == "MSH" {
            continue;
        }
        let source_id = format!("hl7-{tag}-{}", idx + 1);

        // Isolate OBX ED (encapsulated binary data) as FailedPart without dropping valid siblings.
        if tag == "OBX" && fields.get(2).copied() == Some("ED") {
            result.failed_parts.push(FailedPart {
                source_id,
                reason: "HL7 OBX ED encapsulated binary attachment isolated".to_owned(),
                native_sha256_hex: sha256_hex(seg.as_bytes()),
            });
            continue;
        }

        let bucket = match tag {
            "PID" => ClinicalRecordBucket::Patient,
            "PV1" | "SCH" => ClinicalRecordBucket::Visits,
            "DG1" => ClinicalRecordBucket::Problems,
            "AL1" => ClinicalRecordBucket::Allergies,
            "RXE" => ClinicalRecordBucket::Medications,
            "OBR" | "OBX" => ClinicalRecordBucket::Results,
            "TXA" => ClinicalRecordBucket::Documents,
            "NTE" => ClinicalRecordBucket::Notes,
            _ => ClinicalRecordBucket::Other,
        };

        let decoded_summary = decode_hl7_escapes(
            fields.get(5).copied().unwrap_or(tag),
            field_sep,
            comp_sep,
            rep_sep,
            esc_char,
            subcomp_sep,
        );

        result.admitted.push(StoredClinicalRecord {
            bucket,
            source_id,
            revision_id: "hl7-v2".to_owned(),
            acquired_at: acquired_at.to_owned(),
            clinical_date: acquired_at.to_owned(),
            patient_binding: bound_patient.clone(),
            native_bytes: seg.as_bytes().to_vec(),
            sha256_hex: sha256_hex(seg.as_bytes()),
            confidentiality: ConfidentialityTag::Standard,
            part2_notice: None,
            summary: decoded_summary,
        });
    }

    Ok(result)
}

/// Decoded SMART Health Link (`shlink:/`) manifest validated before network fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatedShlink {
    pub url: String,
    pub key_bytes_len: usize,
    pub flags: String,
    pub exp: Option<u64>,
}

/// Validates a `shlink:/` payload (HTTPS-only public host, exact 32-byte key, flags, expiration).
pub fn validate_shlink_payload(
    uri_prefix: &str,
    url: &str,
    decryption_key_bytes: &[u8],
    flags: &str,
    exp: Option<u64>,
    now_secs: u64,
) -> Result<ValidatedShlink, IngestionError> {
    if !uri_prefix.starts_with("shlink:/") {
        return Err(IngestionError::InvalidShlink {
            reason: "URI must begin with shlink:/".to_owned(),
        });
    }
    let Some(rest) = url.strip_prefix("https://") else {
        return Err(IngestionError::InvalidShlink {
            reason: "destination URL must use https://".to_owned(),
        });
    };
    let host = rest
        .split(['/', ':', '?', '#'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if host.is_empty()
        || host == "localhost"
        || host.starts_with("127.")
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || host == "[::1]"
    {
        return Err(IngestionError::InvalidShlink {
            reason: "destination host must be a public HTTPS endpoint".to_owned(),
        });
    }
    if decryption_key_bytes.len() != 32 {
        return Err(IngestionError::InvalidShlink {
            reason: format!(
                "decryption key must be exactly 32 bytes, got {}",
                decryption_key_bytes.len()
            ),
        });
    }
    if let Some(expires_at) = exp
        && now_secs >= expires_at
    {
        return Err(IngestionError::InvalidShlink {
            reason: "SMART Health Link has expired".to_owned(),
        });
    }

    Ok(ValidatedShlink {
        url: url.to_owned(),
        key_bytes_len: decryption_key_bytes.len(),
        flags: flags.to_owned(),
        exp,
    })
}

/// Clinic communication channel transport kind (Gmail API/SMTP or Spruce).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClinicChannelKind {
    GmailApi,
    GmailSmtpAuth,
    SpruceMessaging,
}

/// Product-neutral clinic communication channel adapter enforcing manual-copy-only outbound replies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClinicChannelAdapter {
    pub channel_id: String,
    pub kind: ClinicChannelKind,
    pub health: ChannelHealthStatus,
}

impl ClinicChannelAdapter {
    #[must_use]
    pub fn new(
        channel_id: impl Into<String>,
        kind: ClinicChannelKind,
        health: ChannelHealthStatus,
    ) -> Self {
        Self {
            channel_id: channel_id.into(),
            kind,
            health,
        }
    }

    /// Curie Clinic never auto-sends patient replies; outbound sending is strictly manual copy.
    #[must_use]
    pub const fn auto_send_permitted(&self) -> bool {
        false
    }

    /// Derives the Today banner for this channel (`TodaySaysSo`).
    #[must_use]
    pub fn today_banner(&self) -> TodayChannelBanner {
        derive_channel_banner(&self.channel_id, &self.health)
    }
}

/// Computes lowercase hex FIPS 180-4 SHA-256 digest using pure Rust (zero external dependencies).
#[must_use]
pub fn sha256_hex(data: &[u8]) -> String {
    let digest = sha256_bytes(data);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        let hi = HEX_DIGITS[usize::from(byte >> 4)];
        let lo = HEX_DIGITS[usize::from(byte & 0x0f)];
        hex.push(char::from(hi));
        hex.push(char::from(lo));
    }
    hex
}

/// Computes RFC 7636 PKCE `S256` code challenge (`BASE64URL-NO-PAD(SHA256(verifier))`).
#[must_use]
pub fn pkce_s256_challenge(code_verifier: &str) -> String {
    let digest = sha256_bytes(code_verifier.as_bytes());
    base64url_no_pad(&digest)
}

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

const SHA256_K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

fn sha256_bytes(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity(data.len() + 72);
    padded.extend_from_slice(data);
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for i in 0..16 {
            let b = &chunk[i * 4..i * 4 + 4];
            w[i] = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = [0u8; 32];
    for (idx, word) in h.iter().enumerate() {
        out[idx * 4..idx * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

fn base64url_no_pad(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = if chunk.len() > 1 {
            u32::from(chunk[1])
        } else {
            0
        };
        let b2 = if chunk.len() > 2 {
            u32::from(chunk[2])
        } else {
            0
        };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(char::from(TABLE[((triple >> 18) & 0x3f) as usize]));
        out.push(char::from(TABLE[((triple >> 12) & 0x3f) as usize]));
        if chunk.len() > 1 {
            out.push(char::from(TABLE[((triple >> 6) & 0x3f) as usize]));
        }
        if chunk.len() > 2 {
            out.push(char::from(TABLE[(triple & 0x3f) as usize]));
        }
    }
    out
}

fn extract_between<'a>(haystack: &'a str, start_tag: &str, end_tag: &str) -> Option<&'a str> {
    let start = haystack.find(start_tag)? + start_tag.len();
    let end = haystack[start..].find(end_tag)?;
    Some(&haystack[start..start + end])
}

fn extract_attr<'a>(xml_slice: &'a str, attr_name: &str) -> Option<&'a str> {
    let needle = format!("{attr_name}=\"");
    let start = xml_slice.find(&needle)? + needle.len();
    let end = xml_slice[start..].find('"')?;
    Some(&xml_slice[start..start + end])
}
