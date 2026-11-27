//! Deterministic 100% synthetic clinical workload generator.
//!
//! Generates:
//! - An 80-turn clinical conversation (~40,000 characters) with headings, paragraphs,
//!   bulleted and numbered lists, clinical lab summary tables, fenced JSON/FHIRPath
//!   code blocks, links, and all 12 inline citation markers (`[1]`..=`[12]`).
//! - 11 multi-section synthetic progress note citations (`[1]`..=`[11]`).
//! - 1 structured 5,000-row synthetic clinical laboratory table citation (`[12]`).
//! - A deterministic 40 tokens/sec incremental streaming chunk generator.
//!
//! Zero protected health information and zero real clinical messages appear anywhere.

use crate::domain::{
    BlockId, CitationId, CitationSource, ClinicalLabRow, CodeLanguage, ConversationTurn,
    DeltaPayload, HeadingLevel, InlineSpan, LabFlag, ListKind, MarkdownBlock, ProgressNoteCitation,
    ProgressNoteSection, StreamChunk, StreamSeq, TranscriptBlock, TurnAuthor, TurnId,
};

/// Required number of turns in the M1 stutter workload specification.
pub const WORKLOAD_TURN_COUNT: usize = 80;

/// Required number of rows in the virtualized clinical lab table (`[12]`).
pub const LAB_TABLE_ROW_COUNT: usize = 5_000;

/// Number of distinct inline citation sources (`[1]`..=`[12]`).
pub const CITATION_SOURCE_COUNT: usize = 12;

/// Complete synthetic dataset loaded by the M1 streaming prototype.
#[derive(Debug, Clone)]
pub struct SyntheticClinicalWorkload {
    pub turns: Vec<ConversationTurn>,
    pub citation_sources: Vec<CitationSource>,
    pub lab_rows: Vec<ClinicalLabRow>,
    pub total_chars: usize,
    pub total_blocks: usize,
}

impl SyntheticClinicalWorkload {
    /// Generates the deterministic 80-turn clinical workload (~40,000 chars) and 5,000-row lab table.
    #[must_use]
    pub fn generate() -> Self {
        let mut turns = Vec::with_capacity(WORKLOAD_TURN_COUNT);
        let mut next_block_id: u32 = 0;
        let mut total_chars: usize = 0;

        for turn_index in 0..WORKLOAD_TURN_COUNT {
            let turn_id = TurnId(turn_index as u32);
            let pair_index = turn_index / 2;
            let (author, raw_blocks) = if turn_index.is_multiple_of(2) {
                (TurnAuthor::Clinician, build_clinician_prompt(pair_index))
            } else {
                (TurnAuthor::Assistant, build_assistant_response(pair_index))
            };

            let mut blocks = Vec::with_capacity(raw_blocks.len());
            for content in raw_blocks {
                total_chars = total_chars.saturating_add(content.char_count());
                blocks.push(TranscriptBlock {
                    id: BlockId(next_block_id),
                    turn_id,
                    author,
                    revision: 1,
                    content,
                });
                next_block_id = next_block_id.saturating_add(1);
            }

            turns.push(ConversationTurn {
                id: turn_id,
                author,
                blocks,
            });
        }

        let lab_rows = generate_lab_rows(LAB_TABLE_ROW_COUNT);
        let citation_sources = generate_citation_sources(lab_rows.len());

        Self {
            turns,
            citation_sources,
            lab_rows,
            total_chars,
            total_blocks: next_block_id as usize,
        }
    }
}

fn build_clinician_prompt(pair_index: usize) -> Vec<MarkdownBlock> {
    let scenario = CLINICAL_SCENARIOS[pair_index % CLINICAL_SCENARIOS.len()];
    let citation = citation_for_index(pair_index);
    let text = format!(
        "Synthetic case review #{:02} ({scenario}): summarize interval trends, reconcile medication changes, verify renal dosing thresholds, and cross-check longitudinal laboratory values from source ",
        pair_index + 1,
    );
    vec![MarkdownBlock::Paragraph {
        spans: vec![
            InlineSpan::Text(text),
            InlineSpan::Citation(citation),
            InlineSpan::Text(
                " along with the complete 5,000-row longitudinal chemistry ledger ".to_owned(),
            ),
            InlineSpan::Citation(CitationId::lab_table()),
            InlineSpan::Text(".".to_owned()),
        ],
    }]
}

