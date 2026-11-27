//! Domain types for the M1 clinical streaming transcript and virtualized record prototype.
//!
//! All types are pure Rust and independent of rendering entities or view lifetimes.

use std::fmt;

/// Typed errors for domain validation, streaming sequence checks, and snapshot restoration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamingPerfError {
    /// Citation numbers are strictly bounded to the 12 inline citations `[1]`..=`[12]`.
    InvalidCitationId { raw: u8 },
    /// Incoming stream chunk arrived out of order relative to the expected sequence number.
    OutOfOrderStreamChunk {
        expected: StreamSeq,
        actual: StreamSeq,
    },
    /// Attempted to append a delta chunk when no assistant turn is actively streaming.
    NoActiveStreamingTurn,
    /// Attempted to open a citation that does not exist in the citation store.
    MissingCitationSource { citation: CitationId },
    /// Virtualized window dimensions must be strictly positive finite values.
    InvalidViewportGeometry {
        viewport_height_px: u32,
        row_height_px: u32,
    },
    /// Snapshot restoration referenced a turn or sequence incompatible with the active transcript.
    InvalidSnapshotState { reason: SnapshotInvalidReason },
}

/// Specific reason why a background/return session snapshot could not be restored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotInvalidReason {
    /// The snapshot's active turn ID does not exist in the restored transcript.
    UnknownActiveTurn,
    /// The snapshot's open citation ID is missing from the citation store.
    UnknownCitation,
}

impl fmt::Display for StreamingPerfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCitationId { raw } => {
                write!(f, "invalid citation id {raw}: expected 1..=12")
            }
            Self::OutOfOrderStreamChunk { expected, actual } => write!(
                f,
                "out-of-order stream chunk: expected sequence {}, got {}",
                expected.0, actual.0
            ),
            Self::NoActiveStreamingTurn => write!(f, "no active streaming assistant turn"),
            Self::MissingCitationSource { citation } => {
                write!(f, "missing citation source for [{}]", citation.as_u8())
            }
            Self::InvalidViewportGeometry {
                viewport_height_px,
                row_height_px,
            } => write!(
                f,
                "invalid viewport geometry: height={viewport_height_px}px row_height={row_height_px}px"
            ),
            Self::InvalidSnapshotState { reason } => match reason {
                SnapshotInvalidReason::UnknownActiveTurn => {
                    write!(f, "snapshot references unknown active turn")
                }
                SnapshotInvalidReason::UnknownCitation => {
                    write!(f, "snapshot references unknown citation source")
                }
            },
        }
    }
}

impl std::error::Error for StreamingPerfError {}

/// Unique zero-based index of a conversation turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TurnId(pub u32);

/// Unique identifier for a flattened renderable markdown block across the transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockId(pub u32);

/// Monotonically increasing sequence number for streaming delta chunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StreamSeq(pub u64);

impl StreamSeq {
    /// Returns the next expected sequence number.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

/// Validated inline citation identifier in the range `[1]`..=`[12]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CitationId(u8);

impl CitationId {
    /// Lowest valid inline citation marker (`[1]`).
    pub const MIN: u8 = 1;
    /// Highest valid inline citation marker (`[12]`).
    pub const MAX: u8 = 12;

    /// Parses and validates a citation number into a `CitationId`.
    pub const fn new(raw: u8) -> Result<Self, StreamingPerfError> {
        if raw >= Self::MIN && raw <= Self::MAX {
            Ok(Self(raw))
        } else {
            Err(StreamingPerfError::InvalidCitationId { raw })
        }
    }

    /// Standard multi-section progress note citation `[1]`.
    #[must_use]
    pub const fn progress_note() -> Self {
        Self(1)
    }

    /// 5,000-row virtualized clinical lab table citation `[12]`.
    #[must_use]
    pub const fn lab_table() -> Self {
        Self(12)
    }

    /// Returns the 1-based numeric marker value.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self.0
    }
}

/// Speaker role for a clinical conversation turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnAuthor {
    Clinician,
    Assistant,
}

/// Markdown heading depth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadingLevel {
    H1,
    H2,
    H3,
}

/// List presentation kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    Bulleted,
    Numbered,
}

/// Language tag for fenced code blocks in clinical responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeLanguage {
    Json,
    FhirPath,
    PlainText,
}

