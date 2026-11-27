//! Single-owner multiline text composer domain state machine.
//!
//! Owns committed UTF-8 text, grapheme-safe selection and caret state, bounded undo/redo
//! history, OS IME preedit composition state, dictation insertion, clipboard operations,
//! mobile soft-keyboard viewport clamping, and background/foreground focus restoration.
//! Completely independent of ECS entities and platform rendering lifetimes.

use std::borrow::Cow;
use std::collections::VecDeque;

use crate::grapheme::{
    ByteOffset, LineColumn, LogicalDirection, VisualDirection, cluster_count,
    detect_paragraph_direction, line_column_at, line_end_offset, line_start_offset,
    next_cluster_boundary, offset_for_line_column, prev_cluster_boundary, snap_to_cluster_boundary,
    visual_to_logical,
};
use crate::locale::{Locale, TextDirection, WordSpacingPolicy};

/// Maximum number of undo/redo snapshots retained by a composer instance.
pub const DEFAULT_MAX_UNDO_DEPTH: usize = 128;

const COMPOSER_HORIZONTAL_PADDING_PX: f32 = 12.0;
const COMPOSER_VERTICAL_PADDING_PX: f32 = 8.0;
const CARET_WIDTH_PX: f32 = 2.0;

/// Monotonic revision counter incremented on every committed text mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct DocumentRevision(pub u64);

impl DocumentRevision {
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }

    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// Screen-space rectangle in logical pixels used for caret rendering and IME candidate anchoring.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaretRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Validated caret or non-empty selection range inside [`ComposerDocument`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorState {
    /// Collapsed insertion caret.
    Caret(ByteOffset),
    /// Non-empty selection range where `anchor != focus`.
    Selection {
        anchor: ByteOffset,
        focus: ByteOffset,
    },
}

impl CursorState {
    /// Construct a normalized [`CursorState`], collapsing equal offsets to [`CursorState::Caret`].
    #[must_use]
    pub fn from_anchor_focus(anchor: ByteOffset, focus: ByteOffset) -> Self {
        if anchor == focus {
            Self::Caret(anchor)
        } else {
            Self::Selection { anchor, focus }
        }
    }

    /// Active focus end of the caret or selection.
    #[must_use]
    pub const fn focus(self) -> ByteOffset {
        match self {
            Self::Caret(offset) => offset,
            Self::Selection { focus, .. } => focus,
        }
    }

    /// Anchor end of the caret or selection.
    #[must_use]
    pub const fn anchor(self) -> ByteOffset {
        match self {
            Self::Caret(offset) => offset,
            Self::Selection { anchor, .. } => anchor,
        }
    }

    /// Lower byte bound of the cursor/selection range.
    #[must_use]
    pub fn start(self) -> ByteOffset {
        match self {
            Self::Caret(offset) => offset,
            Self::Selection { anchor, focus } => anchor.min(focus),
        }
    }

    /// Upper byte bound of the cursor/selection range.
    #[must_use]
    pub fn end(self) -> ByteOffset {
        match self {
            Self::Caret(offset) => offset,
            Self::Selection { anchor, focus } => anchor.max(focus),
        }
    }

    /// Return the selected UTF-8 slice if a non-empty selection is active.
    #[must_use]
    pub fn selected_text(self, text: &str) -> Option<&str> {
        match self {
            Self::Caret(_) => None,
            Self::Selection { anchor, focus } => {
                let start = anchor.min(focus).as_usize();
                let end = anchor.max(focus).as_usize();
                text.get(start..end)
            }
        }
    }
}

/// Active OS IME preedit composition state.
#[derive(Debug, Clone, PartialEq)]
pub enum ImeComposition {
    /// No active IME composition.
    Idle,
    /// Active uncommitted IME preedit text anchored at the composer caret.
    Composing {
        text: String,
        /// Byte offset within `text` on a valid cluster boundary.
        cursor_offset: usize,
        /// Viewport-space anchor rectangle for the OS candidate window.
        candidate_anchor: CaretRect,
    },
}

/// Focus lifecycle state, including suspension across mobile background/foreground transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusState {
    Unfocused,
    Focused,
    /// App entered background while focused; restores exact cursor and scroll on foreground.
    SuspendedWaitingRestore {
        saved_cursor: CursorState,
        saved_scroll_tenths_px: i32,
    },
}

/// Mobile or tablet soft-keyboard occlusion state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SoftKeyboardState {
    Hidden,
    Visible { inset_bottom_px: f32 },
}

