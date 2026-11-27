use std::collections::{HashMap, HashSet};
use std::fmt;

use bevy::a11y::AccessibilityNode;
use bevy::prelude::{Component, Entity, Resource};
use serde::{Deserialize, Serialize};

use crate::i18n::{Locale, TextDirection};

/// Canonical subpath prefix enforced on all Elysian Fields web mounts.
pub const ELYSIAN_SUBPATH_PREFIX: &str = "/elysianfields";

/// Stable, serializable accessibility node identifier mapped 1:1 to `accesskit::NodeId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct A11yNodeId(pub u64);

impl A11yNodeId {
    #[must_use]
    pub fn from_entity(entity: Entity) -> Self {
        Self(entity.to_bits())
    }

    #[must_use]
    pub const fn to_accesskit(self) -> accesskit::NodeId {
        accesskit::NodeId(self.0)
    }

    #[must_use]
    pub const fn from_accesskit(id: accesskit::NodeId) -> Self {
        Self(id.0)
    }
}

/// Typed semantic identity for every landmark, control, dialog, or virtualized item.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum A11ySemanticKey {
    NavigationLandmark,
    MainLandmark,
    SearchLandmark,
    ComposerFormLandmark,
    CitationPanelRegion,
    PoliteLiveRegion,
    AssertiveAlertRegion,
    ComposerInput,
    SendButton,
    StopButton,
    ResumeButton,
    TryAgainButton,
    MobileMenuButton,
    PrivacyCoverDialog,
    PrivacyCoverDismissButton,
    PatientFinderDialog,
    PatientFinderInput,
    ShortcutsHelpDialog,
    ListRow(u32),
    TableRow(u32),
    CitationChip(u32),
}