/// Inline span inside a paragraph block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineSpan {
    Text(String),
    Citation(CitationId),
    InlineCode(String),
    Link { label: String, url: String },
}

impl InlineSpan {
    /// Character count contributed by this inline span.
    #[must_use]
    pub fn char_len(&self) -> usize {
        match self {
            Self::Text(text) | Self::InlineCode(text) => text.chars().count(),
            Self::Citation(id) => {
                if id.as_u8() >= 10 {
                    4
                } else {
                    3
                }
            }
            Self::Link { label, url } => label.chars().count() + url.chars().count() + 4,
        }
    }
}

/// Structured rich markdown AST block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkdownBlock {
    Heading {
        level: HeadingLevel,
        text: String,
    },
    Paragraph {
        spans: Vec<InlineSpan>,
    },
    List {
        kind: ListKind,
        items: Vec<String>,
    },
    CodeBlock {
        language: CodeLanguage,
        code: String,
    },
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Link {
        label: String,
        url: String,
    },
}

impl MarkdownBlock {
    /// Estimates the rendered height in logical pixels for layout virtualization.
    #[must_use]
    pub fn estimated_height_px(&self) -> f32 {
        match self {
            Self::Heading { .. } => 34.0,
            Self::Paragraph { spans } => {
                let chars: usize = spans.iter().map(InlineSpan::char_len).sum();
                let lines = (chars / 84).saturating_add(1);
                (lines as f32) * 20.0 + 10.0
            }
            Self::List { items, .. } => (items.len().max(1) as f32) * 22.0 + 8.0,
            Self::CodeBlock { code, .. } => {
                let lines = code.lines().count().max(1);
                (lines as f32) * 18.0 + 16.0
            }
            Self::Table { rows, .. } => ((rows.len() + 1) as f32) * 26.0 + 10.0,
            Self::Link { .. } => 26.0,
        }
    }

    /// Approximate visible character volume contributed by this block.
    #[must_use]
    pub fn char_count(&self) -> usize {
        match self {
            Self::Heading { text, .. } => text.chars().count(),
            Self::Paragraph { spans } => spans.iter().map(InlineSpan::char_len).sum(),
            Self::List { items, .. } => items.iter().map(|item| item.chars().count()).sum(),
            Self::CodeBlock { code, .. } => code.chars().count(),
            Self::Table { headers, rows } => {
                let header_chars: usize = headers.iter().map(|h| h.chars().count()).sum();
                let row_chars: usize = rows
                    .iter()
                    .flat_map(|row| row.iter())
                    .map(|cell| cell.chars().count())
                    .sum();
                header_chars + row_chars
            }
            Self::Link { label, url } => label.chars().count() + url.chars().count(),
        }
    }

    /// Formats a concise display preview for a virtualized UI text slot without cloning the whole transcript.
    pub fn write_preview_into(&self, out: &mut String) {
        out.clear();
        match self {
            Self::Heading { level, text } => {
                let prefix = match level {
                    HeadingLevel::H1 => "# ",
                    HeadingLevel::H2 => "## ",
                    HeadingLevel::H3 => "### ",
                };
                out.push_str(prefix);
                out.push_str(text);
            }
            Self::Paragraph { spans } => {
                for span in spans {
                    match span {
                        InlineSpan::Text(t) => out.push_str(t),
                        InlineSpan::Citation(id) => {
                            out.push('[');
                            push_u8(out, id.as_u8());
                            out.push(']');
                        }
                        InlineSpan::InlineCode(code) => {
                            out.push('`');
                            out.push_str(code);
                            out.push('`');
                        }
                        InlineSpan::Link { label, .. } => out.push_str(label),
                    }
                }
            }
            Self::List { kind, items } => {
                for (idx, item) in items.iter().enumerate() {
                    if idx > 0 {
                        out.push('\n');
                    }
                    match kind {
                        ListKind::Bulleted => out.push_str("- "),
                        ListKind::Numbered => {
                            push_usize(out, idx + 1);
                            out.push_str(". ");
                        }
                    }
                    out.push_str(item);
                }
            }
            Self::CodeBlock { language, code } => {
                let tag = match language {
                    CodeLanguage::Json => "json",
                    CodeLanguage::FhirPath => "fhirpath",
                    CodeLanguage::PlainText => "text",
                };
                out.push_str("```");
                out.push_str(tag);
                out.push('\n');
                out.push_str(code);
                out.push_str("\n```");
            }
            Self::Table { headers, rows } => {
                out.push_str(&headers.join(" | "));
                for row in rows {
                    out.push('\n');
                    out.push_str(&row.join(" | "));
                }
            }
            Self::Link { label, url } => {
                out.push('[');
                out.push_str(label);
                out.push_str("](");
                out.push_str(url);
                out.push(')');
            }
        }
    }
}

