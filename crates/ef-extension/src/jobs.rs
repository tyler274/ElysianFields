//! Declared named read-only EHR job validation (`EF-EXT-0001`).
//!
//! Leased jobs may carry ONLY the 6 allowlisted identifier fields:
//! `id`, `lease`, `source`, `read`, `patient_id`, and `item_id` (optional).
//! Any raw URL, scheme, path traversal, or undeclared key fails closed.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Closed set of declared, vendor-neutral read-only connector operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeclaredReadOperation {
    PatientSummary,
    DocumentIndex,
    DocumentItem,
    EncounterList,
    ResultList,
    MedicationList,
}

impl DeclaredReadOperation {
    /// Parses a declared vendor-neutral read operation identifier at the boundary.
    pub fn parse(raw: &str) -> Result<Self, JobValidationError> {
        match raw {
            "patient_summary" => Ok(Self::PatientSummary),
            "document_index" => Ok(Self::DocumentIndex),
            "document_item" => Ok(Self::DocumentItem),
            "encounter_list" => Ok(Self::EncounterList),
            "result_list" => Ok(Self::ResultList),
            "medication_list" => Ok(Self::MedicationList),
            other => Err(JobValidationError::UnknownReadOperation {
                operation: other.to_owned(),
            }),
        }
    }
}

/// Strictly validated leased read job with zero raw URLs or undeclared fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeasedReadJob {
    pub id: String,
    pub lease: String,
    pub source: String,
    pub read: DeclaredReadOperation,
    pub patient_id: String,
    pub item_id: Option<String>,
}

/// Errors produced when validating a leased job payload.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum JobValidationError {
    #[error("job payload must be a JSON object")]
    NotAnObject,
    #[error("undeclared or forbidden job field '{field}' rejected (raw URLs are forbidden)")]
    ForbiddenField { field: String },
    #[error("missing required job field '{field}'")]
    MissingField { field: &'static str },
    #[error("job field '{field}' must be a non-empty string")]
    InvalidFieldType { field: &'static str },
    #[error("job field '{field}' contains a forbidden URL, scheme, or traversal pattern")]
    RawUrlSmugglingDetected { field: &'static str },
    #[error("undeclared read operation '{operation}'")]
    UnknownReadOperation { operation: String },
}

const ALLOWED_JOB_KEYS: &[&str] = &["id", "lease", "source", "read", "patient_id", "item_id"];

impl LeasedReadJob {
    /// Parses and validates a leased read job from raw JSON, failing closed on any
    /// undeclared field or raw URL smuggling attempt.
    pub fn parse_json(raw: &Value) -> Result<Self, JobValidationError> {
        let Some(map) = raw.as_object() else {
            return Err(JobValidationError::NotAnObject);
        };

        for key in map.keys() {
            if !ALLOWED_JOB_KEYS.contains(&key.as_str()) {
                return Err(JobValidationError::ForbiddenField { field: key.clone() });
            }
        }

        let id = extract_safe_identifier(map, "id")?;
        let lease = extract_safe_identifier(map, "lease")?;
        let source = extract_safe_identifier(map, "source")?;
        let patient_id = extract_safe_identifier(map, "patient_id")?;

        let read_raw = extract_safe_identifier(map, "read")?;
        let read = DeclaredReadOperation::parse(&read_raw)?;

        let item_id = match map.get("item_id") {
            None | Some(Value::Null) => None,
            Some(Value::String(val)) => {
                validate_identifier_value("item_id", val)?;
                Some(val.clone())
            }
            Some(_) => {
                return Err(JobValidationError::InvalidFieldType { field: "item_id" });
            }
        };

        Ok(Self {
            id,
            lease,
            source,
            read,
            patient_id,
            item_id,
        })
    }
}

fn extract_safe_identifier(
    map: &serde_json::Map<String, Value>,
    field: &'static str,
) -> Result<String, JobValidationError> {
    let Some(value) = map.get(field) else {
        return Err(JobValidationError::MissingField { field });
    };
    let Some(text) = value.as_str() else {
        return Err(JobValidationError::InvalidFieldType { field });
    };
    validate_identifier_value(field, text)?;
    Ok(text.to_owned())
}

fn validate_identifier_value(field: &'static str, value: &str) -> Result<(), JobValidationError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed != value {
        return Err(JobValidationError::InvalidFieldType { field });
    }
    let lower = value.to_ascii_lowercase();
    if lower.contains("://")
        || lower.starts_with("//")
        || lower.starts_with("http:")
        || lower.starts_with("https:")
        || lower.starts_with("javascript:")
        || lower.starts_with("data:")
        || lower.contains("..")
        || lower.contains('\\')
        || lower.contains('?')
        || lower.contains('#')
        || value.chars().any(char::is_control)
    {
        return Err(JobValidationError::RawUrlSmugglingDetected { field });
    }
    Ok(())
}