/// Marker component attaching a Bevy UI entity to its unique semantic key and reading order.
#[derive(Debug, Clone, PartialEq, Eq, Component)]
pub struct A11yNodeMarker {
    pub key: A11ySemanticKey,
    pub reading_order: usize,
    pub interactivity: NodeInteractivity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeInteractivity {
    Structural,
    Interactive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeAvailability {
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeVisibility {
    Visible,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModalScopeKind {
    NonModal,
    ModalDialog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExpansionState {
    NotApplicable,
    Collapsed,
    Expanded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectionState {
    NotApplicable,
    Unselected,
    Selected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LivePoliteness {
    Polite,
    Assertive,
}

impl LivePoliteness {
    #[must_use]
    pub const fn to_accesskit(self) -> accesskit::Live {
        match self {
            Self::Polite => accesskit::Live::Polite,
            Self::Assertive => accesskit::Live::Assertive,
        }
    }
}

/// Exhaustive semantic ARIA role projected from `accesskit::Role`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AriaRole {
    Navigation,
    Main,
    Form,
    Region,
    Heading,
    List,
    ListItem,
    Link,
    Button,
    Textbox,
    Searchbox,
    Checkbox,
    Dialog,
    Alert,
    Status,
    Table,
    Row,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsupportedAriaRole {
    pub role: accesskit::Role,
}

impl fmt::Display for UnsupportedAriaRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "AccessKit role {:?} is unsupported in narrow ARIA bridge",
            self.role
        )
    }
}

impl std::error::Error for UnsupportedAriaRole {}

impl AriaRole {
    pub const fn from_accesskit(role: accesskit::Role) -> Result<Self, UnsupportedAriaRole> {
        match role {
            accesskit::Role::Navigation => Ok(Self::Navigation),
            accesskit::Role::Main => Ok(Self::Main),
            accesskit::Role::Form => Ok(Self::Form),
            accesskit::Role::Region => Ok(Self::Region),
            accesskit::Role::Heading => Ok(Self::Heading),
            accesskit::Role::List => Ok(Self::List),
            accesskit::Role::ListItem => Ok(Self::ListItem),
            accesskit::Role::Link => Ok(Self::Link),
            accesskit::Role::Button => Ok(Self::Button),
            accesskit::Role::TextInput | accesskit::Role::MultilineTextInput => Ok(Self::Textbox),
            accesskit::Role::SearchInput => Ok(Self::Searchbox),
            accesskit::Role::CheckBox => Ok(Self::Checkbox),
            accesskit::Role::Dialog => Ok(Self::Dialog),
            accesskit::Role::Alert => Ok(Self::Alert),
            accesskit::Role::Status => Ok(Self::Status),
            accesskit::Role::Table => Ok(Self::Table),
            accesskit::Role::Row => Ok(Self::Row),
            other => Err(UnsupportedAriaRole { role: other }),
        }
    }
}

/// Immutable accessibility node extracted directly from Bevy's `AccessibilityNode`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedA11yNode {
    pub id: A11yNodeId,
    pub key: A11ySemanticKey,
    pub role: AriaRole,
    pub label: String,
    pub value: Option<String>,
    pub reading_order: usize,
    pub interactivity: NodeInteractivity,
    pub availability: NodeAvailability,
    pub visibility: NodeVisibility,
    pub modal_scope: ModalScopeKind,
    pub expansion: ExpansionState,
    pub selection: SelectionState,
    pub live: Option<LivePoliteness>,
}

impl ProjectedA11yNode {
    pub fn from_bevy_node(
        entity: Entity,
        marker: &A11yNodeMarker,
        node: &AccessibilityNode,
    ) -> Result<Self, A11yProjectionError> {
        let role = AriaRole::from_accesskit(node.role()).map_err(|source| {
            A11yProjectionError::UnsupportedRole {
                key: marker.key.clone(),
                source,
            }
        })?;
        let availability = if node.is_disabled() {
            NodeAvailability::Disabled
        } else {
            NodeAvailability::Enabled
        };
        let visibility = if node.is_hidden() {
            NodeVisibility::Hidden
        } else {
            NodeVisibility::Visible
        };
        let modal_scope = if node.is_modal() {
            ModalScopeKind::ModalDialog
        } else {
            ModalScopeKind::NonModal
        };
        let expansion = match node.is_expanded() {
            Some(true) => ExpansionState::Expanded,
            Some(false) => ExpansionState::Collapsed,
            None => ExpansionState::NotApplicable,
        };
        let selection = match node.is_selected() {
            Some(true) => SelectionState::Selected,
            Some(false) => SelectionState::Unselected,
            None => SelectionState::NotApplicable,
        };
        let live = match node.live() {
            Some(accesskit::Live::Polite) => Some(LivePoliteness::Polite),
            Some(accesskit::Live::Assertive) => Some(LivePoliteness::Assertive),
            Some(accesskit::Live::Off) | None => None,
        };

        Ok(Self {
            id: A11yNodeId::from_entity(entity),
            key: marker.key.clone(),
            role,
            label: node.label().unwrap_or_default().to_owned(),
            value: node.value().map(ToOwned::to_owned),
            reading_order: marker.reading_order,
            interactivity: marker.interactivity,
            availability,
            visibility,
            modal_scope,
            expansion,
            selection,
            live,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum A11yProjectionError {
    UnsupportedRole {
        key: A11ySemanticKey,
        source: UnsupportedAriaRole,
    },
    DuplicateNodeId {
        id: A11yNodeId,
    },
    DuplicateSemanticKey {
        key: A11ySemanticKey,
        count: usize,
    },
    EmptyInteractiveLabel {
        key: A11ySemanticKey,
    },
    InvalidMountSubpath {
        subpath: String,
    },
    FocusNodeMissing {
        focus_id: A11yNodeId,
    },
}

impl fmt::Display for A11yProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedRole { key, source } => {
                write!(f, "node {key:?} cannot be projected: {source}")
            }
            Self::DuplicateNodeId { id } => write!(f, "duplicate accessibility NodeId {id:?}"),
            Self::DuplicateSemanticKey { key, count } => {
                write!(f, "duplicate semantic key {key:?} appeared {count} times")
            }
            Self::EmptyInteractiveLabel { key } => {
                write!(f, "interactive node {key:?} is missing an accessible label")
            }
            Self::InvalidMountSubpath { subpath } => {
                write!(
                    f,
                    "mount subpath '{subpath}' must start with '{ELYSIAN_SUBPATH_PREFIX}'"
                )
            }
            Self::FocusNodeMissing { focus_id } => {
                write!(
                    f,
                    "focused node {focus_id:?} is absent from visible projection"
                )
            }
        }
    }
}

impl std::error::Error for A11yProjectionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::UnsupportedRole { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Verified summary confirming single-owner zero-duplicate integrity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct A11yAuditSummary {
    pub total_nodes: usize,
    pub visible_nodes: usize,
    pub interactive_nodes: usize,
    pub duplicated_nodes: usize,
    pub mirrored_css_rules: usize,
}

/// Single-owner accessibility projection extracted from Bevy ECS.
#[derive(Debug, Clone, Default, Resource)]
pub struct A11yProjection {
    pub mount_subpath: String,
    pub locale: Locale,
    pub nodes: Vec<ProjectedA11yNode>,
    pub focused_node: Option<A11yNodeId>,
}

impl A11yProjection {
    #[must_use]
    pub fn from_nodes(
        mut nodes: Vec<ProjectedA11yNode>,
        focused_node: Option<A11yNodeId>,
        locale: Locale,
    ) -> Self {
        nodes.sort_by_key(|node| node.reading_order);
        Self {
            mount_subpath: ELYSIAN_SUBPATH_PREFIX.to_owned(),
            locale,
            nodes,
            focused_node,
        }
    }

    /// Verifies zero duplicate `A11yNodeId`s, zero duplicate semantic keys,
    /// non-empty interactive labels, valid `/elysianfields` subpath, and focus presence.
    pub fn verify_zero_duplicate_nodes(
        &self,
    ) -> Result<A11yAuditSummary, Vec<A11yProjectionError>> {
        let mut errors = Vec::new();
        if !self.mount_subpath.starts_with(ELYSIAN_SUBPATH_PREFIX) {
            errors.push(A11yProjectionError::InvalidMountSubpath {
                subpath: self.mount_subpath.clone(),
            });
        }

        let mut seen_ids = HashSet::new();
        let mut key_counts: HashMap<&A11ySemanticKey, usize> = HashMap::new();
        let mut visible_ids = HashSet::new();
        let mut interactive_nodes = 0;

        for node in &self.nodes {
            if !seen_ids.insert(node.id) {
                errors.push(A11yProjectionError::DuplicateNodeId { id: node.id });
            }
            *key_counts.entry(&node.key).or_insert(0) += 1;

            if matches!(node.visibility, NodeVisibility::Visible) {
                let _ = visible_ids.insert(node.id);
            }

            if matches!(node.interactivity, NodeInteractivity::Interactive) {
                interactive_nodes += 1;
                if node.label.trim().is_empty() {
                    errors.push(A11yProjectionError::EmptyInteractiveLabel {
                        key: node.key.clone(),
                    });
                }
            }
        }

        for (key, count) in key_counts {
            if count != 1 {
                errors.push(A11yProjectionError::DuplicateSemanticKey {
                    key: key.clone(),
                    count,
                });
            }
        }

        if let Some(focus_id) = self.focused_node
            && !visible_ids.contains(&focus_id)
        {
            errors.push(A11yProjectionError::FocusNodeMissing { focus_id });
        }

        if errors.is_empty() {
            Ok(A11yAuditSummary {
                total_nodes: self.nodes.len(),
                visible_nodes: visible_ids.len(),
                interactive_nodes,
                duplicated_nodes: 0,
                mirrored_css_rules: 0,
            })
        } else {
            Err(errors)
        }
    }
}

/// Differential typed JSON patch emitted to the narrow browser ARIA bridge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum BridgePatch {
    UpsertElement {
        element: ProjectedA11yNode,
    },
    RemoveElement {
        id: A11yNodeId,
    },
    SetFocus {
        id: Option<A11yNodeId>,
    },
    SetPoliteLive {
        text: Option<String>,
    },
    SetAssertiveLive {
        text: Option<String>,
    },
    SetDocumentLocale {
        bcp47: String,
        direction: TextDirection,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct EmitterState {
    visible_nodes: HashMap<A11yNodeId, ProjectedA11yNode>,
    focus: Option<A11yNodeId>,
    polite_live: Option<String>,
    assertive_live: Option<String>,
    locale: Option<Locale>,
}

/// Narrow differential ARIA patch emitter ensuring zero DOM mutations during idle
/// ticks or coalesced token buffers and strict 1:1 NodeId cardinality.
#[derive(Debug, Clone, Default, Resource)]
pub struct AriaBridgeEmitter {
    state: EmitterState,
    last_patches: Vec<BridgePatch>,
}

impl AriaBridgeEmitter {
    #[must_use]
    pub fn last_patches(&self) -> &[BridgePatch] {
        &self.last_patches
    }

    /// Computes minimal patches transitioning from previous visible state to `projection`.
    pub fn sync(
        &mut self,
        projection: &A11yProjection,
        live: &PoliteLiveQueue,
    ) -> Result<&[BridgePatch], Vec<A11yProjectionError>> {
        let _audit = projection.verify_zero_duplicate_nodes()?;
        let mut patches = Vec::new();

        let next_visible: HashMap<A11yNodeId, ProjectedA11yNode> = projection
            .nodes
            .iter()
            .filter(|node| matches!(node.visibility, NodeVisibility::Visible))
            .map(|node| (node.id, node.clone()))
            .collect();

        for &prev_id in self.state.visible_nodes.keys() {
            if !next_visible.contains_key(&prev_id) {
                patches.push(BridgePatch::RemoveElement { id: prev_id });
            }
        }

        for node in projection
            .nodes
            .iter()
            .filter(|node| matches!(node.visibility, NodeVisibility::Visible))
        {
            let unchanged = self
                .state
                .visible_nodes
                .get(&node.id)
                .is_some_and(|prev| prev == node);
            if !unchanged {
                patches.push(BridgePatch::UpsertElement {
                    element: node.clone(),
                });
            }
        }

        if self.state.locale != Some(projection.locale) {
            patches.push(BridgePatch::SetDocumentLocale {
                bcp47: projection.locale.bcp47_tag().to_owned(),
                direction: projection.locale.direction(),
            });
            self.state.locale = Some(projection.locale);
        }

        if self.state.focus != projection.focused_node {
            patches.push(BridgePatch::SetFocus {
                id: projection.focused_node,
            });
            self.state.focus = projection.focused_node;
        }

        let next_polite = live
            .latest_text(LivePoliteness::Polite)
            .map(ToOwned::to_owned);
        if self.state.polite_live != next_polite {
            patches.push(BridgePatch::SetPoliteLive {
                text: next_polite.clone(),
            });
            self.state.polite_live = next_polite;
        }

        let next_assertive = live
            .latest_text(LivePoliteness::Assertive)
            .map(ToOwned::to_owned);
        if self.state.assertive_live != next_assertive {
            patches.push(BridgePatch::SetAssertiveLive {
                text: next_assertive.clone(),
            });
            self.state.assertive_live = next_assertive;
        }

        self.state.visible_nodes = next_visible;
        self.last_patches = patches;
        Ok(&self.last_patches)
    }

    /// Serializes the latest patch batch as structured JSON for the narrow JS adapter.
    pub fn serialize_last_patches(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&self.last_patches)
    }
}

/// Validated coalescing policy preventing 40 tok/s streams from flooding screen readers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoalescingWindow {
    min_interval_ms: u64,
    max_interval_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoalescingWindowError {
    MinBelowFloor { requested_ms: u64, floor_ms: u64 },
    MaxAboveCeiling { requested_ms: u64, ceiling_ms: u64 },
    InvertedBounds { min_ms: u64, max_ms: u64 },
}

impl fmt::Display for CoalescingWindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MinBelowFloor {
                requested_ms,
                floor_ms,
            } => write!(f, "min_interval_ms {requested_ms}ms < floor {floor_ms}ms"),
            Self::MaxAboveCeiling {
                requested_ms,
                ceiling_ms,
            } => write!(
                f,
                "max_interval_ms {requested_ms}ms > ceiling {ceiling_ms}ms"
            ),
            Self::InvertedBounds { min_ms, max_ms } => {
                write!(f, "min_interval_ms {min_ms}ms > max_interval_ms {max_ms}ms")
            }
        }
    }
}

impl std::error::Error for CoalescingWindowError {}

impl CoalescingWindow {
    pub const FLOOR_MS: u64 = 750;
    pub const CEILING_MS: u64 = 1500;

    pub const fn new(
        min_interval_ms: u64,
        max_interval_ms: u64,
    ) -> Result<Self, CoalescingWindowError> {
        if min_interval_ms < Self::FLOOR_MS {
            return Err(CoalescingWindowError::MinBelowFloor {
                requested_ms: min_interval_ms,
                floor_ms: Self::FLOOR_MS,
            });
        }
        if max_interval_ms > Self::CEILING_MS {
            return Err(CoalescingWindowError::MaxAboveCeiling {
                requested_ms: max_interval_ms,
                ceiling_ms: Self::CEILING_MS,
            });
        }
        if min_interval_ms > max_interval_ms {
            return Err(CoalescingWindowError::InvertedBounds {
                min_ms: min_interval_ms,
                max_ms: max_interval_ms,
            });
        }
        Ok(Self {
            min_interval_ms,
            max_interval_ms,
        })
    }
}

impl Default for CoalescingWindow {
    fn default() -> Self {
        Self {
            min_interval_ms: 750,
            max_interval_ms: 1500,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveAnnouncement {
    pub sequence: u64,
    pub politeness: LivePoliteness,
    pub emitted_at_ms: u64,
    pub text: String,
    pub coalesced_chunks: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StreamBuffer {
    Empty,
    Buffering {
        started_at_ms: u64,
        text: String,
        chunks: usize,
    },
}

/// Throttled polite live-region queue with immediate assertive bypass.
#[derive(Debug, Clone, Resource)]
pub struct PoliteLiveQueue {
    window: CoalescingWindow,
    next_seq: u64,
    last_polite_emit_ms: Option<u64>,
    buffer: StreamBuffer,
    emitted: Vec<LiveAnnouncement>,
}

impl Default for PoliteLiveQueue {
    fn default() -> Self {
        Self::new(CoalescingWindow::default())
    }
}

impl PoliteLiveQueue {
    #[must_use]
    pub const fn new(window: CoalescingWindow) -> Self {
        Self {
            window,
            next_seq: 1,
            last_polite_emit_ms: None,
            buffer: StreamBuffer::Empty,
            emitted: Vec::new(),
        }
    }

    /// Buffers a streaming token/chunk and flushes only on a sentence boundary
    /// after `min_interval_ms` or when `max_interval_ms` elapses.
    pub fn push_stream_chunk(&mut self, chunk: &str, now_ms: u64) -> Option<&LiveAnnouncement> {
        if chunk.is_empty() {
            return None;
        }
        match &mut self.buffer {
            StreamBuffer::Empty => {
                self.buffer = StreamBuffer::Buffering {
                    started_at_ms: now_ms,
                    text: chunk.to_owned(),
                    chunks: 1,
                };
            }
            StreamBuffer::Buffering { text, chunks, .. } => {
                text.push_str(chunk);
                *chunks += 1;
            }
        }
        if self.should_flush(now_ms) {
            return self.flush_buffer(now_ms);
        }
        None
    }

    pub fn finish_stream(&mut self, now_ms: u64) -> Option<&LiveAnnouncement> {
        self.flush_buffer(now_ms)
    }

    pub fn announce_status(&mut self, text: impl Into<String>, now_ms: u64) -> &LiveAnnouncement {
        self.push_emitted(LivePoliteness::Polite, text.into(), now_ms, 1)
    }

    pub fn announce_assertive(
        &mut self,
        text: impl Into<String>,
        now_ms: u64,
    ) -> &LiveAnnouncement {
        self.push_emitted(LivePoliteness::Assertive, text.into(), now_ms, 1)
    }

    #[must_use]
    pub fn latest_text(&self, politeness: LivePoliteness) -> Option<&str> {
        self.emitted
            .iter()
            .rev()
            .find(|item| item.politeness == politeness)
            .map(|item| item.text.as_str())
    }

    #[must_use]
    pub fn count_in_window(&self, start_ms: u64, end_ms: u64, politeness: LivePoliteness) -> usize {
        self.emitted
            .iter()
            .filter(|item| {
                item.politeness == politeness
                    && item.emitted_at_ms >= start_ms
                    && item.emitted_at_ms <= end_ms
            })
            .count()
    }

    fn should_flush(&self, now_ms: u64) -> bool {
        let StreamBuffer::Buffering {
            started_at_ms,
            text,
            ..
        } = &self.buffer
        else {
            return false;
        };
        let elapsed_buffer = now_ms.saturating_sub(*started_at_ms);
        if elapsed_buffer >= self.window.max_interval_ms {
            return true;
        }
        let elapsed_since_last = self
            .last_polite_emit_ms
            .map_or(elapsed_buffer, |last| now_ms.saturating_sub(last));
        let trimmed = text.trim_end();
        let sentence_end = trimmed.ends_with('.')
            || trimmed.ends_with('!')
            || trimmed.ends_with('?')
            || text.ends_with('\n');
        sentence_end && elapsed_since_last >= self.window.min_interval_ms
    }

    fn flush_buffer(&mut self, now_ms: u64) -> Option<&LiveAnnouncement> {
        let prev = std::mem::replace(&mut self.buffer, StreamBuffer::Empty);
        let StreamBuffer::Buffering { text, chunks, .. } = prev else {
            return None;
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(self.push_emitted(LivePoliteness::Polite, trimmed.to_owned(), now_ms, chunks))
    }

    fn push_emitted(
        &mut self,
        politeness: LivePoliteness,
        text: String,
        now_ms: u64,
        coalesced_chunks: usize,
    ) -> &LiveAnnouncement {
        let sequence = self.next_seq;
        self.next_seq += 1;
        if matches!(politeness, LivePoliteness::Polite) {
            self.last_polite_emit_ms = Some(now_ms);
        }
        self.emitted.push(LiveAnnouncement {
            sequence,
            politeness,
            emitted_at_ms: now_ms,
            text,
            coalesced_chunks,
        });
        &self.emitted[self.emitted.len() - 1]
    }
}
