//! Verbatim versioned legal and transparency archive store (`EF-PUBLIC-0001`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::html::{RobotsPolicy, escape_html, render_semantic_page};

/// One verbatim legal section preserved without rewriting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegalSection {
    pub heading: String,
    pub paragraphs: Vec<String>,
}

/// Structured verbatim legal archive document stored as JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegalDocument {
    pub slug: String,
    pub route: String,
    pub title: String,
    pub effective_date: String,
    pub summary: String,
    pub sections: Vec<LegalSection>,
}

/// Errors loading or looking up legal archive documents.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LegalArchiveError {
    #[error("invalid legal archive JSON: {0}")]
    InvalidJson(String),
    #[error("legal document route '{route}' not found")]
    NotFound { route: String },
}

/// Registry of verbatim legal & transparency documents, including dated historical archives.
#[derive(Debug, Clone)]
pub struct LegalArchiveRegistry {
    by_route: BTreeMap<String, LegalDocument>,
}

impl LegalArchiveRegistry {
    /// Parses a JSON array of [`LegalDocument`] records verbatim.
    pub fn from_json_str(raw_json: &str) -> Result<Self, LegalArchiveError> {
        let docs: Vec<LegalDocument> = serde_json::from_str(raw_json)
            .map_err(|e| LegalArchiveError::InvalidJson(e.to_string()))?;
        let mut by_route = BTreeMap::new();
        for doc in docs {
            by_route.insert(doc.route.clone(), doc);
        }
        Ok(Self { by_route })
    }

    /// Returns the built-in canonical legal & transparency archive set covering all required routes.
    #[must_use]
    pub fn canonical() -> Self {
        let docs = vec![
            LegalDocument {
                slug: "privacy-current".to_owned(),
                route: "/privacy".to_owned(),
                title: "Elysian Fields Privacy Policy".to_owned(),
                effective_date: "2026-03-01".to_owned(),
                summary: "How Elysian Fields protects clinical and visitor privacy across Curie Clinic.".to_owned(),
                sections: vec![
                    LegalSection {
                        heading: "Scope and Product Separation".to_owned(),
                        paragraphs: vec![
                            "Curie Clinic and Curie Mobile operate with strictly separated accounts, storage, and clinical records.".to_owned(),
                            "Operational telemetry uses a strict content-free allowlist and never records clinical prose or patient identifiers.".to_owned(),
                        ],
                    },
                    LegalSection {
                        heading: "42 CFR Part 2 Protections".to_owned(),
                        paragraphs: vec![
                            "Substance use disorder records protected under 42 CFR Part 2 require explicit written consent prior to disclosure and carry statutory prohibition-of-redisclosure notices.".to_owned(),
                        ],
                    },
                ],
            },
            LegalDocument {
                slug: "terms-current".to_owned(),
                route: "/terms".to_owned(),
                title: "Elysian Fields Terms of Service".to_owned(),
                effective_date: "2026-03-01".to_owned(),
                summary: "Terms governing clinical workspace access and public information services.".to_owned(),
                sections: vec![
                    LegalSection {
                        heading: "Clinical Decision Authority".to_owned(),
                        paragraphs: vec![
                            "Licensed clinicians retain sole clinical judgment and outbound communication authority. Automated systems never send messages to patients without explicit clinician action.".to_owned(),
                        ],
                    },
                ],
            },
            LegalDocument {
                slug: "clinical-transparency-current".to_owned(),
                route: "/clinical-transparency".to_owned(),
                title: "Clinical AI Transparency Notice".to_owned(),
                effective_date: "2026-03-01".to_owned(),
                summary: "Provenance, citation verifiability, and bounded execution guarantees.".to_owned(),
                sections: vec![
                    LegalSection {
                        heading: "Verifiable Citations and Bounded Execution".to_owned(),
                        paragraphs: vec![
                            "Every clinical summary links directly to source chart artifacts and dates. Incomplete sources are disclosed explicitly rather than presented as a quiet state.".to_owned(),
                        ],
                    },
                ],
            },
            LegalDocument {
                slug: "mobile-privacy-2026-03-01".to_owned(),
                route: "/mobile/privacy".to_owned(),
                title: "Curie Mobile Privacy Policy".to_owned(),
                effective_date: "2026-03-01".to_owned(),
                summary: "Patient-owned standalone privacy commitments for Curie Mobile.".to_owned(),
                sections: vec![
                    LegalSection {
                        heading: "Standalone Patient Ownership".to_owned(),
                        paragraphs: vec![
                            "Curie Mobile accounts belong solely to the patient and remain independent of any practice EHR.".to_owned(),
                            "Account deletion records a metadata-only privacy receipt in an independent create-only journal so database restores never resurrect deleted accounts.".to_owned(),
                        ],
                    },
                ],
            },
            LegalDocument {
                slug: "2025-11-01".to_owned(),
                route: "/mobile/privacy/2025-11-01".to_owned(),
                title: "Curie Mobile Privacy Policy (Archive 2025-11-01)".to_owned(),
                effective_date: "2025-11-01".to_owned(),
                summary: "Historical verbatim archive of the 2025-11-01 Curie Mobile Privacy Policy.".to_owned(),
                sections: vec![
                    LegalSection {
                        heading: "Archived Standalone Account Guarantee".to_owned(),
                        paragraphs: vec![
                            "Archive 2025-11-01 verbatim text: Curie Mobile stores patient-imported records strictly under the patient's own standalone account.".to_owned(),
                        ],
                    },
                ],
            },
            LegalDocument {
                slug: "2026-03-01".to_owned(),
                route: "/mobile/privacy/2026-03-01".to_owned(),
                title: "Curie Mobile Privacy Policy (Archive 2026-03-01)".to_owned(),
                effective_date: "2026-03-01".to_owned(),
                summary: "Historical verbatim archive of the 2026-03-01 Curie Mobile Privacy Policy.".to_owned(),
                sections: vec![
                    LegalSection {
                        heading: "Standalone Patient Ownership".to_owned(),
                        paragraphs: vec![
                            "Curie Mobile accounts belong solely to the patient and remain independent of any practice EHR.".to_owned(),
                        ],
                    },
                ],
            },
            LegalDocument {
                slug: "mobile-terms-2026-03-01".to_owned(),
                route: "/mobile/terms".to_owned(),
                title: "Curie Mobile Terms of Service".to_owned(),
                effective_date: "2026-03-01".to_owned(),
                summary: "Terms governing standalone Curie Mobile accounts and curated sharing.".to_owned(),
                sections: vec![
                    LegalSection {
                        heading: "Curated Sharing Disclosure".to_owned(),
                        paragraphs: vec![
                            "Nothing is shared with a care team until you explicitly confirm. Confirmed sends are irreversible and are not monitored for medical emergencies.".to_owned(),
                        ],
                    },
                ],
            },
            LegalDocument {
                slug: "2025-11-01".to_owned(),
                route: "/mobile/terms/2025-11-01".to_owned(),
                title: "Curie Mobile Terms of Service (Archive 2025-11-01)".to_owned(),
                effective_date: "2025-11-01".to_owned(),
                summary: "Historical verbatim archive of the 2025-11-01 Curie Mobile Terms of Service.".to_owned(),
                sections: vec![
                    LegalSection {
                        heading: "Archived Curated Sharing Terms".to_owned(),
                        paragraphs: vec![
                            "Archive 2025-11-01 verbatim text: Patient sharing requires explicit confirmation for each curated package.".to_owned(),
                        ],
                    },
                ],
            },
            LegalDocument {
                slug: "mobile-consumer-health-data".to_owned(),
                route: "/mobile/consumer-health-data".to_owned(),
                title: "Consumer Health Data Privacy Policy".to_owned(),
                effective_date: "2026-03-01".to_owned(),
                summary: "State consumer health data disclosures for Curie Mobile.".to_owned(),
                sections: vec![
                    LegalSection {
                        heading: "Zero Sale of Consumer Health Data".to_owned(),
                        paragraphs: vec![
                            "Elysian Fields never sells consumer health data, never uses health data for advertising, and honors withdrawal and deletion requests durably across backups.".to_owned(),
                        ],
                    },
                ],
            },
        ];

        let mut by_route = BTreeMap::new();
        for doc in docs {
            by_route.insert(doc.route.clone(), doc);
        }
        Self { by_route }
    }

