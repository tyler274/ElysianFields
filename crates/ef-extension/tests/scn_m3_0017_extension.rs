//! Acceptance tests proving SCN-M3-0017 (`EF-EXT-0001`):
//! Chrome MV3 extension pairing, memory-only credentials, declared named reads,
//! billing/price/PII redaction, SHA-256 receipts, and Decision D2 popup size budget.

use std::fs;
use std::path::PathBuf;

use ef_extension::{
    ChromeStorageArea, DeclaredReadOperation, ExtensionAuditViolation, ExtensionOutbox,
    JobValidationError, LeasedReadJob, MV3_POPUP_MAX_BYTES, MV3_POPUP_STRICT_UNCOMPRESSED_BYTES,
    MemoryOnlySessionStore, OutboxStatus, POPUP_REQUIRED_LOCALES, POPUP_RTL_LOCALES, PairingState,
    SessionCredentials, SessionError, audit_extension_package, audit_popup_parity,
    verify_payload_receipt,
};
use serde_json::json;

fn extension_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|crates| crates.parent())
        .expect("crate must live under <workspace>/crates/<name>")
        .join("extension")
}

#[test]
fn scn_m3_0017_mv3_package_passes_manifest_csp_neutrality_and_d2_size_audit() {
    let report = audit_extension_package(&extension_dir())
        .expect("extension/ package must satisfy MV3 security, neutrality, and D2 size rules");
    assert_eq!(report.manifest_version, 3);
    assert!(
        report.popup_total_bytes <= MV3_POPUP_MAX_BYTES,
        "popup size {} B must stay <= 1.0 MiB",
        report.popup_total_bytes
    );
    // Decision D2 semantic HTML popup stays orders of magnitude below 1.0 MiB (< 20 KiB uncompressed,
    // including cross-platform CRLF line endings on Windows).
    assert!(
        report.popup_total_bytes < MV3_POPUP_STRICT_UNCOMPRESSED_BYTES,
        "narrow semantic HTML popup should stay compact (< 20 KiB uncompressed), got {} B",
        report.popup_total_bytes
    );
}

#[test]
fn scn_m3_0017_credentials_live_strictly_in_memory_only_session_storage() {
    let mut store = MemoryOnlySessionStore::new();
    let creds =
        SessionCredentials::new("pair-01", "bearer-secret-xyz", "refresh-secret-abc", 5_000)
            .expect("valid credentials");

    // Custom Debug never leaks bearer or refresh tokens.
    let debug_text = format!("{creds:?}");
    assert!(!debug_text.contains("bearer-secret-xyz"));
    assert!(!debug_text.contains("refresh-secret-abc"));
    assert!(debug_text.contains("[REDACTED]"));

    // Attempting to persist into chrome.storage.local or chrome.storage.sync fails closed.
    assert_eq!(
        store.store_credentials(ChromeStorageArea::LocalDisk, creds.clone()),
        Err(SessionError::NonSessionStorageForbidden {
            area: ChromeStorageArea::LocalDisk
        })
    );
    assert_eq!(
        store.store_credentials(ChromeStorageArea::SyncCloud, creds.clone()),
        Err(SessionError::NonSessionStorageForbidden {
            area: ChromeStorageArea::SyncCloud
        })
    );
    assert!(store.local_disk_keys().is_empty());
    assert!(store.sync_cloud_keys().is_empty());

    // Storing in chrome.storage.session succeeds and expires cleanly on suspension sweep.
    store
        .store_credentials(ChromeStorageArea::SessionMemoryOnly, creds)
        .expect("session storage must succeed");
    assert_eq!(
        store.pairing_state(1_000),
        PairingState::Paired {
            pairing_id: "pair-01".to_owned(),
            expires_at_ms: 5_000
        }
    );
    assert!(store.active_credentials(1_000).is_ok());

    assert!(store.sweep_expired(5_000));
    assert_eq!(store.pairing_state(5_000), PairingState::Unpaired);
    assert_eq!(
        store.active_credentials(5_000),
        Err(SessionError::NotPaired)
    );
}

#[test]
fn scn_m3_0017_declared_named_read_jobs_accept_valid_and_reject_raw_url_smuggling() {
    let valid_json = json!({
        "id": "job-101",
        "lease": "lease-9a8b",
        "source": "connector-primary",
        "read": "medication_list",
        "patient_id": "pat-42",
        "item_id": "item-7"
    });
    let job = LeasedReadJob::parse_json(&valid_json).expect("valid allowlisted job must parse");
    assert_eq!(job.read, DeclaredReadOperation::MedicationList);
    assert_eq!(job.item_id.as_deref(), Some("item-7"));

    // Reject raw URL field smuggling.
    let smuggled_field = json!({
        "id": "job-102",
        "lease": "lease-2",
        "source": "connector-primary",
        "read": "patient_summary",
        "patient_id": "pat-42",
        "url": "https://internal.example/admin"
    });
    assert_eq!(
        LeasedReadJob::parse_json(&smuggled_field),
        Err(JobValidationError::ForbiddenField {
            field: "url".to_owned()
        })
    );

    // Reject URL smuggled inside an allowlisted field.
    let smuggled_value = json!({
        "id": "job-103",
        "lease": "lease-3",
        "source": "connector-primary",
        "read": "document_item",
        "patient_id": "pat-42",
        "item_id": "https://attacker.example/exfil"
    });
    assert_eq!(
        LeasedReadJob::parse_json(&smuggled_value),
        Err(JobValidationError::RawUrlSmugglingDetected { field: "item_id" })
    );

    // Reject undeclared read operation.
    let unknown_op = json!({
        "id": "job-104",
        "lease": "lease-4",
        "source": "connector-primary",
        "read": "raw_http_get",
        "patient_id": "pat-42"
    });
    assert_eq!(
        LeasedReadJob::parse_json(&unknown_op),
        Err(JobValidationError::UnknownReadOperation {
            operation: "raw_http_get".to_owned()
        })
    );
}

