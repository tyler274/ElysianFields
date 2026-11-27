use serde::{Deserialize, Serialize};

use crate::i18n::UiKey;

/// Computed O(viewport rows) slice window with top/bottom scroll spacers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisibleRange {
    pub start_row: usize,
    pub end_row: usize,
    pub top_spacer_px: f32,
    pub bottom_spacer_px: f32,
    pub total_content_height_px: f32,
}

impl VisibleRange {
    #[must_use]
    pub const fn materialized_count(self) -> usize {
        self.end_row.saturating_sub(self.start_row)
    }
}

/// Single-owner vertical virtualization window calculation shared by
/// [`VirtualizedListView`] and [`VirtualizedTableView`].
#[must_use]
pub fn compute_visible_range(
    total_rows: usize,
    scroll_offset_px: f32,
    viewport_height_px: f32,
    row_height_px: f32,
    overscan_rows: usize,
) -> VisibleRange {
    let safe_row_h = row_height_px.max(1.0);
    let safe_viewport_h = viewport_height_px.max(safe_row_h);
    let total_content_height_px = total_rows as f32 * safe_row_h;

    if total_rows == 0 {
        return VisibleRange {
            start_row: 0,
            end_row: 0,
            top_spacer_px: 0.0,
            bottom_spacer_px: 0.0,
            total_content_height_px: 0.0,
        };
    }

    let max_scroll = (total_content_height_px - safe_viewport_h).max(0.0);
    let clamped_scroll = scroll_offset_px.clamp(0.0, max_scroll);

    let first_visible = (clamped_scroll / safe_row_h).floor() as usize;
    let visible_count = (safe_viewport_h / safe_row_h).ceil() as usize + 1;

    let start_row = first_visible.saturating_sub(overscan_rows).min(total_rows);
    let end_row = first_visible
        .saturating_add(visible_count)
        .saturating_add(overscan_rows)
        .min(total_rows);

    let top_spacer_px = start_row as f32 * safe_row_h;
    let bottom_spacer_px = (total_rows.saturating_sub(end_row)) as f32 * safe_row_h;

    VisibleRange {
        start_row,
        end_row,
        top_spacer_px,
        bottom_spacer_px,
        total_content_height_px,
    }
}

/// O(viewport rows) virtualized vertical list for patient rosters, message
/// threads, and 80+ turn transcripts.
#[derive(Debug, Clone, PartialEq)]
pub struct VirtualizedListView {
    total_rows: usize,
    scroll_offset_px: f32,
    viewport_height_px: f32,
    row_height_px: f32,
    overscan_rows: usize,
    focused_index: Option<usize>,
    selected_index: Option<usize>,
}

impl VirtualizedListView {
    #[must_use]
    pub fn new(
        total_rows: usize,
        viewport_height_px: f32,
        row_height_px: f32,
        overscan_rows: usize,
    ) -> Self {
        Self {
            total_rows,
            scroll_offset_px: 0.0,
            viewport_height_px: viewport_height_px.max(1.0),
            row_height_px: row_height_px.max(1.0),
            overscan_rows,
            focused_index: (total_rows > 0).then_some(0),
            selected_index: None,
        }
    }

    #[must_use]
    pub fn visible_range(&self) -> VisibleRange {
        compute_visible_range(
            self.total_rows,
            self.scroll_offset_px,
            self.viewport_height_px,
            self.row_height_px,
            self.overscan_rows,
        )
    }

    pub fn set_total_rows(&mut self, total_rows: usize) {
        self.total_rows = total_rows;
        self.clamp_scroll();
        if total_rows == 0 {
            self.focused_index = None;
            self.selected_index = None;
        } else {
            self.focused_index = self.focused_index.map(|idx| idx.min(total_rows - 1));
            self.selected_index = self.selected_index.map(|idx| idx.min(total_rows - 1));
        }
    }

    pub fn set_viewport_height(&mut self, viewport_height_px: f32) {
        self.viewport_height_px = viewport_height_px.max(1.0);
        self.clamp_scroll();
    }

    pub fn scroll_by(&mut self, delta_px: f32) {
        self.scroll_offset_px += delta_px;
        self.clamp_scroll();
    }

    pub fn scroll_to(&mut self, offset_px: f32) {
        self.scroll_offset_px = offset_px;
        self.clamp_scroll();
    }

    /// Moves focus to the next row (`j` / `ArrowDown`) and scrolls minimally to keep it visible.
    pub fn move_focus_next(&mut self) -> Option<usize> {
        if self.total_rows == 0 {
            return None;
        }
        let next = match self.focused_index {
            Some(idx) => (idx + 1).min(self.total_rows - 1),
            None => 0,
        };
        self.focused_index = Some(next);
        self.ensure_row_visible(next);
        Some(next)
    }

