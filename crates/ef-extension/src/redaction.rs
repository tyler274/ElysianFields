//! Automated billing, medication-price, invoice, and payment redaction with
//! SHA-256 payload receipts and durable idempotent outbox delivery.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::jobs::LeasedReadJob;
use crate::sha256::sha256_hex;

/// Normalized field substrings stripped unconditionally before computing payload receipts.
const FORBIDDEN_BILLING_OR_PAYMENT_KEY_SUBSTRINGS: &[&str] = &[
    "billing",
    "invoice",
    "price",
    "cost",
    "copay",
    "co_pay",
    "coinsurance",
    "deductible",
    "charge",
    "amount_due",
    "balance_due",
    "unit_price",
    "medication_price",
    "rx_price",
    "claim_amount",
    "payer_amount",
    "payment",
    "credit_card",
    "card_number",
    "bank_account",
    "routing_number",
    "ssn",
    "social_security",
];

/// Summary of redactions applied prior to receipt hashing and delivery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactionReport {
    pub stripped_field_paths: Vec<String>,
    pub redacted_string_count: usize,
}

/// Redacted payload together with its canonical JSON bytes and SHA-256 receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactedPayload {
    pub payload: Value,
    pub canonical_json: String,
    pub sha256_receipt: String,
    pub report: RedactionReport,
}

/// Strips billing, medication price, invoice, and payment fields recursively and computes
/// a deterministic canonical-JSON SHA-256 receipt over the redacted output.
#[must_use]
pub fn redact_and_receipt(raw_payload: &Value) -> RedactedPayload {
    let mut stripped_field_paths = Vec::new();
    let mut redacted_string_count = 0usize;

    let sanitized = redact_value(
        raw_payload,
        "",
        &mut stripped_field_paths,
        &mut redacted_string_count,
    );
    stripped_field_paths.sort();

    let canonical_json = canonicalize_json(&sanitized);
    let sha256_receipt = sha256_hex(canonical_json.as_bytes());

    RedactedPayload {
        payload: sanitized,
        canonical_json,
        sha256_receipt,
        report: RedactionReport {
            stripped_field_paths,
            redacted_string_count,
        },
    }
}

/// Verifies that a delivered payload matches its declared SHA-256 receipt.
#[must_use]
pub fn verify_payload_receipt(payload: &Value, expected_sha256: &str) -> bool {
    let canonical = canonicalize_json(payload);
    sha256_hex(canonical.as_bytes()) == expected_sha256
}

