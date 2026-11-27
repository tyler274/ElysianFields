//! Bevy 0.20.1 plugin wiring the single-owner composer and incremental streaming transcript.
//!
//! Keeps authoritative domain state in [`ComposerState`] and [`TranscriptFeedState`]
//! independent of ECS view entities, syncing view nodes incrementally and producing
//! zero view mutations during idle frames.

use bevy::prelude::*;

use crate::adapter::AdapterReadback;
use crate::composer::{
    ClipboardMutation, CommandOutcome, ComposerDocument, DocumentRevision, TextInputCommand,
};
use crate::locale::Locale;

/// Bevy plugin registering multiline composer resources, messages, and incremental layout systems.
#[derive(Debug, Clone, Copy)]
pub struct TextI18nPrototypePlugin {
    pub initial_locale: Locale,
}

impl Default for TextI18nPrototypePlugin {
    fn default() -> Self {
        Self {
            initial_locale: Locale::En,
        }
    }
}

impl Plugin for TextI18nPrototypePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ComposerState::new(self.initial_locale))
            .init_resource::<TranscriptFeedState>()
            .init_resource::<AdapterReadbackState>()
            .init_resource::<ComposerRuntimeMetrics>()
            .add_message::<TextCommandMessage>()
            .add_message::<TranscriptChunkMessage>()
            .add_message::<ComposerOutcomeMessage>()
            .add_systems(Startup, spawn_prototype_view_nodes)
            .add_systems(
                Update,
                (
                    process_text_commands_system,
                    apply_streaming_transcript_system,
                    sync_composer_view_system,
                    sync_transcript_view_system,
                    record_idle_frame_system,
                )
                    .chain(),
            );
    }
}

/// Resource owning the single authoritative [`ComposerDocument`].
#[derive(Resource, Debug)]
pub struct ComposerState {
    document: ComposerDocument,
    view_sync_generation: u64,
    last_synced_generation: u64,
    last_clipboard_payload: Option<String>,
}

impl ComposerState {
    #[must_use]
    pub fn new(locale: Locale) -> Self {
        Self {
            document: ComposerDocument::new(locale),
            view_sync_generation: 1,
            last_synced_generation: 0,
            last_clipboard_payload: None,
        }
    }

    #[must_use]
    pub const fn document(&self) -> &ComposerDocument {
        &self.document
    }

    #[must_use]
    pub fn document_mut(&mut self) -> &mut ComposerDocument {
        self.view_sync_generation = self.view_sync_generation.saturating_add(1);
        &mut self.document
    }

    #[must_use]
    pub fn last_clipboard_payload(&self) -> Option<&str> {
        self.last_clipboard_payload.as_deref()
    }

    fn apply_command(&mut self, command: TextInputCommand) -> CommandOutcome {
        let outcome = self.document.apply(command);
        match &outcome {
            CommandOutcome::DocumentChanged { .. }
            | CommandOutcome::SelectionChanged { .. }
            | CommandOutcome::PreeditUpdated { .. }
            | CommandOutcome::FocusChanged { .. }
            | CommandOutcome::ViewportAdjusted { .. } => {
                self.view_sync_generation = self.view_sync_generation.saturating_add(1);
            }
            CommandOutcome::ClipboardProduced { text, mutation } => {
                self.last_clipboard_payload = Some(text.clone());
                if matches!(mutation, ClipboardMutation::CutApplied { .. }) {
                    self.view_sync_generation = self.view_sync_generation.saturating_add(1);
                }
            }
            CommandOutcome::NoOp(_) => {}
        }
        outcome
    }

    #[must_use]
    const fn needs_view_sync(&self) -> bool {
        self.view_sync_generation != self.last_synced_generation
    }

    const fn mark_view_synced(&mut self) {
        self.last_synced_generation = self.view_sync_generation;
    }
}

/// Speaker role in the synthetic clinical transcript above the composer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptRole {
    Clinician,
    Assistant,
}

/// One turn inside the read-only streaming transcript above the composer.
#[derive(Debug, Clone)]
pub struct TranscriptTurn {
    pub index: usize,
    pub role: TranscriptRole,
    pub content: String,
    pub revision: u64,
}

