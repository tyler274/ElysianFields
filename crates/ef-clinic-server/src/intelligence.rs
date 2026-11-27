//! Workspace-scoped offline semantic chart search (`ef-encoder`) and
//! de-identified clinical reference library query routes (`ef-clinical-lib`).
//!
//! Enforces strict 3-tier patient workspace access control (`Allowed` only;
//! `ClinicDemographic` -> `403`, `Denied` -> `404`), zero cross-patient leakage,
//! withdrawn-source exclusion, and fail-closed PHI rejection on clinical-library queries.

use std::collections::{BTreeMap, BTreeSet};

use ef_clinic_domain::workspace::CitedSourcePayload;
use ef_clinical_lib::ConceptQuery;
use ef_core::{SourceKey, WorkspaceId};
use ef_encoder::{OfflineSemanticEncoder, SemanticChartIndex, SemanticSearchHit};
use serde::{Deserialize, Serialize};

use crate::http::{HttpMethod, HttpRequest, HttpResponse};
use crate::service::{ClinicService, workspace_error_response};

const CLINICAL_LIBRARY_SCOPE: &str = "clinical-library-deidentified";
const DEFAULT_TOP_K: usize = 5;
const MAX_TOP_K: usize = 50;

#[derive(Debug, Clone)]
struct ClinicalLibraryEntry {
    article_id: &'static str,
    title: &'static str,
    summary: &'static str,
}

const DEIDENTIFIED_REFERENCE_ENTRIES: &[ClinicalLibraryEntry] = &[
    ClinicalLibraryEntry {
        article_id: "ref-ckd-egfr-staging",
        title: "Chronic Kidney Disease eGFR and Albuminuria Staging",
        summary: "Classify CKD stages G1 through G5 by eGFR and A1 through A3 by urine albumin-to-creatinine ratio; monitor serum creatinine and potassium after ACEi or ARB titration.",
    },
    ClinicalLibraryEntry {
        article_id: "ref-htn-resistant-management",
        title: "Resistant Hypertension Evaluation and Pharmacotherapy",
        summary: "Confirm ambulatory blood pressure readings, evaluate secondary causes, optimize thiazide-like diuretic and calcium channel blocker dosing, and consider mineralocorticoid receptor antagonist.",
    },
    ClinicalLibraryEntry {
        article_id: "ref-t2dm-glycemic-targets",
        title: "Type 2 Diabetes Glycemic Targets and Cardiorenal Therapy",
        summary: "Individualize HbA1c targets, prioritize SGLT2 inhibitors or GLP-1 receptor agonists for patients with atherosclerotic cardiovascular disease or chronic kidney disease.",
    },
    ClinicalLibraryEntry {
        article_id: "ref-hfref-gdmt-pillars",
        title: "Heart Failure With Reduced Ejection Fraction Guideline-Directed Medical Therapy",
        summary: "Initiate and uptitrate ARNI or ACEi, evidence-based beta-blocker, mineralocorticoid receptor antagonist, and SGLT2 inhibitor while monitoring volume status and renal function.",
    },
];

/// Server-side offline semantic search index and de-identified clinical library catalog.
pub(crate) struct ClinicIntelligenceState {
    encoder: OfflineSemanticEncoder,
    patient_index: SemanticChartIndex,
    withdrawn_sources: BTreeSet<(WorkspaceId, SourceKey)>,
    library_index: SemanticChartIndex,
    library_entries: BTreeMap<&'static str, &'static ClinicalLibraryEntry>,
}

impl Default for ClinicIntelligenceState {
    fn default() -> Self {
        let encoder = OfflineSemanticEncoder::default();
        let mut library_index = SemanticChartIndex::default();
        let mut library_entries = BTreeMap::new();
        for entry in DEIDENTIFIED_REFERENCE_ENTRIES {
            let corpus_text = format!("{} {}", entry.title, entry.summary);
            let _ = library_index.insert_document(
                &encoder,
                CLINICAL_LIBRARY_SCOPE,
                entry.article_id,
                &corpus_text,
            );
            library_entries.insert(entry.article_id, entry);
        }
        Self {
            encoder,
            patient_index: SemanticChartIndex::default(),
            withdrawn_sources: BTreeSet::new(),
            library_index,
            library_entries,
        }
    }
}

impl ClinicIntelligenceState {
    pub(crate) fn index_source(&mut self, payload: &CitedSourcePayload) {
        self.withdrawn_sources
            .remove(&(payload.workspace_id.clone(), payload.source_key.clone()));
        let mut corpus = format!(
            "{} {} {} {}",
            payload.source_label, payload.title, payload.quote, payload.body
        );
        for part in &payload.readable_parts {
            corpus.push(' ');
            corpus.push_str(&part.label);
            corpus.push(' ');
            corpus.push_str(&part.text);
        }
        let _ = self.patient_index.insert_document(
            &self.encoder,
            payload.workspace_id.as_str(),
            payload.source_key.as_str(),
            &corpus,
        );
    }

    pub(crate) fn mark_withdrawn(&mut self, workspace_id: &WorkspaceId, source_key: &SourceKey) {
        self.withdrawn_sources
            .insert((workspace_id.clone(), source_key.clone()));
    }

