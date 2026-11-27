//! Single owner for O(visible viewport rows) virtualization math.
//!
//! Both the 5,000-row clinical lab table and the 80-turn transcript delegate
//! visible window slicing to `VirtualizedWindow` so live scroll, resize, and
//! restored snapshots always compute identical ranges.

use std::ops::Range;

use crate::domain::StreamingPerfError;

/// Viewport geometry and overscan configuration for virtualized list/table rendering.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VirtualizedWindow {
    pub scroll_offset_px: f32,
    pub viewport_height_px: f32,
    pub row_height_px: f32,
    pub overscan_rows: usize,
}

impl VirtualizedWindow {
    /// Creates a validated virtualized window.
    pub fn new(
        scroll_offset_px: f32,
        viewport_height_px: f32,
        row_height_px: f32,
        overscan_rows: usize,
    ) -> Result<Self, StreamingPerfError> {
        if !viewport_height_px.is_finite()
            || !row_height_px.is_finite()
            || viewport_height_px <= 0.0
            || row_height_px <= 0.0
        {
            return Err(StreamingPerfError::InvalidViewportGeometry {
                viewport_height_px: viewport_height_px.max(0.0) as u32,
                row_height_px: row_height_px.max(0.0) as u32,
            });
        }
        let clamped_offset = if scroll_offset_px.is_finite() {
            scroll_offset_px.max(0.0)
        } else {
            0.0
        };
        Ok(Self {
            scroll_offset_px: clamped_offset,
            viewport_height_px,
            row_height_px,
            overscan_rows,
        })
    }

    /// Maximum number of materialized rows for the current viewport height and overscan.
    #[must_use]
    pub fn max_materialized_rows(&self) -> usize {
        let visible = (self.viewport_height_px / self.row_height_px).ceil() as usize;
        visible
            .saturating_add(self.overscan_rows.saturating_mul(2))
            .saturating_add(1)
    }

    /// Computes the exact range of rows to materialize for a collection of `total_rows`.
    ///
    /// Runs in strict O(1) time and returns at most `max_materialized_rows()`.
    #[must_use]
    pub fn visible_range(&self, total_rows: usize) -> Range<usize> {
        if total_rows == 0 || self.row_height_px <= 0.0 || self.viewport_height_px <= 0.0 {
            return 0..0;
        }
        let max_scroll = self.max_scroll_offset_px(total_rows);
        let clamped_scroll = self.scroll_offset_px.clamp(0.0, max_scroll);
        let first_visible = (clamped_scroll / self.row_height_px).floor() as usize;
        let visible_count = (self.viewport_height_px / self.row_height_px).ceil() as usize;

        let start = first_visible.saturating_sub(self.overscan_rows);
        let end = first_visible
            .saturating_add(visible_count)
            .saturating_add(self.overscan_rows)
            .saturating_add(1)
            .min(total_rows);
        start.min(total_rows)..end
    }

    /// Maximum valid scroll offset in logical pixels for `total_rows`.
    #[must_use]
    pub fn max_scroll_offset_px(&self, total_rows: usize) -> f32 {
        let total_height = (total_rows as f32) * self.row_height_px;
        (total_height - self.viewport_height_px).max(0.0)
    }

    /// Applies a scroll delta in logical pixels and clamps to valid content bounds.
    pub fn scroll_by(&mut self, delta_px: f32, total_rows: usize) {
        if !delta_px.is_finite() {
            return;
        }
        let max_scroll = self.max_scroll_offset_px(total_rows);
        self.scroll_offset_px = (self.scroll_offset_px + delta_px).clamp(0.0, max_scroll);
    }

    /// Resizes the viewport height and re-clamps scroll offset to valid bounds.
    pub fn set_viewport_height(
        &mut self,
        viewport_height_px: f32,
        total_rows: usize,
    ) -> Result<(), StreamingPerfError> {
        if !viewport_height_px.is_finite() || viewport_height_px <= 0.0 {
            return Err(StreamingPerfError::InvalidViewportGeometry {
                viewport_height_px: viewport_height_px.max(0.0) as u32,
                row_height_px: self.row_height_px.max(0.0) as u32,
            });
        }
        self.viewport_height_px = viewport_height_px;
        let max_scroll = self.max_scroll_offset_px(total_rows);
        self.scroll_offset_px = self.scroll_offset_px.clamp(0.0, max_scroll);
        Ok(())
    }
}
