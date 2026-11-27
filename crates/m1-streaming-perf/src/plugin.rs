//! Bevy 0.20.1 headless-compatible plugin wiring the authoritative domain store
//! into virtualized ECS UI entity pools and reactive dirty-gated frame updates.
//!
//! Architectural invariant: Bevy ECS holds ONLY view-slot entities (`O(viewport)`)
//! derived from `StreamingPrototypeResource`; it never holds a second authoritative
//! store and never spawns `O(total_rows)` entities for the 5,000-row lab table or
//! 80-turn transcript.

use std::ops::Range;

use bevy::prelude::{
    App, Commands, Component, Display, Entity, FlexDirection, Node, Plugin, Query, ResMut,
    Resource, Text, Update, Val, With, Without,
};

use crate::domain::{BlockId, CitationId, CitationSource};
use crate::engine::{ActiveCitation, CitationPanelState, StreamingPrototypeStore};
use crate::redraw::RedrawDecision;
use crate::virtualization::VirtualizedWindow;
use crate::workload::SyntheticClinicalWorkload;

/// Bevy `Resource` wrapper holding the single authoritative prototype domain store.
#[derive(Resource)]
pub struct StreamingPrototypeResource {
    pub store: StreamingPrototypeStore,
}

/// ECS marker for a pooled virtualized transcript block row entity.
#[derive(Component, Debug, Clone, Copy)]
pub struct TranscriptSlot {
    pub slot_index: usize,
    pub bound_block: Option<(BlockId, u64)>,
}

/// ECS marker for a pooled virtualized 5,000-row lab table row entity.
#[derive(Component, Debug, Clone, Copy)]
pub struct LabTableRowSlot {
    pub slot_index: usize,
    pub bound_row_index: Option<usize>,
}

/// ECS marker for the non-modal citation side panel header/summary entity.
#[derive(Component, Debug, Clone, Copy)]
pub struct CitationPanelSummarySlot;

/// ECS marker for the single-owner multiline composer display entity.
#[derive(Component, Debug, Clone, Copy)]
pub struct ComposerTextSlot;

/// Tracks materialized ECS entity counts and reactive sync telemetry.
#[derive(Resource, Debug, Default, Clone)]
pub struct VirtualizedEcsPoolState {
    pub transcript_entities: Vec<Entity>,
    pub lab_table_entities: Vec<Entity>,
    pub citation_summary_entity: Option<Entity>,
    pub composer_entity: Option<Entity>,
    pub last_transcript_range: Range<usize>,
    pub last_lab_table_range: Range<usize>,
    pub last_citation_id: Option<CitationId>,
    pub last_composer_keystrokes: u64,
    pub transcript_slot_updates: u64,
    pub lab_row_slot_updates: u64,
    pub reactive_sync_runs: u64,
    pub idle_ticks_short_circuited: u64,
}

impl VirtualizedEcsPoolState {
    /// Total ECS entities currently materialized across transcript, lab table, citation header, and composer.
    #[must_use]
    pub fn total_materialized_entities(&self) -> usize {
        self.transcript_entities.len()
            + self.lab_table_entities.len()
            + usize::from(self.citation_summary_entity.is_some())
            + usize::from(self.composer_entity.is_some())
    }
}

/// Bevy 0.20.1 plugin implementing reactive zero-redraw sync and O(viewport) ECS virtualization.
#[derive(Debug, Clone, Copy)]
pub struct StreamingPerfPrototypePlugin {
    pub viewport_height_px: f32,
    pub transcript_row_height_px: f32,
    pub overscan_rows: usize,
}

impl Default for StreamingPerfPrototypePlugin {
    fn default() -> Self {
        Self {
            viewport_height_px: 720.0,
            transcript_row_height_px: 36.0,
            overscan_rows: 4,
        }
    }
}

impl Plugin for StreamingPerfPrototypePlugin {
    fn build(&self, app: &mut App) {
        let window = VirtualizedWindow::new(
            0.0,
            self.viewport_height_px,
            self.transcript_row_height_px,
            self.overscan_rows,
        )
        .unwrap_or(VirtualizedWindow {
            scroll_offset_px: 0.0,
            viewport_height_px: 720.0,
            row_height_px: 36.0,
            overscan_rows: 4,
        });

        let workload = SyntheticClinicalWorkload::generate();
        let store = StreamingPrototypeStore::from_workload(workload, window);

        app.insert_resource(StreamingPrototypeResource { store })
            .init_resource::<VirtualizedEcsPoolState>()
            .add_systems(Update, reactive_sync_ui_system);
    }
}

