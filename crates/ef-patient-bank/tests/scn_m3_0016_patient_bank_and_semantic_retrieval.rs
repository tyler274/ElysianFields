use ef_encoder::{OfflineSemanticEncoder, SemanticChartIndex};
use ef_patient_bank::{
    AttachmentPageExtraction, BinaryAttachmentKind, COHORT_COMPLEX_COUNT, COHORT_FLAGSHIP_COUNT,
    COHORT_LIGHT_COUNT, COHORT_MODERATE_COUNT, COHORT_TOTAL_PATIENTS, ComplexityTier,
    DEFAULT_AS_OF_DATE, DEFAULT_COHORT_SEED, ResultAvailability, RuntimeBankLoader,
    extract_pdf_pages, generate_synthetic_cohort, validate_rendered_bank,
};

#[test]
fn scn_m3_0016_byte_deterministic_468_patient_bank_validation_and_idempotent_import() {
    // 1. Build bank twice with identical seed and assert byte-identical manifests and digests.
    let (bank_a, ledger_a) = generate_synthetic_cohort(DEFAULT_COHORT_SEED, DEFAULT_AS_OF_DATE);
    let (bank_b, ledger_b) = generate_synthetic_cohort(DEFAULT_COHORT_SEED, DEFAULT_AS_OF_DATE);

    assert_eq!(bank_a.manifest.bank_sha256, bank_b.manifest.bank_sha256);
    assert_eq!(bank_a.manifest.file_digests, bank_b.manifest.file_digests);
    assert_eq!(ledger_a.per_patient_recipes.len(), COHORT_TOTAL_PATIENTS);
    assert_eq!(ledger_a, ledger_b);

    // 2. Validate every M3 §4.2 invariant across all 468 patients.
    let summary = validate_rendered_bank(&bank_a).expect("cohort validation must pass");
    assert_eq!(summary.total_patients, COHORT_TOTAL_PATIENTS);
    assert_eq!(summary.light_count, COHORT_LIGHT_COUNT);
    assert_eq!(summary.moderate_count, COHORT_MODERATE_COUNT);
    assert_eq!(summary.complex_count, COHORT_COMPLEX_COUNT);
    assert_eq!(summary.flagship_count, COHORT_FLAGSHIP_COUNT);
    assert_eq!(summary.calendar_appointments, COHORT_TOTAL_PATIENTS);
    assert!(summary.pending_post_as_of_observations > 0);
    assert!(summary.flagship_min_note_bytes >= 8_192);

    // Verify isolated corrupted PDF page preserves readable sibling pages.
    let flagship_id = "syn-pt-0183";
    let flagship_bundle = bank_a.patients.get(flagship_id).expect("flagship chart");
    assert!(flagship_bundle.patient.flagship.is_some());
    let corrupt_att = flagship_bundle.documents[0]
        .attachments
        .iter()
        .find(|a| a.kind == BinaryAttachmentKind::CorruptedSinglePagePdf)
        .expect("corrupted single-page PDF fixture");
    let pages = extract_pdf_pages(corrupt_att);
    assert_eq!(pages.len(), 3);
    assert!(matches!(
        pages[0],
        AttachmentPageExtraction::Readable { page_number: 1, .. }
    ));
    assert!(matches!(
        pages[1],
        AttachmentPageExtraction::Corrupted { page_number: 2, .. }
    ));
    assert!(matches!(
        pages[2],
        AttachmentPageExtraction::Readable { page_number: 3, .. }
    ));

    // Verify pending post-as_of observations are withheld at as_of_date.
    let visible_at_as_of = flagship_bundle.observations_visible_at(DEFAULT_AS_OF_DATE);
    assert!(
        visible_at_as_of
            .iter()
            .all(|o| o.availability == ResultAvailability::AvailableAtAsOf)
    );
    assert!(visible_at_as_of.len() < flagship_bundle.observations.len());

    // 3. Load bank twice idempotently into RuntimeBankLoader (accepts ONLY RenderedRuntimeBank, never AuthoringLedger).
    let mut loader = RuntimeBankLoader::new();
    loader.assign_panel(flagship_id, "phys-primary-1");

    let first = loader.import_rendered_bank(&bank_a);
    assert_eq!(first.inserted_records, COHORT_TOTAL_PATIENTS);
    assert_eq!(first.changed_records, 0);
    assert_eq!(first.unchanged_records, 0);
    assert_eq!(first.appointments_loaded, COHORT_TOTAL_PATIENTS);

    let second = loader.import_rendered_bank(&bank_a);
    assert_eq!(second.inserted_records, 0);
    assert_eq!(second.changed_records, 0);
    assert_eq!(second.unchanged_records, COHORT_TOTAL_PATIENTS);
    assert_eq!(loader.panel_assignment(flagship_id), Some("phys-primary-1"));

    // Indexed cohort, condition, and lab-range queries.
    assert_eq!(loader.patients_by_tier(ComplexityTier::Light).len(), 60);
    assert_eq!(loader.patients_by_tier(ComplexityTier::Moderate).len(), 122);
    assert_eq!(loader.patients_by_tier(ComplexityTier::Complex).len(), 286);
    assert_eq!(loader.patients_by_condition_code("N18.3").len(), 286);

    let elevated_cr = loader.patients_by_lab_range("2160-0", 1.35, 3.0, DEFAULT_AS_OF_DATE);
    assert_eq!(elevated_cr.len(), 286);

    // 4. Local offline semantic indexing of flagship progress note sections with zero external APIs.
    let encoder = OfflineSemanticEncoder::default();
    let mut index = SemanticChartIndex::default();
    for (idx, sec) in flagship_bundle.documents[0].sections.iter().enumerate() {
        index
            .insert_document(
                &encoder,
                flagship_id,
                &format!("sec-{idx}"),
                &format!("{} {}", sec.heading, sec.body),
            )
            .expect("index section");
    }
    let hits = index
        .search(&encoder, flagship_id, "serum creatinine eGFR HbA1c", 3)
        .expect("semantic search");
    assert_eq!(hits.len(), 3);
    assert!(hits[0].score > 0.0);
}
