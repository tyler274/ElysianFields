//! Grapheme/orthographic-cluster boundary navigation and bidirectional direction rules.
//!
//! Keeps UTF-8 validity and combining-mark / Indic akshara conjunct integrity in one
//! single-owner module so cursor motion, selection, deletion, and IME preedit offsets
//! never split multi-codepoint clusters.

use crate::locale::TextDirection;

/// Validated UTF-8 byte offset guaranteed to lie on an orthographic cluster boundary
/// inside the document it was created for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ByteOffset(pub usize);

impl ByteOffset {
    #[must_use]
    pub const fn as_usize(self) -> usize {
        self.0
    }
}

/// Visual horizontal caret movement direction as pressed on a keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VisualDirection {
    Left,
    Right,
}

/// Logical buffer movement direction (towards start or end of UTF-8 text).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogicalDirection {
    Backward,
    Forward,
}

/// Single rule mapping visual horizontal arrow input into logical text progression
/// given the active paragraph's base direction.
#[must_use]
pub const fn visual_to_logical(
    visual: VisualDirection,
    direction: TextDirection,
) -> LogicalDirection {
    match (direction, visual) {
        (TextDirection::Ltr, VisualDirection::Left)
        | (TextDirection::Rtl, VisualDirection::Right) => LogicalDirection::Backward,
        (TextDirection::Ltr, VisualDirection::Right)
        | (TextDirection::Rtl, VisualDirection::Left) => LogicalDirection::Forward,
    }
}

/// Detect the strong paragraph direction from the first strong directional code point,
/// falling back to the locale's default direction when the buffer has no strong characters.
#[must_use]
pub fn detect_paragraph_direction(text: &str, fallback: TextDirection) -> TextDirection {
    for ch in text.chars() {
        if is_strong_rtl(ch) {
            return TextDirection::Rtl;
        }
        if is_strong_ltr(ch) {
            return TextDirection::Ltr;
        }
    }
    fallback
}

/// Returns true if `ch` is a strong RTL character (Arabic, Urdu, Syriac, Thaana, Hebrew,
/// or Arabic presentation forms).
#[must_use]
pub const fn is_strong_rtl(ch: char) -> bool {
    matches!(
        ch as u32,
        0x0590..=0x05FF
            | 0x0600..=0x06FF
            | 0x0750..=0x077F
            | 0x08A0..=0x08FF
            | 0xFB50..=0xFDFF
            | 0xFE70..=0xFEFF
    ) && !is_combining_mark(ch)
}

/// Returns true if `ch` is a strong LTR alphabetic or CJK/Indic base character.
#[must_use]
pub const fn is_strong_ltr(ch: char) -> bool {
    matches!(
        ch as u32,
        0x0041..=0x005A
            | 0x0061..=0x007A
            | 0x00C0..=0x024F
            | 0x1E00..=0x1EFF
            | 0x0900..=0x097F
            | 0x0980..=0x09FF
            | 0x0B80..=0x0BFF
            | 0x0C00..=0x0C7F
            | 0x3040..=0x30FF
            | 0x3400..=0x9FFF
            | 0xAC00..=0xD7AF
    ) && !is_combining_mark(ch)
}

/// Returns true if `ch` is an Indic virama / halant / pulli joining consonants into a conjunct.
#[must_use]
pub const fn is_indic_virama(ch: char) -> bool {
    matches!(
        ch,
        '\u{094D}' // Devanagari Sign Virama (Hindi, Marathi)
            | '\u{09CD}' // Bengali Sign Virama
            | '\u{0BCD}' // Tamil Sign Virama (Pulli)
            | '\u{0C4D}' // Telugu Sign Virama
    )
}

