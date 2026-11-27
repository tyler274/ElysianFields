//! Byte-deterministic 468-patient synthetic FHIR R4 cohort generator.

use std::collections::BTreeMap;

use crate::fhir::{
    AuthoringLedger, BankManifest, BinaryAttachment, BinaryAttachmentKind, ComplexityTier,
    DocumentSection, FhirAllergyIntolerance, FhirAppointment, FhirCarePlan, FhirCondition,
    FhirDiagnosticReport, FhirDocumentReference, FhirEncounter, FhirImmunization,
    FhirMedicationRequest, FhirObservation, FhirPatient, FhirProcedure, FlagshipUnit,
    RenderedPatientBundle, RenderedRuntimeBank, ResultAvailability,
};

/// Exact cohort composition mandated by M3 §4.2.
pub const COHORT_LIGHT_COUNT: usize = 60;
pub const COHORT_MODERATE_COUNT: usize = 122;
pub const COHORT_COMPLEX_COUNT: usize = 286;
pub const COHORT_TOTAL_PATIENTS: usize =
    COHORT_LIGHT_COUNT + COHORT_MODERATE_COUNT + COHORT_COMPLEX_COUNT;
pub const COHORT_FLAGSHIP_COUNT: usize = 5;

/// Default reproducible generator seed.
pub const DEFAULT_COHORT_SEED: u64 = 0xE175_1A4F_0468_0001;

/// Default `as_of` clinical reference date (`YYYY-MM-DD`).
pub const DEFAULT_AS_OF_DATE: &str = "2026-06-15";

const FLAGSHIP_UNITS: [FlagshipUnit; 5] = [
    FlagshipUnit::Unit1,
    FlagshipUnit::Unit2,
    FlagshipUnit::Unit3,
    FlagshipUnit::Unit4,
    FlagshipUnit::Unit5,
];

/// Reproducible SplitMix64 PRNG for byte-identical cross-platform synthetic generation.
#[derive(Debug, Clone)]
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn range_usize(&mut self, min: usize, max_inclusive: usize) -> usize {
        if max_inclusive <= min {
            return min;
        }
        let span = (max_inclusive - min) + 1;
        min + ((self.next_u64() as usize) % span)
    }
}