fn build_assistant_response(pair_index: usize) -> Vec<MarkdownBlock> {
    let scenario = CLINICAL_SCENARIOS[pair_index % CLINICAL_SCENARIOS.len()];
    let primary_cite = citation_for_index(pair_index);
    let secondary_cite = citation_for_index(pair_index.saturating_add(5));
    let lab_cite = CitationId::lab_table();

    let mut blocks = vec![
        MarkdownBlock::Heading {
            level: HeadingLevel::H2,
            text: format!("Clinical Synthesis {:02}: {scenario}", pair_index + 1),
        },
        MarkdownBlock::Paragraph {
            spans: vec![
                InlineSpan::Text(format!(
                    "Synthetic longitudinal assessment #{:02} indicates stable hemodynamics with gradual metabolic improvement over the prior three encounters ",
                    pair_index + 1
                )),
                InlineSpan::Citation(primary_cite),
                InlineSpan::Text(
                    ". Renal clearance remains above dose-adjustment bounds while electrolyte variance stays within protocol thresholds ".to_owned(),
                ),
                InlineSpan::Citation(secondary_cite),
                InlineSpan::Text(" and structured chemistry series ".to_owned()),
                InlineSpan::Citation(lab_cite),
                InlineSpan::Text(".".to_owned()),
            ],
        },
        MarkdownBlock::List {
            kind: if pair_index.is_multiple_of(2) {
                ListKind::Bulleted
            } else {
                ListKind::Numbered
            },
            items: vec![
                format!(
                    "Confirm baseline metabolic stability for {scenario} before modifying standing maintenance therapy."
                ),
                "Review interval creatinine, potassium, and bicarbonate trajectories across the virtualized lab ledger.".to_owned(),
                "Document follow-up safety triggers and next ambulatory check-in interval.".to_owned(),
            ],
        },
    ];

    match pair_index % 3 {
        0 => {
            blocks.push(MarkdownBlock::Table {
                headers: vec![
                    "Analyte".to_owned(),
                    "Baseline".to_owned(),
                    "Current".to_owned(),
                    "Status".to_owned(),
                ],
                rows: vec![
                    vec![
                        "Sodium".to_owned(),
                        "138.0 mEq/L".to_owned(),
                        "139.2 mEq/L".to_owned(),
                        "Within range".to_owned(),
                    ],
                    vec![
                        "Potassium".to_owned(),
                        "4.3 mEq/L".to_owned(),
                        "4.1 mEq/L".to_owned(),
                        "Within range".to_owned(),
                    ],
                    vec![
                        "Creatinine".to_owned(),
                        "1.04 mg/dL".to_owned(),
                        "0.98 mg/dL".to_owned(),
                        "Stable".to_owned(),
                    ],
                ],
            });
        }
        1 => {
            blocks.push(MarkdownBlock::CodeBlock {
                language: CodeLanguage::Json,
                code: format!(
                    "{{\n  \"syntheticTurn\": {},\n  \"panel\": \"CMP-14\",\n  \"egfrThresholdMlMin\": 60,\n  \"alertState\": \"nominal\"\n}}",
                    pair_index + 1
                ),
            });
        }
        _ => {
            blocks.push(MarkdownBlock::Link {
                label: format!(
                    "Synthetic guideline reference {:02} ({scenario})",
                    pair_index + 1
                ),
                url: format!(
                    "https://synthetic.example.invalid/guidelines/case-{:02}",
                    pair_index + 1
                ),
            });
        }
    }

    blocks
}

fn citation_for_index(index: usize) -> CitationId {
    let raw = ((index % CITATION_SOURCE_COUNT) as u8) + 1;
    CitationId::new(raw).unwrap_or_else(|_| CitationId::progress_note())
}