/// Returns true if `ch` is a combining mark, Arabic harakat, Indic dependent vowel sign (matra),
/// nukta, anusvara, visarga, variation selector, or joiner that attaches to the preceding base.
#[must_use]
pub const fn is_combining_mark(ch: char) -> bool {
    matches!(
        ch as u32,
        // General combining diacritical marks
        0x0300..=0x036F
            | 0x1AB0..=0x1AFF
            | 0x1DC0..=0x1DFF
            | 0x20D0..=0x20FF
            | 0xFE20..=0xFE2F
            // Hebrew cantillation & nikud vowel points
            | 0x0591..=0x05BD
            | 0x05BF
            | 0x05C1..=0x05C2
            | 0x05C4..=0x05C5
            | 0x05C7
            // Arabic combining marks & harakat (Arabic + Urdu)
            | 0x0610..=0x061A
            | 0x064B..=0x065F
            | 0x0670
            | 0x06D6..=0x06DC
            | 0x06DF..=0x06E4
            | 0x06E7..=0x06E8
            | 0x06EA..=0x06ED
            // Devanagari combining signs, nukta, matras, virama
            | 0x0900..=0x0903
            | 0x093A..=0x094F
            | 0x0951..=0x0957
            | 0x0962..=0x0963
            // Bengali combining signs, nukta, matras, virama
            | 0x0981..=0x0983
            | 0x09BC
            | 0x09BE..=0x09C4
            | 0x09C7..=0x09C8
            | 0x09CB..=0x09CD
            | 0x09D7
            | 0x09E2..=0x09E3
            // Tamil combining signs, matras, pulli
            | 0x0B82
            | 0x0BBE..=0x0BC2
            | 0x0BC6..=0x0BC8
            | 0x0BCA..=0x0BCD
            | 0x0BD7
            // Telugu combining signs, matras, virama
            | 0x0C00..=0x0C04
            | 0x0C3C
            | 0x0C3E..=0x0C44
            | 0x0C46..=0x0C48
            | 0x0C4A..=0x0C4D
            | 0x0C55..=0x0C56
            | 0x0C62..=0x0C63
            // Variation selectors & zero-width joiners
            | 0x200C..=0x200D
            | 0xFE00..=0xFE0F
            | 0xE0100..=0xE01EF
    )
}

/// Returns true if `ch` is an Indic consonant that can be linked after a virama into one akshara.
#[must_use]
pub const fn is_indic_consonant(ch: char) -> bool {
    matches!(
        ch as u32,
        0x0915..=0x0939 // Devanagari
            | 0x0958..=0x095F
            | 0x0995..=0x09B9 // Bengali
            | 0x09DC..=0x09DF
            | 0x0B95..=0x0BB9 // Tamil
            | 0x0C15..=0x0C39 // Telugu
    )
}

/// Compute all orthographic cluster boundary byte offsets in `text`, always starting with `0`
/// and ending with `text.len()`.
#[must_use]
pub fn cluster_boundaries(text: &str) -> Vec<usize> {
    let mut boundaries = Vec::with_capacity(text.len().saturating_add(1));
    boundaries.push(0);

    let mut prev_char: Option<char> = None;

    for (byte_idx, ch) in text.char_indices() {
        if byte_idx == 0 {
            prev_char = Some(ch);
            continue;
        }

        let stays_attached = match prev_char {
            None => false,
            Some('\r') if ch == '\n' => true,
            Some(prev) if is_indic_virama(prev) && (is_indic_consonant(ch) || ch == '\u{200D}') => {
                true
            }
            Some('\u{200D}') if ch != '\n' && ch != '\r' => true,
            Some(_) => is_combining_mark(ch),
        };

        if !stays_attached {
            boundaries.push(byte_idx);
        }

        prev_char = Some(ch);
    }

    if boundaries.last().copied() != Some(text.len()) {
        boundaries.push(text.len());
    }

    boundaries
}

/// Count orthographic clusters in `text`.
#[must_use]
pub fn cluster_count(text: &str) -> usize {
    cluster_boundaries(text).len().saturating_sub(1)
}