/// Generates the complete byte-deterministic 468-patient synthetic FHIR R4 cohort bank,
/// strictly separating the runtime bundle from the authoring ledger.
#[must_use]
pub fn generate_synthetic_cohort(
    seed: u64,
    as_of_date: &str,
) -> (RenderedRuntimeBank, AuthoringLedger) {
    let mut rng = SplitMix64::new(seed);
    let mut patients = BTreeMap::new();
    let mut calendar = Vec::with_capacity(COHORT_TOTAL_PATIENTS);
    let mut per_patient_recipes = BTreeMap::new();

    for idx in 0..COHORT_TOTAL_PATIENTS {
        let tier = if idx < COHORT_LIGHT_COUNT {
            ComplexityTier::Light
        } else if idx < COHORT_LIGHT_COUNT + COHORT_MODERATE_COUNT {
            ComplexityTier::Moderate
        } else {
            ComplexityTier::Complex
        };

        // First 5 complex charts serve as the 5 flagship units.
        let flagship_idx = idx.checked_sub(COHORT_LIGHT_COUNT + COHORT_MODERATE_COUNT);
        let flagship = flagship_idx.and_then(|fi| FLAGSHIP_UNITS.get(fi).copied());

        let patient_num = idx + 1;
        let patient_id = format!("syn-pt-{patient_num:04}");
        let bundle = build_patient_bundle(
            &mut rng,
            patient_num,
            &patient_id,
            tier,
            flagship,
            as_of_date,
        );

        let appt_day = 16 + (idx % 14);
        let visit_type = match tier {
            ComplexityTier::Light => "annual_preventive_review",
            ComplexityTier::Moderate => "chronic_care_follow_up",
            ComplexityTier::Complex => "multimorbidity_reconciliation",
        };
        calendar.push(FhirAppointment {
            id: format!("appt-{patient_num:04}"),
            patient_id: patient_id.clone(),
            scheduled_date: format!("2026-06-{appt_day:02}"),
            visit_type: visit_type.to_owned(),
            status: "booked".to_owned(),
        });

        per_patient_recipes.insert(
            patient_id.clone(),
            format!("recipe-v1:tier={tier:?}:flagship={flagship:?}:seq={patient_num}"),
        );
        patients.insert(patient_id, bundle);
    }

    let mut file_digests = BTreeMap::new();
    for (patient_id, bundle) in &patients {
        let serialized = serde_json::to_vec(bundle).expect("bundle serializable");
        file_digests.insert(
            format!("patients/{patient_id}.json"),
            sha256_hex(&serialized),
        );
    }
    let cal_bytes = serde_json::to_vec(&calendar).expect("calendar serializable");
    file_digests.insert(
        "calendar/appointments.json".to_owned(),
        sha256_hex(&cal_bytes),
    );

    let mut manifest_preimage = Vec::new();
    for (path, digest) in &file_digests {
        manifest_preimage.extend_from_slice(path.as_bytes());
        manifest_preimage.push(b'=');
        manifest_preimage.extend_from_slice(digest.as_bytes());
        manifest_preimage.push(b'\n');
    }
    let bank_sha256 = sha256_hex(&manifest_preimage);

    let manifest = BankManifest {
        schema_version: 1,
        seed,
        as_of_date: as_of_date.to_owned(),
        total_patients: patients.len(),
        light_count: COHORT_LIGHT_COUNT,
        moderate_count: COHORT_MODERATE_COUNT,
        complex_count: COHORT_COMPLEX_COUNT,
        flagship_count: COHORT_FLAGSHIP_COUNT,
        file_digests,
        bank_sha256,
    };

    let runtime_bank = RenderedRuntimeBank {
        patients,
        calendar,
        manifest,
    };
    let ledger = AuthoringLedger {
        seed,
        generator_version: "ef-patient-bank-v1".to_owned(),
        per_patient_recipes,
    };
    (runtime_bank, ledger)
}