fn generate_citation_sources(lab_row_count: usize) -> Vec<CitationSource> {
    let mut sources = Vec::with_capacity(CITATION_SOURCE_COUNT);
    for raw in 1..=11u8 {
        let id = CitationId::new(raw).unwrap_or_else(|_| CitationId::progress_note());
        let topic = CLINICAL_SCENARIOS[usize::from(raw - 1) % CLINICAL_SCENARIOS.len()];
        sources.push(CitationSource::ProgressNote(ProgressNoteCitation {
            id,
            title: format!("Synthetic Progress Note [{raw}]: {topic}"),
            encounter_label: format!("Synthetic Outpatient Encounter #{raw:02}"),
            sections: vec![
                ProgressNoteSection {
                    heading: "Interval History".to_owned(),
                    body: format!(
                        "Synthetic clinician narrative for note [{raw}]. Symptoms remained well controlled during ambulatory monitoring with full adherence to scheduled follow-up checks."
                    ),
                },
                ProgressNoteSection {
                    heading: "Objective Summary".to_owned(),
                    body: "Vital signs stable in synthetic reference band. Cardiopulmonary and volume exams unremarkable; laboratory trend reviewed against longitudinal baseline.".to_owned(),
                },
                ProgressNoteSection {
                    heading: "Assessment & Plan".to_owned(),
                    body: "Continue current synthetic care protocol, maintain surveillance schedule, and re-evaluate lab markers at next planned visit.".to_owned(),
                },
            ],
        }));
    }

    sources.push(CitationSource::VirtualizedLabTable {
        id: CitationId::lab_table(),
        title: "Synthetic Longitudinal Clinical Chemistry & Hematology Ledger [12]".to_owned(),
        row_count: lab_row_count,
    });

    sources
}

fn generate_lab_rows(count: usize) -> Vec<ClinicalLabRow> {
    let mut rows = Vec::with_capacity(count);
    for row_index in 0..count {
        let spec = &SYNTHETIC_ANALYTES[row_index % SYNTHETIC_ANALYTES.len()];
        let cycle = (row_index / SYNTHETIC_ANALYTES.len()) as i32;
        let drift = ((cycle * 7) % 19) - 9;
        let value_tenths = spec.nominal_tenths + drift;
        let flag = if value_tenths > spec.high_tenths + 15 {
            LabFlag::Critical
        } else if value_tenths > spec.high_tenths {
            LabFlag::High
        } else if value_tenths < spec.low_tenths {
            LabFlag::Low
        } else {
            LabFlag::Normal
        };

        rows.push(ClinicalLabRow {
            row_index,
            recorded_day: ((row_index / 10) % 1_825) as u16,
            analyte: spec.name,
            value_tenths,
            unit: spec.unit,
            reference_range: spec.reference_range,
            flag,
            panel_code: spec.panel_code,
        });
    }
    rows
}

struct AnalyteTemplate {
    name: &'static str,
    unit: &'static str,
    reference_range: &'static str,
    panel_code: &'static str,
    nominal_tenths: i32,
    low_tenths: i32,
    high_tenths: i32,
}

const SYNTHETIC_ANALYTES: &[AnalyteTemplate] = &[
    AnalyteTemplate {
        name: "Sodium",
        unit: "mEq/L",
        reference_range: "136-145",
        panel_code: "BMP",
        nominal_tenths: 1395,
        low_tenths: 1360,
        high_tenths: 1450,
    },
    AnalyteTemplate {
        name: "Potassium",
        unit: "mEq/L",
        reference_range: "3.5-5.1",
        panel_code: "BMP",
        nominal_tenths: 42,
        low_tenths: 35,
        high_tenths: 51,
    },
    AnalyteTemplate {
        name: "Chloride",
        unit: "mEq/L",
        reference_range: "98-107",
        panel_code: "BMP",
        nominal_tenths: 1020,
        low_tenths: 980,
        high_tenths: 1070,
    },
    AnalyteTemplate {
        name: "Bicarbonate",
        unit: "mEq/L",
        reference_range: "22-29",
        panel_code: "BMP",
        nominal_tenths: 250,
        low_tenths: 220,
        high_tenths: 290,
    },
    AnalyteTemplate {
        name: "Creatinine",
        unit: "mg/dL",
        reference_range: "0.7-1.3",
        panel_code: "RENAL",
        nominal_tenths: 10,
        low_tenths: 7,
        high_tenths: 13,
    },
    AnalyteTemplate {
        name: "Glucose",
        unit: "mg/dL",
        reference_range: "70-99",
        panel_code: "CMP",
        nominal_tenths: 920,
        low_tenths: 700,
        high_tenths: 990,
    },
    AnalyteTemplate {
        name: "Hemoglobin",
        unit: "g/dL",
        reference_range: "12.0-16.0",
        panel_code: "CBC",
        nominal_tenths: 138,
        low_tenths: 120,
        high_tenths: 160,
    },
    AnalyteTemplate {
        name: "Platelets",
        unit: "K/uL",
        reference_range: "150-400",
        panel_code: "CBC",
        nominal_tenths: 2450,
        low_tenths: 1500,
        high_tenths: 4000,
    },
    AnalyteTemplate {
        name: "AST",
        unit: "U/L",
        reference_range: "10-40",
        panel_code: "LFT",
        nominal_tenths: 240,
        low_tenths: 100,
        high_tenths: 400,
    },
    AnalyteTemplate {
        name: "ALT",
        unit: "U/L",
        reference_range: "7-56",
        panel_code: "LFT",
        nominal_tenths: 260,
        low_tenths: 70,
        high_tenths: 560,
    },
];