    /// Moves focus to the previous row (`k` / `ArrowUp`) and scrolls minimally to keep it visible.
    pub fn move_focus_prev(&mut self) -> Option<usize> {
        if self.total_rows == 0 {
            return None;
        }
        let prev = match self.focused_index {
            Some(idx) => idx.saturating_sub(1),
            None => 0,
        };
        self.focused_index = Some(prev);
        self.ensure_row_visible(prev);
        Some(prev)
    }

    pub fn select_focused(&mut self) -> Option<usize> {
        self.selected_index = self.focused_index;
        self.selected_index
    }

    pub fn ensure_row_visible(&mut self, row_index: usize) {
        if row_index >= self.total_rows {
            return;
        }
        let row_top = row_index as f32 * self.row_height_px;
        let row_bottom = row_top + self.row_height_px;
        if row_top < self.scroll_offset_px {
            self.scroll_offset_px = row_top;
        } else if row_bottom > self.scroll_offset_px + self.viewport_height_px {
            self.scroll_offset_px = row_bottom - self.viewport_height_px;
        }
        self.clamp_scroll();
    }

    #[must_use]
    pub const fn scroll_offset_px(&self) -> f32 {
        self.scroll_offset_px
    }

    #[must_use]
    pub const fn focused_index(&self) -> Option<usize> {
        self.focused_index
    }

    #[must_use]
    pub const fn selected_index(&self) -> Option<usize> {
        self.selected_index
    }

    #[must_use]
    pub fn slice_visible<'a, T>(&self, items: &'a [T]) -> (VisibleRange, &'a [T]) {
        let range = compute_visible_range(
            items.len(),
            self.scroll_offset_px,
            self.viewport_height_px,
            self.row_height_px,
            self.overscan_rows,
        );
        (range, &items[range.start_row..range.end_row])
    }

    fn clamp_scroll(&mut self) {
        let max_scroll =
            (self.total_rows as f32 * self.row_height_px - self.viewport_height_px).max(0.0);
        self.scroll_offset_px = self.scroll_offset_px.clamp(0.0, max_scroll);
    }
}

/// Column alignment in [`VirtualizedTableView`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColumnAlignment {
    Leading,
    Trailing,
}

/// Typed column schema for clinical lab and structured record tables.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableColumn {
    pub id: String,
    pub header_key: UiKey,
    pub width_px: f32,
    pub alignment: ColumnAlignment,
}

/// O(viewport rows) virtualized table built for 5,000+ row clinical lab tables
/// inside non-modal cited-source reading panels.
#[derive(Debug, Clone, PartialEq)]
pub struct VirtualizedTableView {
    columns: Vec<TableColumn>,
    list: VirtualizedListView,
    horizontal_scroll_px: f32,
}

impl VirtualizedTableView {
    #[must_use]
    pub fn new(
        columns: Vec<TableColumn>,
        total_rows: usize,
        viewport_height_px: f32,
        row_height_px: f32,
        overscan_rows: usize,
    ) -> Self {
        Self {
            columns,
            list: VirtualizedListView::new(
                total_rows,
                viewport_height_px,
                row_height_px,
                overscan_rows,
            ),
            horizontal_scroll_px: 0.0,
        }
    }

    #[must_use]
    pub fn columns(&self) -> &[TableColumn] {
        &self.columns
    }

    #[must_use]
    pub fn total_width_px(&self) -> f32 {
        self.columns.iter().map(|c| c.width_px.max(0.0)).sum()
    }

    #[must_use]
    pub fn visible_range(&self) -> VisibleRange {
        self.list.visible_range()
    }

    pub fn scroll_vertical_by(&mut self, delta_px: f32) {
        self.list.scroll_by(delta_px);
    }

    pub fn scroll_vertical_to(&mut self, offset_px: f32) {
        self.list.scroll_to(offset_px);
    }

    pub fn scroll_horizontal_by(&mut self, delta_px: f32, viewport_width_px: f32) {
        let max_x = (self.total_width_px() - viewport_width_px.max(1.0)).max(0.0);
        self.horizontal_scroll_px = (self.horizontal_scroll_px + delta_px).clamp(0.0, max_x);
    }

    pub fn move_focus_next(&mut self) -> Option<usize> {
        self.list.move_focus_next()
    }

    pub fn move_focus_prev(&mut self) -> Option<usize> {
        self.list.move_focus_prev()
    }

    pub fn set_total_rows(&mut self, total_rows: usize) {
        self.list.set_total_rows(total_rows);
    }

    #[must_use]
    pub const fn focused_row(&self) -> Option<usize> {
        self.list.focused_index()
    }

    #[must_use]
    pub const fn vertical_scroll_px(&self) -> f32 {
        self.list.scroll_offset_px()
    }

    #[must_use]
    pub const fn horizontal_scroll_px(&self) -> f32 {
        self.horizontal_scroll_px
    }

    #[must_use]
    pub fn visible_rows_slice<'a, T>(&self, all_rows: &'a [T]) -> (VisibleRange, &'a [T]) {
        self.list.slice_visible(all_rows)
    }
}
