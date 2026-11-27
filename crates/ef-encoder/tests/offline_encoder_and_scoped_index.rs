use ef_encoder::{DEFAULT_EMBEDDING_DIM, EncoderError, OfflineSemanticEncoder, SemanticChartIndex};

#[test]
fn offline_encoder_produces_deterministic_l2_normalized_clinical_embeddings() {
    let encoder = OfflineSemanticEncoder::default();
    assert_eq!(encoder.dim(), DEFAULT_EMBEDDING_DIM);

    let renal_a = encoder.encode("Serum creatinine 1.8 mg/dL, eGFR 42 mL/min, CKD stage 3.");
    let renal_b = encoder.encode("Chronic kidney disease follow-up: creatinine and eGFR trend.");
    let cardio = encoder.encode("Resting 12-lead electrocardiogram shows normal sinus rhythm.");

    // Exact determinism across invocations.
    assert_eq!(
        renal_a,
        encoder.encode("Serum creatinine 1.8 mg/dL, eGFR 42 mL/min, CKD stage 3.")
    );
    assert!((renal_a.l2_norm() - 1.0).abs() < 1e-5);
    assert!((renal_b.l2_norm() - 1.0).abs() < 1e-5);
    assert_eq!(encoder.encode("   ").l2_norm(), 0.0);

    let sim_related = renal_a.cosine_similarity(&renal_b).expect("same dim");
    let sim_unrelated = renal_a.cosine_similarity(&cardio).expect("same dim");
    assert!(
        sim_related > sim_unrelated,
        "related renal notes ({sim_related}) must score higher than unrelated ECG ({sim_unrelated})"
    );
}

#[test]
fn scoped_batch_index_ranks_relevant_notes_with_zero_cross_patient_leakage() {
    let encoder = OfflineSemanticEncoder::default();
    let mut index = SemanticChartIndex::default();

    let count = index
        .insert_batch(
            &encoder,
            &[
                (
                    "syn-pt-0001",
                    "doc-renal",
                    "Metformin dosing review with serum creatinine 1.65 mg/dL and eGFR 44.",
                ),
                (
                    "syn-pt-0001",
                    "doc-vaccine",
                    "Seasonal quadrivalent influenza immunization administered left deltoid.",
                ),
                (
                    "syn-pt-0002",
                    "doc-other-patient-renal",
                    "Metformin dosing review with serum creatinine 1.65 mg/dL and eGFR 44.",
                ),
            ],
        )
        .expect("batch insert");
    assert_eq!(count, 3);
    assert_eq!(index.len(), 3);

    // Search strictly scoped to syn-pt-0001 NEVER returns syn-pt-0002's identical note!
    let hits = index
        .search(&encoder, "syn-pt-0001", "creatinine eGFR metformin", 5)
        .expect("search");
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].doc_id, "doc-renal");
    assert!(hits.iter().all(|h| h.scope_id == "syn-pt-0001"));

    // Serialization round-trip preserves exact scores.
    let json = serde_json::to_string(&index).expect("serialize");
    let restored: SemanticChartIndex = serde_json::from_str(&json).expect("deserialize");
    let restored_hits = restored
        .search(&encoder, "syn-pt-0001", "creatinine eGFR metformin", 5)
        .expect("restored search");
    assert_eq!(restored_hits[0].doc_id, "doc-renal");

    assert_eq!(
        OfflineSemanticEncoder::new(4).expect_err("too small"),
        EncoderError::InvalidDimension { dim: 4 }
    );
}