#[test]
fn scn_m3_0017_billing_and_price_redaction_produces_verified_sha256_receipt_in_outbox() {
    let job = LeasedReadJob::parse_json(&json!({
        "id": "job-201",
        "lease": "lease-aa11",
        "source": "connector-primary",
        "read": "medication_list",
        "patient_id": "pat-88"
    }))
    .expect("valid job");

    let synthetic_ehr_response = json!({
        "patient_id": "pat-88",
        "medications": [
            {
                "name": "Metformin 500 mg",
                "dose": "1 tablet twice daily",
                "medication_price": "$42.50",
                "copay": "$10.00",
                "billing_code": "J9999"
            }
        ],
        "invoice_number": "INV-2026-0091",
        "total_charge": 215.00,
        "notes": "Patient SSN 123-45-6789 confirmed on intake form."
    });

    let mut outbox = ExtensionOutbox::new();
    let entry = outbox.enqueue_redacted(job.clone(), &synthetic_ehr_response);
    let receipt = entry.redacted.sha256_receipt.clone();
    assert_eq!(receipt.len(), 64);
    assert_eq!(entry.status, OutboxStatus::Pending);

    // All billing, price, copay, invoice, and charge fields stripped before hashing.
    let serialized = &entry.redacted.canonical_json;
    assert!(!serialized.contains("medication_price"));
    assert!(!serialized.contains("copay"));
    assert!(!serialized.contains("billing_code"));
    assert!(!serialized.contains("invoice_number"));
    assert!(!serialized.contains("total_charge"));
    assert!(!serialized.contains("123-45-6789"));
    assert!(serialized.contains("[REDACTED-SSN]"));
    assert!(serialized.contains("Metformin 500 mg"));
    assert_eq!(entry.redacted.report.stripped_field_paths.len(), 5);
    assert_eq!(entry.redacted.report.redacted_string_count, 1);

    // Receipt matches delivered redacted payload.
    assert!(verify_payload_receipt(&entry.redacted.payload, &receipt));

    // Re-enqueuing same (job.id, lease) is idempotent and marks delivered cleanly.
    let retry_entry = outbox.enqueue_redacted(job, &synthetic_ehr_response);
    assert_eq!(retry_entry.redacted.sha256_receipt, receipt);
    assert_eq!(outbox.pending().len(), 1);

    outbox
        .mark_delivered("job-201", "lease-aa11")
        .expect("mark delivered");
    assert!(outbox.pending().is_empty());
}

#[test]
fn mv3_popup_passes_12_locale_rtl_theme_triage_and_strict_20kib_parity_audit() {
    let report = audit_popup_parity(&extension_dir())
        .expect("extension/ popup must pass 12-locale RTL, Light/Dark, triage, and < 20 KiB audit");
    assert_eq!(report.base.manifest_version, 3);
    assert_eq!(report.locale_count, POPUP_REQUIRED_LOCALES.len());
    assert_eq!(report.rtl_locale_count, POPUP_RTL_LOCALES.len());
    assert!(
        report.base.popup_total_bytes < MV3_POPUP_STRICT_UNCOMPRESSED_BYTES,
        "popup size {} B must stay strictly < 20 KiB ({} B)",
        report.base.popup_total_bytes,
        MV3_POPUP_STRICT_UNCOMPRESSED_BYTES
    );
}

#[test]
fn mv3_popup_parity_audit_rejects_missing_locale_inline_handler_and_phi_persistence() {
    let tmp = std::env::temp_dir().join(format!("ef-ext-popup-neg-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp extension dir");

    for file in [
        "manifest.json",
        "worker.js",
        "connector.js",
        "popup.html",
        "popup.css",
        "popup.js",
    ] {
        let src = extension_dir().join(file);
        fs::copy(&src, tmp.join(file)).expect("copy extension fixture");
    }

    // Corrupt popup.js: remove Hebrew locale & inject forbidden localStorage persistence.
    let mut js = fs::read_to_string(tmp.join("popup.js")).expect("read popup.js");
    js = js.replace("he:", "xx:");
    js.push_str("\n// bad\nlocalStorage.setItem('raw', 'leak');\n");
    fs::write(tmp.join("popup.js"), js).expect("write bad popup.js");

    let errs = audit_popup_parity(&tmp).expect_err("corrupted popup must fail parity audit");
    assert!(errs.iter().any(|e| matches!(
        e,
        ExtensionAuditViolation::MissingPopupLocale { locale: "he", .. }
    )));
    assert!(
        errs.iter()
            .any(|e| matches!(e, ExtensionAuditViolation::TriagePrivacyViolation { .. }))
    );

    // Corrupt popup.html with inline event handler -> caught by base audit.
    let mut html = fs::read_to_string(tmp.join("popup.html")).expect("read popup.html");
    html = html.replace("<button ", "<button onclick=\"alert(1)\" ");
    fs::write(tmp.join("popup.html"), html).expect("write bad popup.html");

    let base_errs = audit_extension_package(&tmp).expect_err("inline handler must fail audit");
    assert!(base_errs.iter().any(|e| matches!(
        e,
        ExtensionAuditViolation::InlineScriptDetected { file } if file == "popup.html"
    )));

    let _ = fs::remove_dir_all(&tmp);
}