    pub(crate) fn search_workspace(
        &self,
        workspace_id: &WorkspaceId,
        query: &str,
        top_k: usize,
    ) -> Vec<SemanticSearchHit> {
        let fetch_k = top_k.saturating_add(self.withdrawn_sources.len());
        let raw_hits = self
            .patient_index
            .search(&self.encoder, workspace_id.as_str(), query, fetch_k)
            .unwrap_or_default();
        raw_hits
            .into_iter()
            .filter(|hit| {
                hit.scope_id == workspace_id.as_str()
                    && !self
                        .withdrawn_sources
                        .iter()
                        .any(|(ws, sk)| ws == workspace_id && sk.as_str() == hit.doc_id)
            })
            .take(top_k)
            .collect()
    }

    pub(crate) fn search_library(
        &self,
        concept: &ConceptQuery,
        top_k: usize,
    ) -> Vec<ClinicalLibraryMatch> {
        let hits = self
            .library_index
            .search(
                &self.encoder,
                CLINICAL_LIBRARY_SCOPE,
                concept.as_str(),
                top_k,
            )
            .unwrap_or_default();
        hits.into_iter()
            .filter_map(|hit| {
                let entry = self.library_entries.get(hit.doc_id.as_str())?;
                Some(ClinicalLibraryMatch {
                    article_id: entry.article_id.to_owned(),
                    title: entry.title.to_owned(),
                    summary: entry.summary.to_owned(),
                    score: hit.score,
                })
            })
            .collect()
    }
}

#[derive(Debug, Deserialize)]
struct SearchRequestBody {
    query: String,
    #[serde(default)]
    top_k: Option<usize>,
}

#[derive(Debug, Serialize)]
struct PatientSemanticSearchResponse {
    workspace_id: WorkspaceId,
    model_version: &'static str,
    embedding_dim: usize,
    hits: Vec<SemanticSearchHit>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ClinicalLibraryMatch {
    article_id: String,
    title: String,
    summary: String,
    score: f32,
}

#[derive(Debug, Serialize)]
struct ClinicalLibraryQueryResponse {
    concept_query: String,
    deidentified_verified: bool,
    model_version: &'static str,
    embedding_dim: usize,
    matches: Vec<ClinicalLibraryMatch>,
}

impl ClinicService {
    /// Routes `POST /elysianfields/api/patients/:id/search`.
    ///
    /// Enforces 3-tier access control (`Allowed` only) and workspace-scoped
    /// offline 256-dim semantic search via `ef-encoder` with zero cross-patient leakage.
    pub(crate) fn route_patient_semantic_search(
        &mut self,
        req: &HttpRequest,
        raw_workspace_id: &str,
    ) -> HttpResponse {
        if req.method != HttpMethod::Post {
            return HttpResponse::method_not_allowed();
        }
        let whoami = match self.authenticate_request(req) {
            Ok(w) => w,
            Err(resp) => return resp,
        };
        let workspace_id = match WorkspaceId::new(raw_workspace_id) {
            Ok(id) => id,
            Err(err) => return HttpResponse::bad_request(err.to_string()),
        };
        if let Err(err) = self.evidence.require_clinical_access(
            &whoami.practice_id,
            &whoami.physician_id,
            &workspace_id,
        ) {
            return workspace_error_response(err);
        }

        let body: SearchRequestBody = match serde_json::from_slice(&req.body) {
            Ok(parsed) => parsed,
            Err(err) => return HttpResponse::bad_request(err.to_string()),
        };
        let trimmed_query = body.query.trim();
        if trimmed_query.is_empty() {
            return HttpResponse::bad_request("search query must not be empty");
        }
        let top_k = body.top_k.unwrap_or(DEFAULT_TOP_K).clamp(1, MAX_TOP_K);
        let hits = self
            .intelligence
            .search_workspace(&workspace_id, trimmed_query, top_k);

        HttpResponse::json_clinical(
            200,
            &PatientSemanticSearchResponse {
                workspace_id,
                model_version: self.intelligence.encoder.model_version(),
                embedding_dim: self.intelligence.encoder.dim(),
                hits,
            },
        )
    }

    /// Routes `POST /elysianfields/api/clinical-library/query`.
    ///
    /// Validates strictly via `ef_clinical_lib::ConceptQuery::parse` so any
    /// patient-identifying / PHI query fails closed with `400 Bad Request`.
    pub(crate) fn route_clinical_library_query(&mut self, req: &HttpRequest) -> HttpResponse {
        if req.method != HttpMethod::Post {
            return HttpResponse::method_not_allowed();
        }
        if let Err(resp) = self.authenticate_request(req) {
            return resp;
        }
        let body: SearchRequestBody = match serde_json::from_slice(&req.body) {
            Ok(parsed) => parsed,
            Err(err) => return HttpResponse::bad_request(err.to_string()),
        };
        let concept = match ConceptQuery::parse(&body.query) {
            Ok(valid) => valid,
            Err(err) => {
                return HttpResponse::json_clinical(
                    400,
                    &serde_json::json!({
                        "error": "phi_or_invalid_concept_query",
                        "detail": err.to_string(),
                    }),
                );
            }
        };
        let top_k = body.top_k.unwrap_or(DEFAULT_TOP_K).clamp(1, MAX_TOP_K);
        let matches = self.intelligence.search_library(&concept, top_k);

        HttpResponse::json_clinical(
            200,
            &ClinicalLibraryQueryResponse {
                concept_query: concept.as_str().to_owned(),
                deidentified_verified: true,
                model_version: self.intelligence.encoder.model_version(),
                embedding_dim: self.intelligence.encoder.dim(),
                matches,
            },
        )
    }
}