    /// Exports all registered documents as canonical JSON for verbatim archive verification.
    pub fn to_json_pretty(&self) -> Result<String, LegalArchiveError> {
        let docs: Vec<&LegalDocument> = self.by_route.values().collect();
        serde_json::to_string_pretty(&docs)
            .map_err(|e| LegalArchiveError::InvalidJson(e.to_string()))
    }

    /// Looks up a legal document by exact route path.
    #[must_use]
    pub fn get(&self, route: &str) -> Option<&LegalDocument> {
        self.by_route.get(route)
    }

    /// Returns every registered legal archive route path.
    #[must_use]
    pub fn routes(&self) -> Vec<&str> {
        self.by_route.keys().map(String::as_str).collect()
    }

    /// Renders a legal document verbatim as semantic, indexable HTML (zero canvas, zero JS required).
    pub fn render_html(&self, route: &str) -> Result<String, LegalArchiveError> {
        let Some(doc) = self.get(route) else {
            return Err(LegalArchiveError::NotFound {
                route: route.to_owned(),
            });
        };

        let mut body = String::new();
        body.push_str("    <article class=\"legal-document\">\n");
        body.push_str(&format!("      <h1>{}</h1>\n", escape_html(&doc.title)));
        body.push_str(&format!(
            "      <p class=\"effective-date\">Effective date: <time datetime=\"{0}\">{0}</time></p>\n",
            escape_html(&doc.effective_date)
        ));
        body.push_str(&format!("      <p>{}</p>\n", escape_html(&doc.summary)));

        for section in &doc.sections {
            body.push_str("      <section>\n");
            body.push_str(&format!(
                "        <h2>{}</h2>\n",
                escape_html(&section.heading)
            ));
            for paragraph in &section.paragraphs {
                body.push_str(&format!("        <p>{}</p>\n", escape_html(paragraph)));
            }
            body.push_str("      </section>\n");
        }
        body.push_str("    </article>");

        Ok(render_semantic_page(
            &doc.title,
            &doc.summary,
            &doc.route,
            RobotsPolicy::IndexFollow,
            &body,
        ))
    }
}