fn push_u8(out: &mut String, value: u8) {
    if value >= 10 {
        out.push(char::from(b'0' + (value / 10)));
    }
    out.push(char::from(b'0' + (value % 10)));
}

fn push_usize(out: &mut String, mut value: usize) {
    let mut digits = [0u8; 20];
    let mut len = 0;
    if value == 0 {
        out.push('0');
        return;
    }
    while value > 0 {
        digits[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }
    for byte in digits[..len].iter().rev() {
        out.push(char::from(*byte));
    }
}

/// A single turn in the clinical transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationTurn {
    pub id: TurnId,
    pub author: TurnAuthor,
    pub blocks: Vec<TranscriptBlock>,
}

/// A block inside a conversation turn with a stable ID and monotonic revision counter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptBlock {
    pub id: BlockId,
    pub turn_id: TurnId,
    pub author: TurnAuthor,
    pub revision: u64,
    pub content: MarkdownBlock,
}

/// Clinical abnormal flag for a synthetic laboratory observation row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabFlag {
    Normal,
    High,
    Low,
    Critical,
}

impl LabFlag {
    #[must_use]
    pub const fn as_code(self) -> &'static str {
        match self {
            Self::Normal => "N",
            Self::High => "H",
            Self::Low => "L",
            Self::Critical => "CRIT",
        }
    }
}

/// One row in the 5,000-row structured synthetic clinical lab/record table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClinicalLabRow {
    pub row_index: usize,
    pub recorded_day: u16,
    pub analyte: &'static str,
    pub value_tenths: i32,
    pub unit: &'static str,
    pub reference_range: &'static str,
    pub flag: LabFlag,
    pub panel_code: &'static str,
}

impl ClinicalLabRow {
    /// Formats this row into a reusable caller-owned string buffer with zero intermediate `format!` allocations.
    pub fn write_row_text(&self, out: &mut String) {
        out.clear();
        out.push('#');
        push_usize(out, self.row_index + 1);
        out.push_str(" | Day-");
        push_usize(out, usize::from(self.recorded_day));
        out.push_str(" | ");
        out.push_str(self.panel_code);
        out.push_str(" | ");
        out.push_str(self.analyte);
        out.push_str(" | ");
        let negative = self.value_tenths < 0;
        let abs = self.value_tenths.unsigned_abs() as usize;
        if negative {
            out.push('-');
        }
        push_usize(out, abs / 10);
        out.push('.');
        out.push(char::from(b'0' + (abs % 10) as u8));
        out.push(' ');
        out.push_str(self.unit);
        out.push_str(" (");
        out.push_str(self.reference_range);
        out.push_str(") [");
        out.push_str(self.flag.as_code());
        out.push(']');
    }
}

/// Multi-section synthetic progress note backing citations `[1]`..=`[11]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressNoteCitation {
    pub id: CitationId,
    pub title: String,
    pub encounter_label: String,
    pub sections: Vec<ProgressNoteSection>,
}

/// One structured section inside a synthetic progress note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressNoteSection {
    pub heading: String,
    pub body: String,
}

/// Backing document for a citation marker `[1]`..=`[12]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CitationSource {
    ProgressNote(ProgressNoteCitation),
    VirtualizedLabTable {
        id: CitationId,
        title: String,
        row_count: usize,
    },
}

/// Incremental delta payload appended to the active assistant turn during streaming.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeltaPayload {
    /// Append token text directly onto the tail paragraph or code block in O(1).
    AppendText(String),
    /// Append an inline citation marker `[1]`..=`[12]` onto the tail paragraph in O(1).
    AppendCitation(CitationId),
    /// Start a new markdown block at the tail of the active turn in O(1).
    StartBlock(MarkdownBlock),
}

/// One sequenced streaming chunk delivered at 40 tokens/sec cadence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamChunk {
    pub seq: StreamSeq,
    pub token_count: u32,
    pub payload: DeltaPayload,
}