const CLINICAL_SCENARIOS: &[&str] = &[
    "Cardiometabolic Longitudinal Review",
    "Renal Dosing Reconciliation",
    "Electrolyte Surveillance Check",
    "Anticoagulation Safety Assessment",
    "Endocrine Titration Follow-Up",
    "Pulmonary Baseline Evaluation",
    "Hematology Trend Verification",
    "Hepatic Function Panel Review",
    "Lipid Management Synthesis",
    "Preventive Screening Summary",
    "Perioperative Risk Stratification",
    "Ambulatory Care Transition Plan",
];

const STREAM_VOCABULARY: &[&str] = &[
    "synthetic ",
    "clinical ",
    "trend ",
    "remains ",
    "stable ",
    "within ",
    "reference ",
    "interval ",
    "electrolyte ",
    "panel ",
    "confirming ",
    "renal ",
    "clearance ",
    "margin ",
    "and ",
    "baseline ",
    "monitoring ",
    "plan ",
    "without ",
    "acute ",
    "variance. ",
];

/// Deterministic generator for 40 tokens/sec incremental streaming chunks.
#[derive(Debug, Clone)]
pub struct SyntheticTokenStream {
    next_seq: StreamSeq,
    token_cursor: usize,
}

impl SyntheticTokenStream {
    /// Starts a token stream at the given initial sequence number.
    #[must_use]
    pub const fn starting_at(next_seq: StreamSeq) -> Self {
        Self {
            next_seq,
            token_cursor: 0,
        }
    }

    /// Generates the next `StreamChunk` carrying `tokens_per_chunk` tokens.
    pub fn next_chunk(&mut self, tokens_per_chunk: u32) -> StreamChunk {
        let seq = self.next_seq;
        self.next_seq = seq.next();
        let count = tokens_per_chunk.max(1);

        let payload = if seq.0 > 0 && seq.0.is_multiple_of(45) {
            let cite_raw = (((seq.0 / 45) % u64::from(CitationId::MAX)) as u8) + 1;
            let cite = CitationId::new(cite_raw).unwrap_or_else(|_| CitationId::lab_table());
            DeltaPayload::AppendCitation(cite)
        } else if seq.0 > 0 && seq.0.is_multiple_of(120) {
            DeltaPayload::StartBlock(MarkdownBlock::Heading {
                level: HeadingLevel::H3,
                text: format!("Streaming Checkpoint {}", seq.0 / 120),
            })
        } else {
            let mut text = String::with_capacity((count as usize) * 10);
            for _ in 0..count {
                let word = STREAM_VOCABULARY[self.token_cursor % STREAM_VOCABULARY.len()];
                text.push_str(word);
                self.token_cursor = self.token_cursor.saturating_add(1);
            }
            DeltaPayload::AppendText(text)
        };

        StreamChunk {
            seq,
            token_count: count,
            payload,
        }
    }

    /// Returns the next sequence number that will be emitted.
    #[must_use]
    pub const fn peek_next_seq(&self) -> StreamSeq {
        self.next_seq
    }
}