fn is_forbidden_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .map(|c| {
            if c == '-' || c == ' ' {
                '_'
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect();
    FORBIDDEN_BILLING_OR_PAYMENT_KEY_SUBSTRINGS
        .iter()
        .any(|needle| normalized.contains(needle))
}

fn redact_value(
    value: &Value,
    prefix: &str,
    stripped_paths: &mut Vec<String>,
    redacted_strings: &mut usize,
) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::new();
            // Sort keys deterministically so canonical serialization is order-independent.
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for key in keys {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                if is_forbidden_key(key) {
                    stripped_paths.push(path);
                    continue;
                }
                if let Some(child) = map.get(key) {
                    let clean_child = redact_value(child, &path, stripped_paths, redacted_strings);
                    out.insert(key.clone(), clean_child);
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => {
            let clean_items = items
                .iter()
                .enumerate()
                .map(|(idx, item)| {
                    let path = format!("{prefix}[{idx}]");
                    redact_value(item, &path, stripped_paths, redacted_strings)
                })
                .collect();
            Value::Array(clean_items)
        }
        Value::String(text) => {
            let (clean, changed) = redact_inline_sensitive_patterns(text);
            if changed {
                *redacted_strings += 1;
            }
            Value::String(clean)
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
    }
}

fn redact_inline_sensitive_patterns(input: &str) -> (String, bool) {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut idx = 0usize;
    let mut changed = false;

    while idx < bytes.len() {
        // Detect US SSN pattern: DDD-DD-DDDD bounded by non-digits.
        if idx + 11 <= bytes.len()
            && is_digit(bytes[idx])
            && is_digit(bytes[idx + 1])
            && is_digit(bytes[idx + 2])
            && bytes[idx + 3] == b'-'
            && is_digit(bytes[idx + 4])
            && is_digit(bytes[idx + 5])
            && bytes[idx + 6] == b'-'
            && is_digit(bytes[idx + 7])
            && is_digit(bytes[idx + 8])
            && is_digit(bytes[idx + 9])
            && is_digit(bytes[idx + 10])
            && (idx == 0 || !is_digit(bytes[idx - 1]))
            && (idx + 11 == bytes.len() || !is_digit(bytes[idx + 11]))
        {
            out.push_str("[REDACTED-SSN]");
            idx += 11;
            changed = true;
            continue;
        }

        let ch = input[idx..].chars().next().unwrap_or('\0');
        out.push(ch);
        idx += ch.len_utf8();
    }

    (out, changed)
}

const fn is_digit(b: u8) -> bool {
    b.is_ascii_digit()
}

fn canonicalize_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<&String, &Value> = map.iter().collect();
            let mut out = String::from("{");
            for (idx, (k, v)) in sorted.iter().enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                let encoded_key = serde_json::to_string(k).unwrap_or_else(|_| "\"\"".to_owned());
                out.push_str(&encoded_key);
                out.push(':');
                out.push_str(&canonicalize_json(v));
            }
            out.push('}');
            out
        }
        Value::Array(items) => {
            let mut out = String::from("[");
            for (idx, item) in items.iter().enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                out.push_str(&canonicalize_json(item));
            }
            out.push(']');
            out
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            serde_json::to_string(value).unwrap_or_else(|_| "null".to_owned())
        }
    }
}

/// Delivery state of a redacted job entry in the durable extension outbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutboxStatus {
    Pending,
    Delivered,
}

/// One queued redacted job delivery with its verified SHA-256 receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxEntry {
    pub job: LeasedReadJob,
    pub redacted: RedactedPayload,
    pub status: OutboxStatus,
}

/// Errors produced by outbox operations.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OutboxError {
    #[error("outbox entry for job '{job_id}' lease '{lease}' not found")]
    NotFound { job_id: String, lease: String },
}

/// Idempotent durable outbox delivering post-redaction payloads with SHA-256 receipts.
#[derive(Debug, Default)]
pub struct ExtensionOutbox {
    entries: BTreeMap<(String, String), OutboxEntry>,
}

impl ExtensionOutbox {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Redacts `raw_payload`, computes its SHA-256 receipt, and enqueues idempotently by `(job.id, job.lease)`.
    pub fn enqueue_redacted(&mut self, job: LeasedReadJob, raw_payload: &Value) -> &OutboxEntry {
        let key = (job.id.clone(), job.lease.clone());
        self.entries.entry(key).or_insert_with(|| OutboxEntry {
            job,
            redacted: redact_and_receipt(raw_payload),
            status: OutboxStatus::Pending,
        })
    }

    /// Marks an entry delivered after bridge acknowledgement.
    pub fn mark_delivered(&mut self, job_id: &str, lease: &str) -> Result<(), OutboxError> {
        let key = (job_id.to_owned(), lease.to_owned());
        let Some(entry) = self.entries.get_mut(&key) else {
            return Err(OutboxError::NotFound {
                job_id: job_id.to_owned(),
                lease: lease.to_owned(),
            });
        };
        entry.status = OutboxStatus::Delivered;
        Ok(())
    }

    /// Lists pending entries awaiting delivery.
    #[must_use]
    pub fn pending(&self) -> Vec<&OutboxEntry> {
        self.entries
            .values()
            .filter(|e| e.status == OutboxStatus::Pending)
            .collect()
    }

    /// Looks up a queued or delivered entry by `(job_id, lease)`.
    #[must_use]
    pub fn get(&self, job_id: &str, lease: &str) -> Option<&OutboxEntry> {
        self.entries.get(&(job_id.to_owned(), lease.to_owned()))
    }
}