/// Reactive Bevy `Update` system.
///
/// Evaluates `IdleRedrawPolicy` first: when no visual state changed, it returns
/// immediately with zero ECS queries or text formatting (`0 frames/sec` idle).
/// When dirty, it updates ONLY the `O(viewport)` materialized entities for the
/// visible transcript slice, dirty streaming tail block, composer, and active
/// non-modal citation panel.
#[allow(clippy::type_complexity)]
fn reactive_sync_ui_system(
    mut commands: Commands,
    mut proto: ResMut<StreamingPrototypeResource>,
    mut pool: ResMut<VirtualizedEcsPoolState>,
    mut transcript_slots: Query<
        (&mut TranscriptSlot, &mut Text, &mut Node),
        (
            Without<LabTableRowSlot>,
            Without<CitationPanelSummarySlot>,
            Without<ComposerTextSlot>,
        ),
    >,
    mut lab_slots: Query<
        (&mut LabTableRowSlot, &mut Text, &mut Node),
        (
            Without<TranscriptSlot>,
            Without<CitationPanelSummarySlot>,
            Without<ComposerTextSlot>,
        ),
    >,
    mut citation_summary: Query<
        (&mut Text, &mut Node),
        (
            With<CitationPanelSummarySlot>,
            Without<TranscriptSlot>,
            Without<LabTableRowSlot>,
            Without<ComposerTextSlot>,
        ),
    >,
    mut composer_slot: Query<
        &mut Text,
        (
            With<ComposerTextSlot>,
            Without<TranscriptSlot>,
            Without<LabTableRowSlot>,
            Without<CitationPanelSummarySlot>,
        ),
    >,
) {
    let decision = proto.store.redraw_policy_mut().evaluate_tick();
    if matches!(decision, RedrawDecision::Idle) {
        pool.idle_ticks_short_circuited = pool.idle_ticks_short_circuited.saturating_add(1);
        return;
    }

    pool.reactive_sync_runs = pool.reactive_sync_runs.saturating_add(1);
    let dirty_blocks = proto.store.drain_dirty_blocks();

    // 1. Ensure singleton composer + citation summary entities exist.
    if pool.composer_entity.is_none() {
        let entity = commands
            .spawn((
                ComposerTextSlot,
                Node {
                    display: Display::Flex,
                    flex_direction: FlexDirection::Row,
                    height: Val::Px(64.0),
                    ..Node::default()
                },
                Text::new(proto.store.composer().text().to_owned()),
            ))
            .id();
        pool.composer_entity = Some(entity);
        pool.last_composer_keystrokes = proto.store.composer().keystrokes_applied();
    } else if pool.last_composer_keystrokes != proto.store.composer().keystrokes_applied() {
        if let Some(entity) = pool.composer_entity
            && let Ok(mut text) = composer_slot.get_mut(entity)
        {
            text.0.clear();
            text.0.push_str(proto.store.composer().text());
        }
        pool.last_composer_keystrokes = proto.store.composer().keystrokes_applied();
    }

    if pool.citation_summary_entity.is_none() {
        let entity = commands
            .spawn((
                CitationPanelSummarySlot,
                Node {
                    display: Display::None,
                    flex_direction: FlexDirection::Column,
                    ..Node::default()
                },
                Text::new(String::new()),
            ))
            .id();
        pool.citation_summary_entity = Some(entity);
    }

    // 2. Virtualized transcript pool: materialize strictly O(visible viewport blocks).
    let transcript_window = proto.store.transcript_window();
    let visible_blocks = transcript_window.visible_range(proto.store.blocks().len());
    let required_transcript_slots = visible_blocks.len();
    let max_transcript_slots = transcript_window.max_materialized_rows();

    while pool.transcript_entities.len() < required_transcript_slots.min(max_transcript_slots) {
        let slot_index = pool.transcript_entities.len();
        let entity = commands
            .spawn((
                TranscriptSlot {
                    slot_index,
                    bound_block: None,
                },
                Node {
                    display: Display::Flex,
                    height: Val::Px(transcript_window.row_height_px),
                    ..Node::default()
                },
                Text::new(String::with_capacity(256)),
            ))
            .id();
        pool.transcript_entities.push(entity);
    }

    let range_changed = pool.last_transcript_range != visible_blocks;
    if range_changed || !dirty_blocks.is_empty() {
        let mut updates: u64 = 0;
        for (slot_idx, entity) in pool.transcript_entities.iter().copied().enumerate() {
            let Ok((mut slot, mut text, mut node)) = transcript_slots.get_mut(entity) else {
                continue;
            };
            let target_idx = visible_blocks.start + slot_idx;
            if target_idx < visible_blocks.end
                && let Some(block) = proto.store.blocks().get(target_idx)
            {
                let binding = (block.id, block.revision);
                if slot.bound_block != Some(binding) {
                    block.content.write_preview_into(&mut text.0);
                    node.display = Display::Flex;
                    node.height = Val::Px(block.content.estimated_height_px());
                    slot.bound_block = Some(binding);
                    updates = updates.saturating_add(1);
                }
            } else if slot.bound_block.is_some() {
                node.display = Display::None;
                slot.bound_block = None;
            }
        }
        pool.transcript_slot_updates = pool.transcript_slot_updates.saturating_add(updates);
        pool.last_transcript_range = visible_blocks;
    }

    // 3. Non-modal citation side panel & O(viewport) 5,000-row virtualized lab table pool.
    match proto.store.citation_panel() {
        CitationPanelState::Closed => {
            if let Some(summary_entity) = pool.citation_summary_entity
                && let Ok((_, mut node)) = citation_summary.get_mut(summary_entity)
            {
                node.display = Display::None;
            }
            for entity in pool.lab_table_entities.iter().copied() {
                if let Ok((mut slot, _, mut node)) = lab_slots.get_mut(entity)
                    && slot.bound_row_index.is_some()
                {
                    node.display = Display::None;
                    slot.bound_row_index = None;
                }
            }
            pool.last_citation_id = None;
            pool.last_lab_table_range = 0..0;
        }
        CitationPanelState::Open(ActiveCitation::ProgressNote(citation_id)) => {
            if pool.last_citation_id != Some(*citation_id) {
                if let Some(summary_entity) = pool.citation_summary_entity
                    && let Ok((mut text, mut node)) = citation_summary.get_mut(summary_entity)
                    && let Some(CitationSource::ProgressNote(note)) =
                        proto.store.citation_source(*citation_id)
                {
                    text.0.clear();
                    text.0.push_str(&note.title);
                    text.0.push('\n');
                    text.0.push_str(&note.encounter_label);
                    for section in &note.sections {
                        text.0.push_str("\n\n");
                        text.0.push_str(&section.heading);
                        text.0.push_str(": ");
                        text.0.push_str(&section.body);
                    }
                    node.display = Display::Flex;
                }
                for entity in pool.lab_table_entities.iter().copied() {
                    if let Ok((mut slot, _, mut node)) = lab_slots.get_mut(entity)
                        && slot.bound_row_index.is_some()
                    {
                        node.display = Display::None;
                        slot.bound_row_index = None;
                    }
                }
                pool.last_citation_id = Some(*citation_id);
                pool.last_lab_table_range = 0..0;
            }
        }
        CitationPanelState::Open(ActiveCitation::VirtualizedLabTable { citation, window }) => {
            let citation_id = *citation;
            let lab_window = *window;
            if pool.last_citation_id != Some(citation_id)
                && let Some(summary_entity) = pool.citation_summary_entity
                && let Ok((mut text, mut node)) = citation_summary.get_mut(summary_entity)
                && let Some(CitationSource::VirtualizedLabTable {
                    title, row_count, ..
                }) = proto.store.citation_source(citation_id)
            {
                text.0.clear();
                text.0.push_str(title);
                text.0.push_str(" (");
                text.0.push_str(&row_count.to_string());
                text.0.push_str(" rows virtualized)");
                node.display = Display::Flex;
            }

            let visible_rows = lab_window.visible_range(proto.store.lab_rows().len());
            let required_lab_slots = visible_rows.len().min(lab_window.max_materialized_rows());

            while pool.lab_table_entities.len() < required_lab_slots {
                let slot_index = pool.lab_table_entities.len();
                let entity = commands
                    .spawn((
                        LabTableRowSlot {
                            slot_index,
                            bound_row_index: None,
                        },
                        Node {
                            display: Display::Flex,
                            height: Val::Px(lab_window.row_height_px),
                            ..Node::default()
                        },
                        Text::new(String::with_capacity(96)),
                    ))
                    .id();
                pool.lab_table_entities.push(entity);
            }

            if pool.last_lab_table_range != visible_rows
                || pool.last_citation_id != Some(citation_id)
            {
                let mut lab_updates: u64 = 0;
                for (slot_idx, entity) in pool.lab_table_entities.iter().copied().enumerate() {
                    let Ok((mut slot, mut text, mut node)) = lab_slots.get_mut(entity) else {
                        continue;
                    };
                    let row_idx = visible_rows.start + slot_idx;
                    if row_idx < visible_rows.end
                        && let Some(row) = proto.store.lab_rows().get(row_idx)
                    {
                        if slot.bound_row_index != Some(row_idx) {
                            row.write_row_text(&mut text.0);
                            node.display = Display::Flex;
                            node.height = Val::Px(lab_window.row_height_px);
                            slot.bound_row_index = Some(row_idx);
                            lab_updates = lab_updates.saturating_add(1);
                        }
                    } else if slot.bound_row_index.is_some() {
                        node.display = Display::None;
                        slot.bound_row_index = None;
                    }
                }
                pool.lab_row_slot_updates = pool.lab_row_slot_updates.saturating_add(lab_updates);
                pool.last_lab_table_range = visible_rows;
            }
            pool.last_citation_id = Some(citation_id);
        }
    }
}