/// Snap an arbitrary byte offset down to the nearest valid orthographic cluster boundary in `text`.
#[must_use]
pub fn snap_to_cluster_boundary(text: &str, byte_offset: usize) -> ByteOffset {
    let clamped = byte_offset.min(text.len());
    let boundaries = cluster_boundaries(text);
    match boundaries.binary_search(&clamped) {
        Ok(idx) => ByteOffset(boundaries[idx]),
        Err(insert_idx) => ByteOffset(boundaries[insert_idx.saturating_sub(1)]),
    }
}

/// Advance one orthographic cluster forward from `offset`, clamping at `text.len()`.
#[must_use]
pub fn next_cluster_boundary(text: &str, offset: ByteOffset) -> ByteOffset {
    let boundaries = cluster_boundaries(text);
    let snapped = snap_to_cluster_boundary(text, offset.0).0;
    match boundaries.binary_search(&snapped) {
        Ok(idx) => boundaries
            .get(idx + 1)
            .copied()
            .map_or(ByteOffset(text.len()), ByteOffset),
        Err(idx) => boundaries
            .get(idx)
            .copied()
            .map_or(ByteOffset(text.len()), ByteOffset),
    }
}

/// Step one orthographic cluster backward from `offset`, clamping at `0`.
#[must_use]
pub fn prev_cluster_boundary(text: &str, offset: ByteOffset) -> ByteOffset {
    let boundaries = cluster_boundaries(text);
    let snapped = snap_to_cluster_boundary(text, offset.0).0;
    match boundaries.binary_search(&snapped) {
        Ok(0) => ByteOffset(0),
        Ok(idx) => ByteOffset(boundaries[idx - 1]),
        Err(0) => ByteOffset(0),
        Err(idx) => ByteOffset(boundaries[idx - 1]),
    }
}

/// Line and column coordinates (both 0-based; column counts orthographic clusters).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineColumn {
    pub line: usize,
    pub column: usize,
}

/// Compute 0-based line index and cluster column for a validated offset in `text`.
#[must_use]
pub fn line_column_at(text: &str, offset: ByteOffset) -> LineColumn {
    let snapped = snap_to_cluster_boundary(text, offset.0).0;
    let prefix = &text[..snapped];
    let line = prefix.bytes().filter(|&b| b == b'\n').count();
    let line_start = prefix.rfind('\n').map_or(0, |idx| idx + 1);
    let column = cluster_count(&text[line_start..snapped]);
    LineColumn { line, column }
}

/// Resolve a `(line, column)` pair into a cluster-safe byte offset, clamping to line bounds.
#[must_use]
pub fn offset_for_line_column(text: &str, target: LineColumn) -> ByteOffset {
    let mut current_line = 0usize;
    let mut line_start = 0usize;

    for (idx, ch) in text.char_indices() {
        if current_line == target.line {
            break;
        }
        if ch == '\n' {
            current_line += 1;
            line_start = idx + 1;
        }
    }

    if current_line < target.line {
        return ByteOffset(text.len());
    }

    let rest = &text[line_start..];
    let line_end_rel = rest.find('\n').unwrap_or(rest.len());
    let line_slice = &rest[..line_end_rel];
    let boundaries = cluster_boundaries(line_slice);
    let col_idx = target.column.min(boundaries.len().saturating_sub(1));
    ByteOffset(line_start + boundaries[col_idx])
}

/// Byte offset of the start of the line containing `offset`.
#[must_use]
pub fn line_start_offset(text: &str, offset: ByteOffset) -> ByteOffset {
    let snapped = snap_to_cluster_boundary(text, offset.0).0;
    let start = text[..snapped].rfind('\n').map_or(0, |idx| idx + 1);
    ByteOffset(start)
}

/// Byte offset of the end of the line containing `offset` (before `\n` if present).
#[must_use]
pub fn line_end_offset(text: &str, offset: ByteOffset) -> ByteOffset {
    let snapped = snap_to_cluster_boundary(text, offset.0).0;
    let rel_end = text[snapped..].find('\n').unwrap_or(text.len() - snapped);
    ByteOffset(snapped + rel_end)
}
