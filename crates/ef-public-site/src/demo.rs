//! Isolated per-visitor synthetic physician demo (`EF-PUBLIC-0001`).
//!
//! Runs strictly in memory from synthetic fixture charts with verbatim citation matching,
//! hard per-visitor turn budgets, and zero access to production clinical databases or secrets.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// One synthetic chart passage addressable by verbatim citation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyntheticChartPassage {
    pub citation_id: String,
    pub source_label: String,
    pub recorded_date: String,
    pub text: String,
}

/// Synthetic chart fixture seeded into each isolated visitor session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyntheticDemoChart {
    pub chart_id: String,
    pub display_name: String,
    pub passages: Vec<SyntheticChartPassage>,
}

/// Verified verbatim citation returned in a synthetic demo response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerbatimCitation {
    pub citation_id: String,
    pub source_label: String,
    pub quote: String,
}

/// One completed visitor question-and-answer turn inside the synthetic demo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DemoTurnRecord {
    pub turn_number: u32,
    pub prompt: String,
    pub answer: String,
    pub citations: Vec<VerbatimCitation>,
}

/// Outcome of submitting a prompt to the synthetic demo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DemoTurnOutcome {
    Answered {
        record: DemoTurnRecord,
        turns_remaining: u32,
    },
    BudgetExhausted {
        max_turns: u32,
        notice: String,
    },
}

/// Errors produced by synthetic demo validation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DemoError {
    #[error("visitor_id must not be empty")]
    EmptyVisitorId,
    #[error("demo prompt must not be empty")]
    EmptyPrompt,
}

/// Per-visitor isolated in-memory state.
#[derive(Debug, Clone)]
pub struct VisitorDemoSession {
    pub visitor_id: String,
    pub chart: SyntheticDemoChart,
    pub turns: Vec<DemoTurnRecord>,
}

/// Ephemeral in-memory synthetic physician demo engine with zero clinical persistence.
#[derive(Debug, Clone)]
pub struct SyntheticDemoEngine {
    max_turns_per_visitor: u32,
    fixture_chart: SyntheticDemoChart,
    sessions: BTreeMap<String, VisitorDemoSession>,
}

impl SyntheticDemoEngine {
    /// Creates an isolated synthetic demo engine with the given per-visitor turn budget.
    #[must_use]
    pub fn new(max_turns_per_visitor: u32) -> Self {
        let fixture_chart = SyntheticDemoChart {
            chart_id: "syn-demo-001".to_owned(),
            display_name: "Jordan Avery (Synthetic Demo Patient)".to_owned(),
            passages: vec![
                SyntheticChartPassage {
                    citation_id: "1".to_owned(),
                    source_label: "Synthetic Lab Panel".to_owned(),
                    recorded_date: "2026-02-14".to_owned(),
                    text: "Hemoglobin A1c measured at 6.8% on 2026-02-14, down from 7.4% six months prior.".to_owned(),
                },
                SyntheticChartPassage {
                    citation_id: "2".to_owned(),
                    source_label: "Synthetic Medication List".to_owned(),
                    recorded_date: "2026-02-14".to_owned(),
                    text: "Active synthetic regimen: metformin 500 mg twice daily and lisinopril 10 mg once daily.".to_owned(),
                },
                SyntheticChartPassage {
                    citation_id: "3".to_owned(),
                    source_label: "Synthetic Primary Care Visit Note".to_owned(),
                    recorded_date: "2026-02-14".to_owned(),
                    text: "Home blood pressure readings averaged 126/78 mmHg over the past two weeks with no dizziness.".to_owned(),
                },
            ],
        };

        Self {
            max_turns_per_visitor,
            fixture_chart,
            sessions: BTreeMap::new(),
        }
    }

    /// Submits a question for `visitor_id`, isolating state per visitor and verifying
    /// every citation appears verbatim in that visitor's synthetic fixture chart.
    pub fn submit_turn(
        &mut self,
        visitor_id: &str,
        prompt: &str,
    ) -> Result<DemoTurnOutcome, DemoError> {
        let visitor_trimmed = visitor_id.trim();
        if visitor_trimmed.is_empty() {
            return Err(DemoError::EmptyVisitorId);
        }
        let prompt_trimmed = prompt.trim();
        if prompt_trimmed.is_empty() {
            return Err(DemoError::EmptyPrompt);
        }

        let session = self
            .sessions
            .entry(visitor_trimmed.to_owned())
            .or_insert_with(|| VisitorDemoSession {
                visitor_id: visitor_trimmed.to_owned(),
                chart: self.fixture_chart.clone(),
                turns: Vec::new(),
            });

        let used_turns = session.turns.len() as u32;
        if used_turns >= self.max_turns_per_visitor {
            return Ok(DemoTurnOutcome::BudgetExhausted {
                max_turns: self.max_turns_per_visitor,
                notice: format!(
                    "Demo turn budget ({}) reached for this visitor session. No real clinical data or backend was accessed.",
                    self.max_turns_per_visitor
                ),
            });
        }

        let matched = select_matching_passage(&session.chart, prompt_trimmed);
        let citation = VerbatimCitation {
            citation_id: matched.citation_id.clone(),
            source_label: matched.source_label.clone(),
            quote: matched.text.clone(),
        };
        let answer = format!(
            "Synthetic chart finding [{}]: {}",
            citation.citation_id, citation.quote
        );

        let turn_number = used_turns + 1;
        let record = DemoTurnRecord {
            turn_number,
            prompt: prompt_trimmed.to_owned(),
            answer,
            citations: vec![citation],
        };
        session.turns.push(record.clone());

        Ok(DemoTurnOutcome::Answered {
            record,
            turns_remaining: self.max_turns_per_visitor - turn_number,
        })
    }

    /// Verifies that every citation returned in a turn appears verbatim in the synthetic chart.
    #[must_use]
    pub fn verify_citations_verbatim(&self, visitor_id: &str, record: &DemoTurnRecord) -> bool {
        let Some(session) = self.sessions.get(visitor_id) else {
            return false;
        };
        record.citations.iter().all(|cit| {
            session
                .chart
                .passages
                .iter()
                .any(|p| p.citation_id == cit.citation_id && p.text.contains(&cit.quote))
        })
    }

    /// Returns a read-only view of one visitor's isolated session.
    #[must_use]
    pub fn session(&self, visitor_id: &str) -> Option<&VisitorDemoSession> {
        self.sessions.get(visitor_id)
    }

    /// Returns total active ephemeral visitor sessions.
    #[must_use]
    pub fn active_session_count(&self) -> usize {
        self.sessions.len()
    }
}

fn select_matching_passage<'a>(
    chart: &'a SyntheticDemoChart,
    prompt: &str,
) -> &'a SyntheticChartPassage {
    let lower = prompt.to_ascii_lowercase();
    if lower.contains("med") || lower.contains("metformin") || lower.contains("lisinopril") {
        return &chart.passages[1];
    }
    if lower.contains("bp") || lower.contains("blood pressure") || lower.contains("dizziness") {
        return &chart.passages[2];
    }
    &chart.passages[0]
}