/// Logical viewport and typography metrics used to compute caret rectangles and scroll clamping.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportMetrics {
    pub width_px: f32,
    pub height_px: f32,
    pub line_height_px: f32,
    pub char_advance_px: f32,
    pub composer_min_height_px: f32,
    pub composer_max_height_px: f32,
    pub scroll_offset_px: f32,
}

impl Default for ViewportMetrics {
    fn default() -> Self {
        Self {
            width_px: 800.0,
            height_px: 600.0,
            line_height_px: 22.0,
            char_advance_px: 9.0,
            composer_min_height_px: 56.0,
            composer_max_height_px: 176.0,
            scroll_offset_px: 0.0,
        }
    }
}

/// Whether scroll clamping had to adjust the composer scroll offset to keep the caret visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollAdjustment {
    Unchanged,
    ClampedForCaret,
}

/// Computed layout snapshot for rendering and platform adapter readback.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComposerLayoutSnapshot {
    pub composer_top_px: f32,
    pub composer_height_px: f32,
    pub visible_viewport_height_px: f32,
    pub caret_rect: CaretRect,
    pub scroll_offset_px: f32,
    pub total_lines: usize,
    pub scroll_adjustment: ScrollAdjustment,
}

/// Caret motion selection policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionBehavior {
    Collapse,
    Extend,
}

/// Caret movement commands emitted by keyboard or touch gestures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveDirection {
    VisualLeft,
    VisualRight,
    LineUp,
    LineDown,
    LineStart,
    LineEnd,
    DocumentStart,
    DocumentEnd,
}

/// Word-spacing rule when inserting OS dictation results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DictationSpacing {
    /// Insert a separating space when the active locale uses word spaces and adjacent text needs one.
    AutoSpace,
    /// Insert the dictated text verbatim.
    Exact,
}

/// Dictation payload delivered by OS speech-to-text services.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictationSegment {
    pub text: String,
    pub spacing: DictationSpacing,
}

/// Typed commands accepted by [`ComposerDocument`] from platform adapters and keyboard input.
#[derive(Debug, Clone, PartialEq)]
pub enum TextInputCommand {
    Focus,
    Blur,
    AppBackgrounded,
    AppForegrounded,
    SetLocale(Locale),
    InsertText(String),
    InsertNewline,
    Backspace,
    DeleteForward,
    MoveCaret {
        direction: MoveDirection,
        selection: SelectionBehavior,
    },
    SelectAll,
    Cut,
    Copy,
    Paste(String),
    Undo,
    Redo,
    ImeStart,
    ImeUpdatePreedit {
        text: String,
        cursor_offset: Option<usize>,
    },
    ImeCommit(String),
    ImeCancel,
    DictationInsert(DictationSegment),
    UpdateViewport {
        width_px: f32,
        height_px: f32,
    },
    UpdateSoftKeyboard(SoftKeyboardState),
}

/// Whether a clipboard command modified the document.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClipboardMutation {
    CopiedOnly,
    CutApplied {
        revision: DocumentRevision,
        caret_rect: CaretRect,
    },
}

/// Reason a command produced no state change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoOpReason {
    NothingSelected,
    UndoStackEmpty,
    RedoStackEmpty,
    AtBoundary,
    UnfocusedIgnored,
    AlreadyInState,
    EmptyInput,
}