fn build_patient_bundle(
    rng: &mut SplitMix64,
    num: usize,
    patient_id: &str,
    tier: ComplexityTier,
    flagship: Option<FlagshipUnit>,
    as_of_date: &str,
) -> RenderedPatientBundle {
    let birth_year = match tier {
        ComplexityTier::Light => rng.range_usize(1975, 1998),
        ComplexityTier::Moderate => rng.range_usize(1958, 1982),
        ComplexityTier::Complex => rng.range_usize(1942, 1968),
    };
    let birth_month = rng.range_usize(1, 12);
    let birth_day = rng.range_usize(1, 28);
    let sex_at_birth = if num.is_multiple_of(2) {
        "female"
    } else {
        "male"
    };

    let patient = FhirPatient {
        id: patient_id.to_owned(),
        synthetic_display_name: format!("Synthetic Cohort Chart {num:04}"),
        birth_date: format!("{birth_year:04}-{birth_month:02}-{birth_day:02}"),
        sex_at_birth: sex_at_birth.to_owned(),
        tier,
        flagship,
    };

    // Conditions scaled by complexity tier.
    let mut conditions = vec![FhirCondition {
        id: format!("cond-{num:04}-1"),
        icd10_code: "I10".to_owned(),
        display: "Essential primary hypertension".to_owned(),
        clinical_status: "active".to_owned(),
        onset_date: format!("{}-03-10", birth_year + 35),
    }];
    let has_diabetes = !matches!(tier, ComplexityTier::Light) && !num.is_multiple_of(3);
    if has_diabetes {
        conditions.push(FhirCondition {
            id: format!("cond-{num:04}-2"),
            icd10_code: "E11.9".to_owned(),
            display: "Type 2 diabetes mellitus without complications".to_owned(),
            clinical_status: "active".to_owned(),
            onset_date: format!("{}-07-14", birth_year + 40),
        });
    }
    let has_ckd = matches!(tier, ComplexityTier::Complex);
    if has_ckd {
        conditions.push(FhirCondition {
            id: format!("cond-{num:04}-3"),
            icd10_code: "N18.3".to_owned(),
            display: "Chronic kidney disease stage 3".to_owned(),
            clinical_status: "active".to_owned(),
            onset_date: format!("{}-11-02", birth_year + 45),
        });
    }

    // Physiologically consistent vitals & labs.
    let systolic = rng.range_usize(112, 158) as f64;
    let pulse_pressure = rng.range_usize(36, 62) as f64;
    let diastolic = (systolic - pulse_pressure).max(60.0);

    let creatinine_hundredths = if has_ckd {
        rng.range_usize(135, 235)
    } else {
        rng.range_usize(70, 115)
    };
    let creatinine = (creatinine_hundredths as f64) / 100.0;
    // Simplified deterministic inverse relationship ensuring physiological consistency.
    let egfr = ((95.0 / creatinine) * if sex_at_birth == "female" { 0.92 } else { 1.0 })
        .clamp(22.0, 118.0);
    let egfr_rounded = (egfr * 10.0).round() / 10.0;

    let hba1c_tenths = if has_diabetes {
        rng.range_usize(66, 94)
    } else {
        rng.range_usize(51, 59)
    };
    let hba1c = (hba1c_tenths as f64) / 10.0;
    let potassium = (rng.range_usize(37, 51) as f64) / 10.0;
    let sodium = rng.range_usize(136, 144) as f64;

    let mut observations = vec![
        FhirObservation {
            id: format!("obs-{num:04}-sbp"),
            loinc_code: "8480-6".to_owned(),
            display: "Systolic blood pressure".to_owned(),
            value: systolic,
            unit: "mmHg".to_owned(),
            effective_date: "2026-06-01".to_owned(),
            availability: ResultAvailability::AvailableAtAsOf,
        },
        FhirObservation {
            id: format!("obs-{num:04}-dbp"),
            loinc_code: "8462-4".to_owned(),
            display: "Diastolic blood pressure".to_owned(),
            value: diastolic,
            unit: "mmHg".to_owned(),
            effective_date: "2026-06-01".to_owned(),
            availability: ResultAvailability::AvailableAtAsOf,
        },
        FhirObservation {
            id: format!("obs-{num:04}-cr"),
            loinc_code: "2160-0".to_owned(),
            display: "Creatinine [Mass/volume] in Serum or Plasma".to_owned(),
            value: creatinine,
            unit: "mg/dL".to_owned(),
            effective_date: "2026-06-05".to_owned(),
            availability: ResultAvailability::AvailableAtAsOf,
        },
        FhirObservation {
            id: format!("obs-{num:04}-egfr"),
            loinc_code: "62238-1".to_owned(),
            display: "Glomerular filtration rate".to_owned(),
            value: egfr_rounded,
            unit: "mL/min/1.73m2".to_owned(),
            effective_date: "2026-06-05".to_owned(),
            availability: ResultAvailability::AvailableAtAsOf,
        },
        FhirObservation {
            id: format!("obs-{num:04}-a1c"),
            loinc_code: "4548-4".to_owned(),
            display: "Hemoglobin A1c".to_owned(),
            value: hba1c,
            unit: "%".to_owned(),
            effective_date: "2026-06-05".to_owned(),
            availability: ResultAvailability::AvailableAtAsOf,
        },
        FhirObservation {
            id: format!("obs-{num:04}-k"),
            loinc_code: "2823-3".to_owned(),
            display: "Potassium".to_owned(),
            value: potassium,
            unit: "mEq/L".to_owned(),
            effective_date: "2026-06-05".to_owned(),
            availability: ResultAvailability::AvailableAtAsOf,
        },
        FhirObservation {
            id: format!("obs-{num:04}-na"),
            loinc_code: "2951-2".to_owned(),
            display: "Sodium".to_owned(),
            value: sodium,
            unit: "mEq/L".to_owned(),
            effective_date: "2026-06-05".to_owned(),
            availability: ResultAvailability::AvailableAtAsOf,
        },
    ];

    // Pending post-as_of lab observation withheld until after `as_of_date`.
    if !matches!(tier, ComplexityTier::Light) || flagship.is_some() {
        observations.push(FhirObservation {
            id: format!("obs-{num:04}-pending"),
            loinc_code: "2160-0".to_owned(),
            display: "Pending follow-up serum creatinine".to_owned(),
            value: (creatinine + 0.04).min(3.0),
            unit: "mg/dL".to_owned(),
            effective_date: "2026-06-22".to_owned(),
            availability: ResultAvailability::PendingPostAsOf,
        });
    }

    let mut medications = vec![FhirMedicationRequest {
        id: format!("med-{num:04}-1"),
        rxnorm_code: "314076".to_owned(),
        medication_name: "Lisinopril 10 MG Oral Tablet".to_owned(),
        dosage_instruction: format!("Chart {num:04}: take 1 tablet daily; monitor BP."),
        status: "active".to_owned(),
    }];
    if has_diabetes {
        medications.push(FhirMedicationRequest {
            id: format!("med-{num:04}-2"),
            rxnorm_code: "860975".to_owned(),
            medication_name: "Metformin hydrochloride 500 MG Extended Release".to_owned(),
            dosage_instruction: format!("Chart {num:04}: take 1 tablet with evening meal."),
            status: "active".to_owned(),
        });
    }

    let allergies = vec![FhirAllergyIntolerance {
        id: format!("alg-{num:04}-1"),
        substance: if num.is_multiple_of(3) {
            "Penicillin G"
        } else {
            "Sulfonamide antibacterial"
        }
        .to_owned(),
        reaction: "Mild cutaneous rash".to_owned(),
        clinical_status: "active".to_owned(),
    }];

    let immunizations = vec![FhirImmunization {
        id: format!("imm-{num:04}-1"),
        vaccine_name: "Influenza seasonal quadrivalent".to_owned(),
        occurrence_date: "2025-10-18".to_owned(),
    }];

    let procedures = vec![FhirProcedure {
        id: format!("proc-{num:04}-1"),
        cpt_code: "93000".to_owned(),
        display: "Resting 12-lead electrocardiogram".to_owned(),
        performed_date: "2026-05-11".to_owned(),
    }];

    let encounters = vec![FhirEncounter {
        id: format!("enc-{num:04}-1"),
        encounter_class: "ambulatory".to_owned(),
        period_start: "2026-06-01".to_owned(),
        reason: format!("Primary-care review for {patient_id} prior to {as_of_date}"),
    }];

    let diagnostic_reports = vec![FhirDiagnosticReport {
        id: format!("dr-{num:04}-1"),
        title: format!("Comprehensive Metabolic Panel ({patient_id})"),
        issued_date: "2026-06-05".to_owned(),
        conclusion: format!(
            "Chart {num:04}: Cr {creatinine:.2} mg/dL, eGFR {egfr_rounded:.1}, K {potassium:.1}, Na {sodium:.0}."
        ),
        availability: ResultAvailability::AvailableAtAsOf,
    }];

    let documents = build_documents(
        num,
        patient_id,
        flagship,
        systolic,
        diastolic,
        creatinine,
        egfr_rounded,
        hba1c,
    );

    let care_plans = vec![FhirCarePlan {
        id: format!("cp-{num:04}-1"),
        title: format!("Longitudinal Primary Care Plan ({patient_id})"),
        status: "active".to_owned(),
        goals: vec![format!(
            "Chart {num:04} target: keep BP under 130/80 and review labs at 12 weeks."
        )],
    }];

    RenderedPatientBundle {
        patient,
        conditions,
        medications,
        observations,
        allergies,
        immunizations,
        procedures,
        encounters,
        diagnostic_reports,
        documents,
        care_plans,
    }
}