/// Resource owning the streaming transcript above the composer with O(visible) viewport windowing.
#[derive(Resource, Debug)]
pub struct TranscriptFeedState {
    turns: Vec<TranscriptTurn>,
    visible_window_start: usize,
    visible_window_len: usize,
    total_chars: usize,
    dirty_turn_index: Option<usize>,
    revision: u64,
}

impl Default for TranscriptFeedState {
    fn default() -> Self {
        Self {
            turns: Vec::new(),
            visible_window_start: 0,
            visible_window_len: 8,
            total_chars: 0,
            dirty_turn_index: None,
            revision: 0,
        }
    }
}

impl TranscriptFeedState {
    /// Populate the transcript with synthetic turns and set the visible viewport window.
    pub fn seed_turns(&mut self, turns: Vec<TranscriptTurn>, visible_window_len: usize) {
        self.total_chars = turns.iter().map(|turn| turn.content.len()).sum();
        self.visible_window_len = visible_window_len.max(1);
        self.visible_window_start = turns.len().saturating_sub(self.visible_window_len);
        self.turns = turns;
        self.revision = self.revision.saturating_add(1);
        self.dirty_turn_index = self.turns.len().checked_sub(1);
    }

    /// Apply an incremental token chunk to one turn in O(delta_len) time without touching
    /// any prior turn.
    pub fn append_delta(&mut self, turn_index: usize, delta: &str) {
        if delta.is_empty() {
            return;
        }
        if turn_index >= self.turns.len() {
            self.turns.push(TranscriptTurn {
                index: self.turns.len(),
                role: TranscriptRole::Assistant,
                content: String::new(),
                revision: 0,
            });
        }
        let target_idx = turn_index.min(self.turns.len() - 1);
        let turn = &mut self.turns[target_idx];
        turn.content.push_str(delta);
        turn.revision = turn.revision.saturating_add(1);
        self.total_chars = self.total_chars.saturating_add(delta.len());
        self.revision = self.revision.saturating_add(1);
        self.dirty_turn_index = Some(target_idx);
        if target_idx >= self.visible_window_start + self.visible_window_len {
            self.visible_window_start = target_idx
                .saturating_add(1)
                .saturating_sub(self.visible_window_len);
        }
    }

    #[must_use]
    pub fn turn_count(&self) -> usize {
        self.turns.len()
    }

    #[must_use]
    pub const fn total_chars(&self) -> usize {
        self.total_chars
    }

    #[must_use]
    pub fn active_turn_text(&self, index: usize) -> Option<&str> {
        self.turns.get(index).map(|turn| turn.content.as_str())
    }

    #[must_use]
    pub fn visible_turns(&self) -> &[TranscriptTurn] {
        let start = self.visible_window_start.min(self.turns.len());
        let end = (start + self.visible_window_len).min(self.turns.len());
        &self.turns[start..end]
    }
}

/// Resource storing the latest read-only [`AdapterReadback`] projected from [`ComposerState`].
#[derive(Resource, Debug, Default)]
pub struct AdapterReadbackState {
    pub latest: Option<AdapterReadback>,
}

/// Runtime instrumentation counters verifying incremental sync and zero-redraw idle behavior.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ComposerRuntimeMetrics {
    pub commands_processed: u64,
    pub streaming_chunks_applied: u64,
    pub composer_view_syncs: u64,
    pub transcript_view_syncs: u64,
    pub idle_frames: u64,
    pub last_synced_document_revision: DocumentRevision,
}

/// ECS message delivering a typed text input command from a platform adapter.
#[derive(Message, Debug, Clone)]
pub struct TextCommandMessage(pub TextInputCommand);

/// ECS message delivering an incremental streaming token delta for the transcript above the composer.
#[derive(Message, Debug, Clone)]
pub struct TranscriptChunkMessage {
    pub turn_index: usize,
    pub token_delta: String,
}

/// ECS message publishing the typed outcome of each applied composer command.
#[derive(Message, Debug, Clone)]
pub struct ComposerOutcomeMessage(pub CommandOutcome);

/// Marker component for the read-only Bevy UI projection of the multiline composer.
#[derive(Component, Debug)]
pub struct ComposerViewNode {
    pub rendered_lines: usize,
}

/// Marker component for the read-only Bevy UI projection of the active streaming transcript turn.
#[derive(Component, Debug)]
pub struct ActiveTranscriptTurnViewNode {
    pub turn_index: usize,
    pub synced_revision: u64,
}

