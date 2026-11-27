use std::fmt;

use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::i18n::TextDirection;

/// UTF-8 byte selection range anchored on orthographic cluster boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SelectionRange {
    pub anchor: usize,
    pub focus: usize,
}

impl SelectionRange {
    #[must_use]
    pub const fn caret(offset: usize) -> Self {
        Self {
            anchor: offset,
            focus: offset,
        }
    }

    #[must_use]
    pub const fn start(self) -> usize {
        if self.anchor <= self.focus {
            self.anchor
        } else {
            self.focus
        }
    }

    #[must_use]
    pub const fn end(self) -> usize {
        if self.anchor >= self.focus {
            self.anchor
        } else {
            self.focus
        }
    }

    #[must_use]
    pub const fn is_collapsed(self) -> bool {
        self.anchor == self.focus
    }
}

/// Active OS IME composition state (CJK, Hangul, Arabic, Indic preedit).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ImeCompositionState {
    #[default]
    Inactive,
    Composing {
        /// UTF-8 byte offset in `ComposerSurface::text` where preedit begins.
        range_start: usize,
        /// Uncommitted preedit string owned exclusively by the single composer state.
        preedit: String,
        /// Caret byte offset within `preedit`.
        cursor_offset: usize,
    },
}

/// Mobile soft-keyboard inset used to keep the composer visible above the IME.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct SoftKeyboardInsets {
    pub bottom_inset_px: f32,
}

/// Platform text/IME/clipboard adapter boundary.
///
/// The platform shell implements this narrow bridge without holding a second
/// authoritative copy of the editable document.
pub trait PlatformTextAdapter {
    fn read_clipboard(&self) -> Result<String, TextAdapterError>;
    fn write_clipboard(&mut self, text: &str) -> Result<(), TextAdapterError>;
    fn notify_ime_cursor(&mut self, selection: SelectionRange, direction: TextDirection);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextAdapterError {
    ClipboardUnavailable,
}

impl fmt::Display for TextAdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClipboardUnavailable => write!(f, "platform clipboard is unavailable"),
        }
    }
}

impl std::error::Error for TextAdapterError {}

/// Deterministic in-memory adapter for headless Bevy testing.
#[derive(Debug, Clone, Default)]
pub struct MemoryTextAdapter {
    pub clipboard: String,
    pub last_notified_selection: Option<SelectionRange>,
    pub last_notified_direction: Option<TextDirection>,
}

impl PlatformTextAdapter for MemoryTextAdapter {
    fn read_clipboard(&self) -> Result<String, TextAdapterError> {
        Ok(self.clipboard.clone())
    }

    fn write_clipboard(&mut self, text: &str) -> Result<(), TextAdapterError> {
        self.clipboard = text.to_owned();
        Ok(())
    }

    fn notify_ime_cursor(&mut self, selection: SelectionRange, direction: TextDirection) {
        self.last_notified_selection = Some(selection);
        self.last_notified_direction = Some(direction);
    }
}

/// Typed key/edit actions dispatched to [`ComposerSurface`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerAction {
    InsertText(String),
    /// `Enter` submits when not inside IME composition.
    EnterSubmit,
    /// `Shift+Enter` inserts a newline without submitting.
    ShiftEnterNewline,
    Backspace,
    DeleteForward,
    MovePrevCluster {
        extend_selection: bool,
    },
    MoveNextCluster {
        extend_selection: bool,
    },
    MoveStart {
        extend_selection: bool,
    },
    MoveEnd {
        extend_selection: bool,
    },
    SelectAll,
    Undo,
    Redo,
    ImePreedit {
        preedit: String,
        cursor_offset: usize,
    },
    ImeCommit(String),
    ImeCancel,
}

/// Outcome of executing a [`ComposerAction`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerOutcome {
    Submitted(String),
    Edited,
    SelectionChanged,
    NoOp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ComposerSnapshot {
    text: String,
    selection: SelectionRange,
}

/// Single-owner multiline composer supporting OS IME composition, RTL/Indic
/// orthographic cluster navigation, selection, clipboard, undo/redo, and
/// soft-keyboard viewport inset clamping.
#[derive(Debug, Clone, Resource)]
pub struct ComposerSurface {
    text: String,
    selection: SelectionRange,
    ime: ImeCompositionState,
    direction: TextDirection,
    keyboard_insets: SoftKeyboardInsets,
    undo_stack: Vec<ComposerSnapshot>,
    redo_stack: Vec<ComposerSnapshot>,
    max_history: usize,
}