#[allow(clippy::too_many_arguments)]
fn build_documents(
    num: usize,
    patient_id: &str,
    flagship: Option<FlagshipUnit>,
    systolic: f64,
    diastolic: f64,
    creatinine: f64,
    egfr: f64,
    hba1c: f64,
) -> Vec<FhirDocumentReference> {
    let mut sections = vec![
        DocumentSection {
            heading: "Interval History".to_owned(),
            body: format!(
                "Chart {num:04} ({patient_id}): longitudinal review recorded BP {systolic:.0}/{diastolic:.0} mmHg with steady home adherence."
            ),
        },
        DocumentSection {
            heading: "Laboratory Synthesis".to_owned(),
            body: format!(
                "Chart {num:04} ({patient_id}): serum creatinine {creatinine:.2} mg/dL, eGFR {egfr:.1} mL/min/1.73m2, HbA1c {hba1c:.1}%."
            ),
        },
    ];

    let mut attachments = Vec::new();

    if let Some(unit) = flagship {
        // Flagship charts include > 8 KiB multi-section progress notes with unique non-boilerplate
        // narrative lines + all 4 deterministic binary attachment types.
        for sec_idx in 1..=28usize {
            let mut body = String::with_capacity(340);
            for line_idx in 1..=4usize {
                body.push_str(&format!(
                    "[{patient_id}::{unit:?}::S{sec_idx:02}L{line_idx}] Synthetic clinical narrative entry {sec_idx}-{line_idx} tracking Cr {creatinine:.2}, eGFR {egfr:.1}, BP {systolic:.0}/{diastolic:.0}, and HbA1c {hba1c:.1}%.\n"
                ));
            }
            sections.push(DocumentSection {
                heading: format!("Flagship Section {sec_idx:02} ({unit:?})"),
                body,
            });
        }

        attachments.push(BinaryAttachment {
            attachment_id: format!("att-{num:04}-pdf-valid"),
            content_type: "application/pdf".to_owned(),
            kind: BinaryAttachmentKind::ValidMultiPagePdf,
            bytes: format!(
                "%PDF-1.4\n%PAGE:1:Valid outpatient summary page 1 for {patient_id} ({unit:?}).\n%PAGE:2:Valid medication reconciliation page 2 for {patient_id}.\n%%EOF\n"
            )
            .into_bytes(),
        });
        attachments.push(BinaryAttachment {
            attachment_id: format!("att-{num:04}-pdf-corrupt-page"),
            content_type: "application/pdf".to_owned(),
            kind: BinaryAttachmentKind::CorruptedSinglePagePdf,
            bytes: format!(
                "%PDF-1.4\n%PAGE:1:Readable consultation page 1 for {patient_id} ({unit:?}).\n%PAGE:2:CORRUPTED|zlib stream checksum mismatch on page 2\n%PAGE:3:Readable lab appendix page 3 for {patient_id}.\n%%EOF\n"
            )
            .into_bytes(),
        });
        attachments.push(BinaryAttachment {
            attachment_id: format!("att-{num:04}-png"),
            content_type: "image/png".to_owned(),
            kind: BinaryAttachmentKind::PngScan,
            bytes: vec![
                0x89,
                b'P',
                b'N',
                b'G',
                0x0D,
                0x0A,
                0x1A,
                0x0A,
                (num & 0xFF) as u8,
            ],
        });
        attachments.push(BinaryAttachment {
            attachment_id: format!("att-{num:04}-jpg-fax"),
            content_type: "image/jpeg".to_owned(),
            kind: BinaryAttachmentKind::JpegFaxScan,
            bytes: vec![0xFF, 0xD8, 0xFF, 0xE0, (num & 0xFF) as u8, 0xFF, 0xD9],
        });
    }

    vec![FhirDocumentReference {
        id: format!("doc-{num:04}-1"),
        title: format!("Outpatient Primary Care Progress Note ({patient_id})"),
        date: "2026-06-05".to_owned(),
        sections,
        attachments,
    }]
}

/// Pure-Rust SHA-256 hex digest for manifest integrity checks.
#[must_use]
pub fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
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
    let mut padded = Vec::with_capacity((data.len() + 9).div_ceil(64) * 64);
    padded.extend_from_slice(data);
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in padded.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, wb) in chunk.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*wb);
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
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
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
    let mut out = String::with_capacity(64);
    for word in h {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{word:08x}");
    }
    out
}