/// Typed outcome returned by [`ComposerDocument::apply`].
#[derive(Debug, Clone, PartialEq)]
pub enum CommandOutcome {
    DocumentChanged {
        revision: DocumentRevision,
        caret_rect: CaretRect,
    },
    SelectionChanged {
        caret_rect: CaretRect,
    },
    ClipboardProduced {
        text: String,
        mutation: ClipboardMutation,
    },
    PreeditUpdated {
        anchor: CaretRect,
    },
    FocusChanged {
        state: FocusState,
        caret_rect: CaretRect,
    },
    ViewportAdjusted {
        layout: ComposerLayoutSnapshot,
    },
    NoOp(NoOpReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UndoSnapshot {
    text: String,
    cursor: CursorState,
}

/// Single authoritative state owner for multiline clinical composer text input.
#[derive(Debug, Clone)]
pub struct ComposerDocument {
    text: String,
    cursor: CursorState,
    locale: Locale,
    ime: ImeComposition,
    focus: FocusState,
    soft_keyboard: SoftKeyboardState,
    viewport: ViewportMetrics,
    revision: DocumentRevision,
    undo_stack: VecDeque<UndoSnapshot>,
    redo_stack: VecDeque<UndoSnapshot>,
    max_undo_depth: usize,
}

impl Default for ComposerDocument {
    fn default() -> Self {
        Self::new(Locale::En)
    }
}

impl ComposerDocument {
    /// Create a new unfocused composer initialized to `locale`.
    #[must_use]
    pub fn new(locale: Locale) -> Self {
        Self {
            text: String::new(),
            cursor: CursorState::Caret(ByteOffset(0)),
            locale,
            ime: ImeComposition::Idle,
            focus: FocusState::Unfocused,
            soft_keyboard: SoftKeyboardState::Hidden,
            viewport: ViewportMetrics::default(),
            revision: DocumentRevision(0),
            undo_stack: VecDeque::new(),
            redo_stack: VecDeque::new(),
            max_undo_depth: DEFAULT_MAX_UNDO_DEPTH,
        }
    }

    /// Create a composer with custom viewport metrics and undo stack depth.
    #[must_use]
    pub fn with_config(locale: Locale, viewport: ViewportMetrics, max_undo_depth: usize) -> Self {
        let mut doc = Self::new(locale);
        doc.viewport = viewport;
        doc.max_undo_depth = max_undo_depth.max(1);
        doc
    }

    /// Committed UTF-8 text buffer.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Read-only rendered view of committed text with any active IME preedit spliced visually.
    #[must_use]
    pub fn display_text(&self) -> Cow<'_, str> {
        match &self.ime {
            ImeComposition::Idle => Cow::Borrowed(&self.text),
            ImeComposition::Composing { text: preedit, .. } if preedit.is_empty() => {
                Cow::Borrowed(&self.text)
            }
            ImeComposition::Composing { text: preedit, .. } => {
                let start = self.cursor.start().as_usize();
                let end = self.cursor.end().as_usize();
                let mut combined =
                    String::with_capacity(self.text.len() - (end - start) + preedit.len());
                combined.push_str(&self.text[..start]);
                combined.push_str(preedit);
                combined.push_str(&self.text[end..]);
                Cow::Owned(combined)
            }
        }
    }

    #[must_use]
    pub const fn cursor(&self) -> CursorState {
        self.cursor
    }

    #[must_use]
    pub const fn locale(&self) -> Locale {
        self.locale
    }

    #[must_use]
    pub const fn ime(&self) -> &ImeComposition {
        &self.ime
    }

    #[must_use]
    pub const fn focus(&self) -> FocusState {
        self.focus
    }

    #[must_use]
    pub const fn soft_keyboard(&self) -> SoftKeyboardState {
        self.soft_keyboard
    }

    #[must_use]
    pub const fn viewport(&self) -> ViewportMetrics {
        self.viewport
    }

    #[must_use]
    pub const fn revision(&self) -> DocumentRevision {
        self.revision
    }

    #[must_use]
    pub fn undo_depth(&self) -> usize {
        self.undo_stack.len()
    }

    #[must_use]
    pub fn redo_depth(&self) -> usize {
        self.redo_stack.len()
    }

    /// Single rule determining effective paragraph direction for the active line.
    #[must_use]
    pub fn effective_direction(&self) -> TextDirection {
        let line_start = line_start_offset(&self.text, self.cursor.focus()).as_usize();
        let line_end = line_end_offset(&self.text, self.cursor.focus()).as_usize();
        detect_paragraph_direction(&self.text[line_start..line_end], self.locale.direction())
    }

    /// Compute the current viewport-space caret rectangle without mutating state.
    #[must_use]
    pub fn caret_rect(&self) -> CaretRect {
        self.layout_snapshot().caret_rect
    }

    /// Compute the current composer layout snapshot and caret geometry.
    #[must_use]
    pub fn layout_snapshot(&self) -> ComposerLayoutSnapshot {
        let mut temp_scroll = self.viewport.scroll_offset_px;
        self.compute_layout_with_scroll(&mut temp_scroll)
    }

    /// Apply a single typed command from any platform adapter or input controller.
    pub fn apply(&mut self, command: TextInputCommand) -> CommandOutcome {
        match command {
            TextInputCommand::Focus => self.handle_focus(),
            TextInputCommand::Blur => self.handle_blur(),
            TextInputCommand::AppBackgrounded => self.handle_background(),
            TextInputCommand::AppForegrounded => self.handle_foreground(),
            TextInputCommand::SetLocale(locale) => self.handle_set_locale(locale),
            TextInputCommand::UpdateViewport {
                width_px,
                height_px,
            } => self.handle_update_viewport(width_px, height_px),
            TextInputCommand::UpdateSoftKeyboard(state) => self.handle_soft_keyboard(state),
            other => {
                if self.focus != FocusState::Focused {
                    return CommandOutcome::NoOp(NoOpReason::UnfocusedIgnored);
                }
                self.apply_focused_command(other)
            }
        }
    }

    fn apply_focused_command(&mut self, command: TextInputCommand) -> CommandOutcome {
        match command {
            TextInputCommand::InsertText(text) => self.handle_insert_text(&text),
            TextInputCommand::InsertNewline => self.handle_insert_text("\n"),
            TextInputCommand::Backspace => self.handle_backspace(),
            TextInputCommand::DeleteForward => self.handle_delete_forward(),
            TextInputCommand::MoveCaret {
                direction,
                selection,
            } => self.handle_move_caret(direction, selection),
            TextInputCommand::SelectAll => self.handle_select_all(),
            TextInputCommand::Cut => self.handle_cut(),
            TextInputCommand::Copy => self.handle_copy(),
            TextInputCommand::Paste(text) => self.handle_paste(&text),
            TextInputCommand::Undo => self.handle_undo(),
            TextInputCommand::Redo => self.handle_redo(),
            TextInputCommand::ImeStart => self.handle_ime_start(),
            TextInputCommand::ImeUpdatePreedit {
                text,
                cursor_offset,
            } => self.handle_ime_update(text, cursor_offset),
            TextInputCommand::ImeCommit(text) => self.handle_ime_commit(&text),
            TextInputCommand::ImeCancel => self.handle_ime_cancel(),
            TextInputCommand::DictationInsert(segment) => self.handle_dictation(segment),
            TextInputCommand::Focus
            | TextInputCommand::Blur
            | TextInputCommand::AppBackgrounded
            | TextInputCommand::AppForegrounded
            | TextInputCommand::SetLocale(_)
            | TextInputCommand::UpdateViewport { .. }
            | TextInputCommand::UpdateSoftKeyboard(_) => {
                CommandOutcome::NoOp(NoOpReason::AlreadyInState)
            }
        }
    }

    fn handle_focus(&mut self) -> CommandOutcome {
        if self.focus == FocusState::Focused {
            return CommandOutcome::NoOp(NoOpReason::AlreadyInState);
        }
        self.focus = FocusState::Focused;
        let layout = self.reclamp_scroll();
        CommandOutcome::FocusChanged {
            state: self.focus,
            caret_rect: layout.caret_rect,
        }
    }

    fn handle_blur(&mut self) -> CommandOutcome {
        if self.focus == FocusState::Unfocused {
            return CommandOutcome::NoOp(NoOpReason::AlreadyInState);
        }
        self.ime = ImeComposition::Idle;
        self.focus = FocusState::Unfocused;
        CommandOutcome::FocusChanged {
            state: self.focus,
            caret_rect: self.caret_rect(),
        }
    }

    fn handle_background(&mut self) -> CommandOutcome {
        match self.focus {
            FocusState::Focused => {
                // Cancel transient IME preedit without losing committed text.
                self.ime = ImeComposition::Idle;
                let saved_scroll_tenths_px = (self.viewport.scroll_offset_px * 10.0).round() as i32;
                self.focus = FocusState::SuspendedWaitingRestore {
                    saved_cursor: self.cursor,
                    saved_scroll_tenths_px,
                };
                CommandOutcome::FocusChanged {
                    state: self.focus,
                    caret_rect: self.caret_rect(),
                }
            }
            FocusState::Unfocused | FocusState::SuspendedWaitingRestore { .. } => {
                CommandOutcome::NoOp(NoOpReason::AlreadyInState)
            }
        }
    }

    fn handle_foreground(&mut self) -> CommandOutcome {
        match self.focus {
            FocusState::SuspendedWaitingRestore {
                saved_cursor,
                saved_scroll_tenths_px,
            } => {
                let anchor = snap_to_cluster_boundary(&self.text, saved_cursor.anchor().as_usize());
                let focus = snap_to_cluster_boundary(&self.text, saved_cursor.focus().as_usize());
                self.cursor = CursorState::from_anchor_focus(anchor, focus);
                self.viewport.scroll_offset_px = (saved_scroll_tenths_px as f32) / 10.0;
                self.focus = FocusState::Focused;
                let layout = self.reclamp_scroll();
                CommandOutcome::FocusChanged {
                    state: self.focus,
                    caret_rect: layout.caret_rect,
                }
            }
            FocusState::Unfocused | FocusState::Focused => {
                CommandOutcome::NoOp(NoOpReason::AlreadyInState)
            }
        }
    }

    fn handle_set_locale(&mut self, locale: Locale) -> CommandOutcome {
        if self.locale == locale {
            return CommandOutcome::NoOp(NoOpReason::AlreadyInState);
        }
        self.locale = locale;
        let layout = self.reclamp_scroll();
        self.refresh_preedit_anchor(layout.caret_rect);
        CommandOutcome::ViewportAdjusted { layout }
    }

    fn handle_update_viewport(&mut self, width_px: f32, height_px: f32) -> CommandOutcome {
        self.viewport.width_px = width_px.max(64.0);
        self.viewport.height_px = height_px.max(self.viewport.line_height_px * 2.0);
        let layout = self.reclamp_scroll();
        self.refresh_preedit_anchor(layout.caret_rect);
        CommandOutcome::ViewportAdjusted { layout }
    }

    fn handle_soft_keyboard(&mut self, state: SoftKeyboardState) -> CommandOutcome {
        self.soft_keyboard = match state {
            SoftKeyboardState::Hidden => SoftKeyboardState::Hidden,
            SoftKeyboardState::Visible { inset_bottom_px } => SoftKeyboardState::Visible {
                inset_bottom_px: inset_bottom_px.max(0.0),
            },
        };
        let layout = self.reclamp_scroll();
        self.refresh_preedit_anchor(layout.caret_rect);
        CommandOutcome::ViewportAdjusted { layout }
    }

    fn handle_insert_text(&mut self, raw: &str) -> CommandOutcome {
        let normalized = normalize_line_endings(raw);
        if normalized.is_empty() {
            return CommandOutcome::NoOp(NoOpReason::EmptyInput);
        }
        self.ime = ImeComposition::Idle;
        self.push_undo_snapshot();
        self.replace_selection_inner(&normalized);
        self.commit_mutation()
    }

    fn handle_paste(&mut self, raw: &str) -> CommandOutcome {
        self.handle_insert_text(raw)
    }

    fn handle_backspace(&mut self) -> CommandOutcome {
        self.ime = ImeComposition::Idle;
        match self.cursor {
            CursorState::Selection { .. } => {
                self.push_undo_snapshot();
                self.replace_selection_inner("");
                self.commit_mutation()
            }
            CursorState::Caret(offset) => {
                if offset.as_usize() == 0 {
                    return CommandOutcome::NoOp(NoOpReason::AtBoundary);
                }
                let prev = prev_cluster_boundary(&self.text, offset);
                self.push_undo_snapshot();
                self.text
                    .replace_range(prev.as_usize()..offset.as_usize(), "");
                self.cursor = CursorState::Caret(prev);
                self.commit_mutation()
            }
        }
    }

    fn handle_delete_forward(&mut self) -> CommandOutcome {
        self.ime = ImeComposition::Idle;
        match self.cursor {
            CursorState::Selection { .. } => {
                self.push_undo_snapshot();
                self.replace_selection_inner("");
                self.commit_mutation()
            }
            CursorState::Caret(offset) => {
                if offset.as_usize() >= self.text.len() {
                    return CommandOutcome::NoOp(NoOpReason::AtBoundary);
                }
                let next = next_cluster_boundary(&self.text, offset);
                self.push_undo_snapshot();
                self.text
                    .replace_range(offset.as_usize()..next.as_usize(), "");
                self.cursor = CursorState::Caret(offset);
                self.commit_mutation()
            }
        }
    }

    fn handle_move_caret(
        &mut self,
        direction: MoveDirection,
        selection: SelectionBehavior,
    ) -> CommandOutcome {
        let base_dir = self.effective_direction();
        let current_focus = self.cursor.focus();

        // Collapse active selection cleanly on horizontal non-extending moves.
        if selection == SelectionBehavior::Collapse
            && matches!(self.cursor, CursorState::Selection { .. })
        {
            match direction {
                MoveDirection::VisualLeft => {
                    let target = match visual_to_logical(VisualDirection::Left, base_dir) {
                        LogicalDirection::Backward => self.cursor.start(),
                        LogicalDirection::Forward => self.cursor.end(),
                    };
                    self.cursor = CursorState::Caret(target);
                    let layout = self.reclamp_scroll();
                    return CommandOutcome::SelectionChanged {
                        caret_rect: layout.caret_rect,
                    };
                }
                MoveDirection::VisualRight => {
                    let target = match visual_to_logical(VisualDirection::Right, base_dir) {
                        LogicalDirection::Backward => self.cursor.start(),
                        LogicalDirection::Forward => self.cursor.end(),
                    };
                    self.cursor = CursorState::Caret(target);
                    let layout = self.reclamp_scroll();
                    return CommandOutcome::SelectionChanged {
                        caret_rect: layout.caret_rect,
                    };
                }
                _ => {}
            }
        }

        let target = match direction {
            MoveDirection::VisualLeft => match visual_to_logical(VisualDirection::Left, base_dir) {
                LogicalDirection::Backward => prev_cluster_boundary(&self.text, current_focus),
                LogicalDirection::Forward => next_cluster_boundary(&self.text, current_focus),
            },
            MoveDirection::VisualRight => {
                match visual_to_logical(VisualDirection::Right, base_dir) {
                    LogicalDirection::Backward => prev_cluster_boundary(&self.text, current_focus),
                    LogicalDirection::Forward => next_cluster_boundary(&self.text, current_focus),
                }
            }
            MoveDirection::LineUp => {
                let lc = line_column_at(&self.text, current_focus);
                if lc.line == 0 {
                    ByteOffset(0)
                } else {
                    offset_for_line_column(
                        &self.text,
                        LineColumn {
                            line: lc.line - 1,
                            column: lc.column,
                        },
                    )
                }
            }
            MoveDirection::LineDown => {
                let lc = line_column_at(&self.text, current_focus);
                offset_for_line_column(
                    &self.text,
                    LineColumn {
                        line: lc.line + 1,
                        column: lc.column,
                    },
                )
            }
            MoveDirection::LineStart => line_start_offset(&self.text, current_focus),
            MoveDirection::LineEnd => line_end_offset(&self.text, current_focus),
            MoveDirection::DocumentStart => ByteOffset(0),
            MoveDirection::DocumentEnd => ByteOffset(self.text.len()),
        };

        let next_cursor = match selection {
            SelectionBehavior::Collapse => CursorState::Caret(target),
            SelectionBehavior::Extend => {
                CursorState::from_anchor_focus(self.cursor.anchor(), target)
            }
        };

        if next_cursor == self.cursor {
            return CommandOutcome::NoOp(NoOpReason::AtBoundary);
        }

        self.cursor = next_cursor;
        let layout = self.reclamp_scroll();
        CommandOutcome::SelectionChanged {
            caret_rect: layout.caret_rect,
        }
    }

    fn handle_select_all(&mut self) -> CommandOutcome {
        if self.text.is_empty() {
            return CommandOutcome::NoOp(NoOpReason::NothingSelected);
        }
        self.cursor = CursorState::from_anchor_focus(ByteOffset(0), ByteOffset(self.text.len()));
        let layout = self.reclamp_scroll();
        CommandOutcome::SelectionChanged {
            caret_rect: layout.caret_rect,
        }
    }

    fn handle_copy(&self) -> CommandOutcome {
        match self.cursor.selected_text(&self.text) {
            Some(selected) => CommandOutcome::ClipboardProduced {
                text: selected.to_owned(),
                mutation: ClipboardMutation::CopiedOnly,
            },
            None => CommandOutcome::NoOp(NoOpReason::NothingSelected),
        }
    }

    fn handle_cut(&mut self) -> CommandOutcome {
        let Some(selected) = self.cursor.selected_text(&self.text).map(str::to_owned) else {
            return CommandOutcome::NoOp(NoOpReason::NothingSelected);
        };
        self.ime = ImeComposition::Idle;
        self.push_undo_snapshot();
        self.replace_selection_inner("");
        self.revision = self.revision.next();
        let layout = self.reclamp_scroll();
        CommandOutcome::ClipboardProduced {
            text: selected,
            mutation: ClipboardMutation::CutApplied {
                revision: self.revision,
                caret_rect: layout.caret_rect,
            },
        }
    }

    fn handle_undo(&mut self) -> CommandOutcome {
        let Some(previous) = self.undo_stack.pop_back() else {
            return CommandOutcome::NoOp(NoOpReason::UndoStackEmpty);
        };
        self.ime = ImeComposition::Idle;
        let current = UndoSnapshot {
            text: self.text.clone(),
            cursor: self.cursor,
        };
        push_bounded(&mut self.redo_stack, current, self.max_undo_depth);
        self.text = previous.text;
        self.cursor = previous.cursor;
        self.revision = self.revision.next();
        let layout = self.reclamp_scroll();
        CommandOutcome::DocumentChanged {
            revision: self.revision,
            caret_rect: layout.caret_rect,
        }
    }

    fn handle_redo(&mut self) -> CommandOutcome {
        let Some(next) = self.redo_stack.pop_back() else {
            return CommandOutcome::NoOp(NoOpReason::RedoStackEmpty);
        };
        self.ime = ImeComposition::Idle;
        let current = UndoSnapshot {
            text: self.text.clone(),
            cursor: self.cursor,
        };
        push_bounded(&mut self.undo_stack, current, self.max_undo_depth);
        self.text = next.text;
        self.cursor = next.cursor;
        self.revision = self.revision.next();
        let layout = self.reclamp_scroll();
        CommandOutcome::DocumentChanged {
            revision: self.revision,
            caret_rect: layout.caret_rect,
        }
    }

    fn handle_ime_start(&mut self) -> CommandOutcome {
        let anchor = self.reclamp_scroll().caret_rect;
        self.ime = ImeComposition::Composing {
            text: String::new(),
            cursor_offset: 0,
            candidate_anchor: anchor,
        };
        CommandOutcome::PreeditUpdated { anchor }
    }

    fn handle_ime_update(
        &mut self,
        preedit: String,
        cursor_offset: Option<usize>,
    ) -> CommandOutcome {
        let validated_offset = cursor_offset
            .map(|raw| snap_to_cluster_boundary(&preedit, raw).as_usize())
            .unwrap_or(preedit.len());
        let base_caret = self.reclamp_scroll().caret_rect;
        let preedit_cols = cluster_count(&preedit[..validated_offset]) as f32;
        let advance = preedit_cols * self.viewport.char_advance_px;
        let anchor_x = match self.effective_direction() {
            TextDirection::Ltr => (base_caret.x + advance)
                .min(self.viewport.width_px - COMPOSER_HORIZONTAL_PADDING_PX),
            TextDirection::Rtl => (base_caret.x - advance).max(COMPOSER_HORIZONTAL_PADDING_PX),
        };
        let candidate_anchor = CaretRect {
            x: anchor_x,
            y: base_caret.y,
            width: base_caret.width,
            height: base_caret.height,
        };
        self.ime = ImeComposition::Composing {
            text: preedit,
            cursor_offset: validated_offset,
            candidate_anchor,
        };
        CommandOutcome::PreeditUpdated {
            anchor: candidate_anchor,
        }
    }

    fn handle_ime_commit(&mut self, committed: &str) -> CommandOutcome {
        self.ime = ImeComposition::Idle;
        let normalized = normalize_line_endings(committed);
        if normalized.is_empty() {
            return CommandOutcome::PreeditUpdated {
                anchor: self.caret_rect(),
            };
        }
        self.push_undo_snapshot();
        self.replace_selection_inner(&normalized);
        self.commit_mutation()
    }

    fn handle_ime_cancel(&mut self) -> CommandOutcome {
        if matches!(self.ime, ImeComposition::Idle) {
            return CommandOutcome::NoOp(NoOpReason::AlreadyInState);
        }
        self.ime = ImeComposition::Idle;
        CommandOutcome::PreeditUpdated {
            anchor: self.caret_rect(),
        }
    }

    fn handle_dictation(&mut self, segment: DictationSegment) -> CommandOutcome {
        let normalized = normalize_line_endings(&segment.text);
        if normalized.is_empty() {
            return CommandOutcome::NoOp(NoOpReason::EmptyInput);
        }
        self.ime = ImeComposition::Idle;

        let insert_start = self.cursor.start().as_usize();
        let needs_leading_space = segment.spacing == DictationSpacing::AutoSpace
            && self.locale.metadata().word_spacing == WordSpacingPolicy::SpaceDelimited
            && insert_start > 0
            && self.text[..insert_start]
                .chars()
                .next_back()
                .is_some_and(|ch| !ch.is_whitespace())
            && normalized
                .chars()
                .next()
                .is_some_and(|ch| !ch.is_whitespace() && !is_closing_punctuation(ch));

        let payload = if needs_leading_space {
            let mut spaced = String::with_capacity(normalized.len() + 1);
            spaced.push(' ');
            spaced.push_str(&normalized);
            spaced
        } else {
            normalized.into_owned()
        };

        self.push_undo_snapshot();
        self.replace_selection_inner(&payload);
        self.commit_mutation()
    }

    fn replace_selection_inner(&mut self, replacement: &str) {
        let start = self.cursor.start().as_usize();
        let end = self.cursor.end().as_usize();
        self.text.replace_range(start..end, replacement);
        let new_offset = start + replacement.len();
        self.cursor = CursorState::Caret(snap_to_cluster_boundary(&self.text, new_offset));
    }

    fn push_undo_snapshot(&mut self) {
        let snapshot = UndoSnapshot {
            text: self.text.clone(),
            cursor: self.cursor,
        };
        push_bounded(&mut self.undo_stack, snapshot, self.max_undo_depth);
        self.redo_stack.clear();
    }

    fn commit_mutation(&mut self) -> CommandOutcome {
        self.revision = self.revision.next();
        let layout = self.reclamp_scroll();
        CommandOutcome::DocumentChanged {
            revision: self.revision,
            caret_rect: layout.caret_rect,
        }
    }

    fn reclamp_scroll(&mut self) -> ComposerLayoutSnapshot {
        let mut scroll = self.viewport.scroll_offset_px;
        let snapshot = self.compute_layout_with_scroll(&mut scroll);
        self.viewport.scroll_offset_px = scroll;
        snapshot
    }

    fn refresh_preedit_anchor(&mut self, caret_rect: CaretRect) {
        if let ImeComposition::Composing {
            candidate_anchor, ..
        } = &mut self.ime
        {
            *candidate_anchor = caret_rect;
        }
    }

    fn compute_layout_with_scroll(&self, scroll_offset_px: &mut f32) -> ComposerLayoutSnapshot {
        let keyboard_inset = match self.soft_keyboard {
            SoftKeyboardState::Hidden => 0.0,
            SoftKeyboardState::Visible { inset_bottom_px } => inset_bottom_px,
        };
        let visible_viewport_height_px =
            (self.viewport.height_px - keyboard_inset).max(self.viewport.line_height_px * 2.0);
        let total_lines = self.text.bytes().filter(|&b| b == b'\n').count() + 1;
        let content_height_px = (total_lines as f32) * self.viewport.line_height_px
            + COMPOSER_VERTICAL_PADDING_PX * 2.0;

        let max_allowed_composer_height = self
            .viewport
            .composer_max_height_px
            .min(visible_viewport_height_px);
        let min_allowed_composer_height = self
            .viewport
            .composer_min_height_px
            .min(max_allowed_composer_height);
        let composer_height_px =
            content_height_px.clamp(min_allowed_composer_height, max_allowed_composer_height);
        let composer_top_px = (visible_viewport_height_px - composer_height_px).max(0.0);

        let lc = line_column_at(&self.text, self.cursor.focus());
        let caret_content_y =
            COMPOSER_VERTICAL_PADDING_PX + (lc.line as f32) * self.viewport.line_height_px;
        let inner_height = (composer_height_px - COMPOSER_VERTICAL_PADDING_PX * 2.0)
            .max(self.viewport.line_height_px);
        let max_scroll = (content_height_px - composer_height_px).max(0.0);

        let initial_scroll = *scroll_offset_px;
        let mut adjusted_scroll = initial_scroll.clamp(0.0, max_scroll);
        let caret_top_rel = caret_content_y - COMPOSER_VERTICAL_PADDING_PX;
        let caret_bottom_rel = caret_top_rel + self.viewport.line_height_px;

        if caret_top_rel < adjusted_scroll {
            adjusted_scroll = caret_top_rel;
        } else if caret_bottom_rel > adjusted_scroll + inner_height {
            adjusted_scroll = (caret_bottom_rel - inner_height).min(max_scroll);
        }

        let scroll_adjustment = if (adjusted_scroll - initial_scroll).abs() > f32::EPSILON {
            ScrollAdjustment::ClampedForCaret
        } else {
            ScrollAdjustment::Unchanged
        };
        *scroll_offset_px = adjusted_scroll;

        let caret_y = composer_top_px + (caret_content_y - adjusted_scroll);
        let horizontal_advance = (lc.column as f32) * self.viewport.char_advance_px;
        let caret_x = match self.effective_direction() {
            TextDirection::Ltr => (COMPOSER_HORIZONTAL_PADDING_PX + horizontal_advance)
                .min(self.viewport.width_px - COMPOSER_HORIZONTAL_PADDING_PX),
            TextDirection::Rtl => {
                (self.viewport.width_px - COMPOSER_HORIZONTAL_PADDING_PX - horizontal_advance)
                    .max(COMPOSER_HORIZONTAL_PADDING_PX)
            }
        };

        ComposerLayoutSnapshot {
            composer_top_px,
            composer_height_px,
            visible_viewport_height_px,
            caret_rect: CaretRect {
                x: caret_x,
                y: caret_y,
                width: CARET_WIDTH_PX,
                height: self.viewport.line_height_px,
            },
            scroll_offset_px: adjusted_scroll,
            total_lines,
            scroll_adjustment,
        }
    }
}

fn push_bounded<T>(queue: &mut VecDeque<T>, item: T, max_depth: usize) {
    if queue.len() >= max_depth {
        let _ = queue.pop_front();
    }
    queue.push_back(item);
}

fn normalize_line_endings(input: &str) -> Cow<'_, str> {
    if !input.contains('\r') {
        return Cow::Borrowed(input);
    }
    Cow::Owned(input.replace("\r\n", "\n").replace('\r', "\n"))
}

const fn is_closing_punctuation(ch: char) -> bool {
    matches!(
        ch,
        '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '،' | '۔' | '।'
    )
}