impl Default for ComposerSurface {
    fn default() -> Self {
        Self::new(TextDirection::Ltr)
    }
}

impl ComposerSurface {
    pub const DEFAULT_MAX_HISTORY: usize = 128;

    #[must_use]
    pub fn new(direction: TextDirection) -> Self {
        Self {
            text: String::new(),
            selection: SelectionRange::caret(0),
            ime: ImeCompositionState::Inactive,
            direction,
            keyboard_insets: SoftKeyboardInsets::default(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_history: Self::DEFAULT_MAX_HISTORY,
        }
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub const fn selection(&self) -> SelectionRange {
        self.selection
    }

    #[must_use]
    pub const fn ime_state(&self) -> &ImeCompositionState {
        &self.ime
    }

    #[must_use]
    pub const fn direction(&self) -> TextDirection {
        self.direction
    }

    pub fn set_direction(&mut self, direction: TextDirection) {
        self.direction = direction;
    }

    pub fn set_soft_keyboard_insets(&mut self, insets: SoftKeyboardInsets) {
        self.keyboard_insets = SoftKeyboardInsets {
            bottom_inset_px: insets.bottom_inset_px.max(0.0),
        };
    }

    /// Computes available viewport height above the mobile soft keyboard.
    #[must_use]
    pub fn visible_height_above_keyboard(&self, viewport_height_px: f32) -> f32 {
        (viewport_height_px - self.keyboard_insets.bottom_inset_px).max(120.0)
    }

    /// Renders the single authoritative text view including any active IME preedit.
    #[must_use]
    pub fn display_text(&self) -> String {
        match &self.ime {
            ImeCompositionState::Inactive => self.text.clone(),
            ImeCompositionState::Composing {
                range_start,
                preedit,
                ..
            } => {
                let start = (*range_start).min(self.text.len());
                let mut combined = String::with_capacity(self.text.len() + preedit.len());
                combined.push_str(&self.text[..start]);
                combined.push_str(preedit);
                combined.push_str(&self.text[start..]);
                combined
            }
        }
    }

    #[must_use]
    pub fn selected_text(&self) -> &str {
        &self.text[self.selection.start()..self.selection.end()]
    }

    /// Copies the active selection to the platform clipboard without duplicating state.
    pub fn copy_to_adapter(
        &self,
        adapter: &mut dyn PlatformTextAdapter,
    ) -> Result<bool, TextAdapterError> {
        let selected = self.selected_text();
        if selected.is_empty() {
            return Ok(false);
        }
        adapter.write_clipboard(selected)?;
        Ok(true)
    }

    /// Cuts the active selection through the platform adapter.
    pub fn cut_to_adapter(
        &mut self,
        adapter: &mut dyn PlatformTextAdapter,
    ) -> Result<bool, TextAdapterError> {
        let selected = self.selected_text().to_owned();
        if selected.is_empty() {
            return Ok(false);
        }
        adapter.write_clipboard(&selected)?;
        self.push_undo_snapshot();
        self.delete_selected_range();
        adapter.notify_ime_cursor(self.selection, self.direction);
        Ok(true)
    }

    /// Pastes text from the platform clipboard into the single authoritative buffer.
    pub fn paste_from_adapter(
        &mut self,
        adapter: &mut dyn PlatformTextAdapter,
    ) -> Result<ComposerOutcome, TextAdapterError> {
        let pasted = adapter.read_clipboard()?;
        if pasted.is_empty() {
            return Ok(ComposerOutcome::NoOp);
        }
        let outcome = self.apply(ComposerAction::InsertText(pasted));
        adapter.notify_ime_cursor(self.selection, self.direction);
        Ok(outcome)
    }

    /// Applies a typed editing, navigation, IME, or submission action.
    pub fn apply(&mut self, action: ComposerAction) -> ComposerOutcome {
        match action {
            ComposerAction::InsertText(chunk) => {
                if chunk.is_empty() {
                    return ComposerOutcome::NoOp;
                }
                self.ime = ImeCompositionState::Inactive;
                self.push_undo_snapshot();
                self.replace_selection(&chunk);
                ComposerOutcome::Edited
            }
            ComposerAction::ShiftEnterNewline => {
                self.ime = ImeCompositionState::Inactive;
                self.push_undo_snapshot();
                self.replace_selection("\n");
                ComposerOutcome::Edited
            }
            ComposerAction::EnterSubmit => {
                if !matches!(self.ime, ImeCompositionState::Inactive) {
                    return ComposerOutcome::NoOp;
                }
                let trimmed = self.text.trim();
                if trimmed.is_empty() {
                    return ComposerOutcome::NoOp;
                }
                let payload = trimmed.to_owned();
                self.push_undo_snapshot();
                self.text.clear();
                self.selection = SelectionRange::caret(0);
                ComposerOutcome::Submitted(payload)
            }
            ComposerAction::Backspace => {
                self.ime = ImeCompositionState::Inactive;
                if !self.selection.is_collapsed() {
                    self.push_undo_snapshot();
                    self.delete_selected_range();
                    return ComposerOutcome::Edited;
                }
                if self.selection.focus == 0 {
                    return ComposerOutcome::NoOp;
                }
                let prev = prev_cluster_boundary(&self.text, self.selection.focus);
                self.push_undo_snapshot();
                self.text.drain(prev..self.selection.focus);
                self.selection = SelectionRange::caret(prev);
                ComposerOutcome::Edited
            }
            ComposerAction::DeleteForward => {
                self.ime = ImeCompositionState::Inactive;
                if !self.selection.is_collapsed() {
                    self.push_undo_snapshot();
                    self.delete_selected_range();
                    return ComposerOutcome::Edited;
                }
                if self.selection.focus >= self.text.len() {
                    return ComposerOutcome::NoOp;
                }
                let next = next_cluster_boundary(&self.text, self.selection.focus);
                self.push_undo_snapshot();
                self.text.drain(self.selection.focus..next);
                ComposerOutcome::Edited
            }
            ComposerAction::MovePrevCluster { extend_selection } => {
                let target = if !extend_selection && !self.selection.is_collapsed() {
                    self.selection.start()
                } else {
                    prev_cluster_boundary(&self.text, self.selection.focus)
                };
                self.update_selection(target, extend_selection)
            }
            ComposerAction::MoveNextCluster { extend_selection } => {
                let target = if !extend_selection && !self.selection.is_collapsed() {
                    self.selection.end()
                } else {
                    next_cluster_boundary(&self.text, self.selection.focus)
                };
                self.update_selection(target, extend_selection)
            }
            ComposerAction::MoveStart { extend_selection } => {
                self.update_selection(0, extend_selection)
            }
            ComposerAction::MoveEnd { extend_selection } => {
                self.update_selection(self.text.len(), extend_selection)
            }
            ComposerAction::SelectAll => {
                self.selection = SelectionRange {
                    anchor: 0,
                    focus: self.text.len(),
                };
                ComposerOutcome::SelectionChanged
            }
            ComposerAction::Undo => {
                self.ime = ImeCompositionState::Inactive;
                let Some(previous) = self.undo_stack.pop() else {
                    return ComposerOutcome::NoOp;
                };
                self.redo_stack.push(ComposerSnapshot {
                    text: self.text.clone(),
                    selection: self.selection,
                });
                self.text = previous.text;
                self.selection = previous.selection;
                ComposerOutcome::Edited
            }
            ComposerAction::Redo => {
                self.ime = ImeCompositionState::Inactive;
                let Some(next) = self.redo_stack.pop() else {
                    return ComposerOutcome::NoOp;
                };
                self.undo_stack.push(ComposerSnapshot {
                    text: self.text.clone(),
                    selection: self.selection,
                });
                self.text = next.text;
                self.selection = next.selection;
                ComposerOutcome::Edited
            }
            ComposerAction::ImePreedit {
                preedit,
                cursor_offset,
            } => {
                if preedit.is_empty() {
                    self.ime = ImeCompositionState::Inactive;
                } else {
                    if !self.selection.is_collapsed() {
                        self.push_undo_snapshot();
                        self.delete_selected_range();
                    }
                    let clamped_cursor = cursor_offset.min(preedit.len());
                    self.ime = ImeCompositionState::Composing {
                        range_start: self.selection.focus,
                        preedit,
                        cursor_offset: clamped_cursor,
                    };
                }
                ComposerOutcome::Edited
            }
            ComposerAction::ImeCommit(committed) => {
                self.ime = ImeCompositionState::Inactive;
                if committed.is_empty() {
                    return ComposerOutcome::NoOp;
                }
                self.push_undo_snapshot();
                self.replace_selection(&committed);
                ComposerOutcome::Edited
            }
            ComposerAction::ImeCancel => {
                if matches!(self.ime, ImeCompositionState::Inactive) {
                    ComposerOutcome::NoOp
                } else {
                    self.ime = ImeCompositionState::Inactive;
                    ComposerOutcome::Edited
                }
            }
        }
    }

    fn update_selection(&mut self, target: usize, extend_selection: bool) -> ComposerOutcome {
        let next = if extend_selection {
            SelectionRange {
                anchor: self.selection.anchor,
                focus: target,
            }
        } else {
            SelectionRange::caret(target)
        };
        if next == self.selection {
            ComposerOutcome::NoOp
        } else {
            self.selection = next;
            ComposerOutcome::SelectionChanged
        }
    }

    fn replace_selection(&mut self, replacement: &str) {
        let start = self.selection.start();
        let end = self.selection.end();
        self.text.replace_range(start..end, replacement);
        let new_caret = start + replacement.len();
        self.selection = SelectionRange::caret(new_caret);
    }

    fn delete_selected_range(&mut self) {
        let start = self.selection.start();
        let end = self.selection.end();
        self.text.drain(start..end);
        self.selection = SelectionRange::caret(start);
    }

    fn push_undo_snapshot(&mut self) {
        if self.undo_stack.len() >= self.max_history {
            let _ = self.undo_stack.remove(0);
        }
        self.undo_stack.push(ComposerSnapshot {
            text: self.text.clone(),
            selection: self.selection,
        });
        self.redo_stack.clear();
    }
}

/// Finds the next orthographic cluster boundary after `byte_offset`, keeping
/// combining diacritics, Arabic harakat, Indic virama conjuncts/matras, and
/// ZWJ sequences intact as single cursor steps.
#[must_use]
pub fn next_cluster_boundary(text: &str, byte_offset: usize) -> usize {
    if byte_offset >= text.len() {
        return text.len();
    }

    let mut chars = text[byte_offset..].char_indices().peekable();
    let Some((_, first)) = chars.next() else {
        return text.len();
    };
    let mut end = byte_offset + first.len_utf8();

    // Handle CRLF as a single cluster.
    if first == '\r'
        && let Some((_, '\n')) = chars.peek().copied()
    {
        let _ = chars.next();
        return end + 1;
    }

    let mut prev_was_joiner = is_indic_virama(first) || first == '\u{200D}';
    while let Some((_, ch)) = chars.peek().copied() {
        if prev_was_joiner {
            // Pull the joined consonant/emoji into the same orthographic cluster.
            let _ = chars.next();
            end += ch.len_utf8();
            prev_was_joiner = is_indic_virama(ch) || ch == '\u{200D}';
            continue;
        }
        if is_combining_or_indic_mark(ch) || is_indic_virama(ch) || ch == '\u{200D}' {
            let _ = chars.next();
            end += ch.len_utf8();
            prev_was_joiner = is_indic_virama(ch) || ch == '\u{200D}';
            continue;
        }
        break;
    }

    end
}

/// Finds the previous orthographic cluster boundary strictly before `byte_offset`.
#[must_use]
pub fn prev_cluster_boundary(text: &str, byte_offset: usize) -> usize {
    let clamped = byte_offset.min(text.len());
    if clamped == 0 {
        return 0;
    }
    let mut cursor = 0;
    let mut prev = 0;
    while cursor < clamped {
        prev = cursor;
        cursor = next_cluster_boundary(text, cursor);
        if cursor <= prev {
            break;
        }
    }
    prev
}

fn is_indic_virama(ch: char) -> bool {
    matches!(
        ch,
        '\u{094D}' // Devanagari virama (halant)
            | '\u{09CD}' // Bengali virama
            | '\u{0BCD}' // Tamil virama
            | '\u{0C4D}' // Telugu virama
    )
}

fn is_combining_or_indic_mark(ch: char) -> bool {
    let cp = ch as u32;
    matches!(
        cp,
        0x0300..=0x036F // Combining diacritical marks
            | 0x064B..=0x065F // Arabic harakat / shadda / sukun
            | 0x0670 // Arabic superscript alef
            | 0x0901..=0x0903 // Devanagari signs
            | 0x093A..=0x094C // Devanagari vowel matras
            | 0x094E..=0x0957 // Devanagari additional marks
            | 0x0981..=0x0983 // Bengali signs
            | 0x09BC..=0x09CC // Bengali matras
            | 0x0B82 // Tamil anusvara
            | 0x0BBE..=0x0BCC // Tamil matras
            | 0x0C00..=0x0C04 // Telugu signs
            | 0x0C3E..=0x0C4C // Telugu matras
            | 0xFE00..=0xFE0F // Variation selectors
    )
}