fn spawn_prototype_view_nodes(mut commands: Commands) {
    commands.spawn((
        ComposerViewNode { rendered_lines: 1 },
        Node {
            width: Val::Percent(100.0),
            min_height: Val::Px(56.0),
            max_height: Val::Px(176.0),
            ..default()
        },
        Text::new(""),
    ));

    commands.spawn((
        ActiveTranscriptTurnViewNode {
            turn_index: 0,
            synced_revision: 0,
        },
        Node {
            width: Val::Percent(100.0),
            ..default()
        },
        Text::new(""),
    ));
}

fn process_text_commands_system(
    mut incoming: MessageReader<TextCommandMessage>,
    mut composer: ResMut<ComposerState>,
    mut readback: ResMut<AdapterReadbackState>,
    mut outcomes: MessageWriter<ComposerOutcomeMessage>,
    mut metrics: ResMut<ComposerRuntimeMetrics>,
) {
    let mut processed_any = false;
    for msg in incoming.read() {
        let outcome = composer.apply_command(msg.0.clone());
        outcomes.write(ComposerOutcomeMessage(outcome));
        metrics.commands_processed = metrics.commands_processed.saturating_add(1);
        processed_any = true;
    }

    if processed_any || readback.latest.is_none() {
        readback.latest = Some(AdapterReadback::from_document(composer.document()));
    }
}

fn apply_streaming_transcript_system(
    mut chunks: MessageReader<TranscriptChunkMessage>,
    mut feed: ResMut<TranscriptFeedState>,
    mut metrics: ResMut<ComposerRuntimeMetrics>,
) {
    for chunk in chunks.read() {
        feed.append_delta(chunk.turn_index, &chunk.token_delta);
        metrics.streaming_chunks_applied = metrics.streaming_chunks_applied.saturating_add(1);
    }
}

fn sync_composer_view_system(
    mut composer: ResMut<ComposerState>,
    mut metrics: ResMut<ComposerRuntimeMetrics>,
    mut query: Query<(&mut ComposerViewNode, &mut Text, &mut Node)>,
) {
    if !composer.needs_view_sync() {
        return;
    }

    let snapshot = composer.document().layout_snapshot();
    let display = composer.document().display_text();

    for (mut view, mut text, mut node) in &mut query {
        text.0.clear();
        text.0.push_str(&display);
        node.height = Val::Px(snapshot.composer_height_px);
        view.rendered_lines = snapshot.total_lines;
    }

    metrics.composer_view_syncs = metrics.composer_view_syncs.saturating_add(1);
    metrics.last_synced_document_revision = composer.document().revision();
    composer.mark_view_synced();
}

fn sync_transcript_view_system(
    mut feed: ResMut<TranscriptFeedState>,
    mut metrics: ResMut<ComposerRuntimeMetrics>,
    mut query: Query<(&mut ActiveTranscriptTurnViewNode, &mut Text)>,
) {
    let Some(dirty_idx) = feed.dirty_turn_index.take() else {
        return;
    };

    let Some(turn) = feed.turns.get(dirty_idx) else {
        return;
    };

    for (mut view, mut text) in &mut query {
        // Incremental projection: only mirror the single mutated active turn, never all 80 turns.
        if view.turn_index != turn.index {
            text.0.clear();
            text.0.push_str(&turn.content);
            view.turn_index = turn.index;
        } else {
            let current_len = text.0.len();
            if turn.content.len() >= current_len
                && turn.content.is_char_boundary(current_len)
                && turn.content.starts_with(&text.0)
            {
                text.0.push_str(&turn.content[current_len..]);
            } else {
                text.0.clear();
                text.0.push_str(&turn.content);
            }
        }
        view.synced_revision = turn.revision;
    }

    metrics.transcript_view_syncs = metrics.transcript_view_syncs.saturating_add(1);
}

fn record_idle_frame_system(
    composer: Res<ComposerState>,
    feed: Res<TranscriptFeedState>,
    mut metrics: ResMut<ComposerRuntimeMetrics>,
    mut prev_counts: Local<(u64, u64)>,
) {
    let current = (metrics.composer_view_syncs, metrics.transcript_view_syncs);
    if current == *prev_counts && !composer.needs_view_sync() && feed.dirty_turn_index.is_none() {
        metrics.idle_frames = metrics.idle_frames.saturating_add(1);
    }
    *prev_counts = current;
}
