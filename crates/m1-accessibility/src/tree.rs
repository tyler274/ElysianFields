use std::collections::{HashMap, HashSet};
use std::fmt;

use bevy::a11y::AccessibilityNode;
use bevy::prelude::{Component, Entity, Resource};

use crate::domain::{ModalState, SemanticControlId};

/// Maps a Bevy `Entity` deterministically to its single `accesskit::NodeId`.
#[must_use]
pub fn node_id_for_entity(entity: Entity) -> accesskit::NodeId {
    accesskit::NodeId(entity.to_bits())
}

/// Marker attaching each Bevy UI entity to its single-owner [`SemanticControlId`]
/// and deterministic reading order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Component)]
pub struct SemanticControlMarker {
    pub control: SemanticControlId,
    pub reading_order: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeInteractivity {
    Structural,
    Interactive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeAvailability {
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeVisibility {
    Visible,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalRole {
    NonModal,
    ModalDialog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpansionState {
    NotApplicable,
    Collapsed,
    Expanded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionState {
    NotApplicable,
    Unselected,
    Selected,
}

/// Immutable snapshot of a single node extracted directly from Bevy's
/// `bevy::a11y::AccessibilityNode` (`accesskit::Node`).
#[derive(Debug, Clone, PartialEq)]
pub struct AccessibilityNodeRecord {
    pub node_id: accesskit::NodeId,
    pub control: SemanticControlId,
    pub role: accesskit::Role,
    pub name: String,
    pub value: Option<String>,
    pub reading_order: usize,
    pub interactivity: NodeInteractivity,
    pub availability: NodeAvailability,
    pub visibility: NodeVisibility,
    pub modal_role: ModalRole,
    pub expansion: ExpansionState,
    pub selection: SelectionState,
    pub toggled: Option<accesskit::Toggled>,
    pub live: Option<accesskit::Live>,
    pub invalid: Option<accesskit::Invalid>,
    pub bounds: Option<accesskit::Rect>,
}

impl AccessibilityNodeRecord {
    /// Extracts a typed record directly from an ECS entity and its `AccessibilityNode`.
    #[must_use]
    pub fn from_bevy_node(
        entity: Entity,
        marker: SemanticControlMarker,
        node: &AccessibilityNode,
    ) -> Self {
        let interactivity = if marker.control.is_interactive() {
            NodeInteractivity::Interactive
        } else {
            NodeInteractivity::Structural
        };
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
        let modal_role = if node.is_modal() {
            ModalRole::ModalDialog
        } else {
            ModalRole::NonModal
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

        Self {
            node_id: node_id_for_entity(entity),
            control: marker.control,
            role: node.role(),
            name: node.label().unwrap_or_default().to_owned(),
            value: node.value().map(ToOwned::to_owned),
            reading_order: marker.reading_order,
            interactivity,
            availability,
            visibility,
            modal_role,
            expansion,
            selection,
            toggled: node.toggled(),
            live: node.live(),
            invalid: node.invalid(),
            bounds: node.bounds(),
        }
    }
}

/// Violations detected by the zero-duplicate-node and tree-integrity checker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessibilityInvariantError {
    DuplicateNodeId {
        node_id: accesskit::NodeId,
        first_control: SemanticControlId,
        second_control: SemanticControlId,
    },
    DuplicateSemanticControl {
        control: SemanticControlId,
        count: usize,
    },
    MissingInteractiveControl {
        control: SemanticControlId,
    },
    UnexpectedVisibleInteractiveControl {
        control: SemanticControlId,
    },
    EmptyAccessibleName {
        control: SemanticControlId,
        role: accesskit::Role,
    },
    NonMonotonicReadingOrder {
        expected_index: usize,
        actual_order: usize,
        control: SemanticControlId,
    },
    ModalDialogMissingWhenOpen,
    ModalDialogExposedWhenClosed,
    NonModalControlVisibleDuringModal {
        control: SemanticControlId,
    },
}

impl fmt::Display for AccessibilityInvariantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateNodeId {
                node_id,
                first_control,
                second_control,
            } => write!(
                f,
                "duplicate AccessKit NodeId {:?} shared by {:?} and {:?}",
                node_id, first_control, second_control
            ),
            Self::DuplicateSemanticControl { control, count } => write!(
                f,
                "semantic control {:?} appeared {} times in the accessibility tree (must be exactly 1)",
                control, count
            ),
            Self::MissingInteractiveControl { control } => {
                write!(
                    f,
                    "expected interactive control {:?} missing from tree",
                    control
                )
            }
            Self::UnexpectedVisibleInteractiveControl { control } => write!(
                f,
                "unexpected visible interactive control {:?} exposed outside expected set",
                control
            ),
            Self::EmptyAccessibleName { control, role } => write!(
                f,
                "interactive control {:?} ({:?}) has an empty accessible name",
                control, role
            ),
            Self::NonMonotonicReadingOrder {
                expected_index,
                actual_order,
                control,
            } => write!(
                f,
                "control {:?} reading order {actual_order} violates monotonic sequence at position {expected_index}",
                control
            ),
            Self::ModalDialogMissingWhenOpen => {
                write!(
                    f,
                    "modal state is open but no visible Role::Dialog modal node exists"
                )
            }
            Self::ModalDialogExposedWhenClosed => {
                write!(
                    f,
                    "modal state is closed but a modal Role::Dialog node is still visible"
                )
            }
            Self::NonModalControlVisibleDuringModal { control } => write!(
                f,
                "non-modal interactive control {:?} remained visible while modal dialog is open",
                control
            ),
        }
    }
}

impl std::error::Error for AccessibilityInvariantError {}

/// Verified summary returned when a snapshot passes all single-owner tree invariants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeAuditSummary {
    pub total_nodes: usize,
    pub visible_interactive_nodes: usize,
    pub hidden_nodes: usize,
    pub live_regions: usize,
}

/// Single-owner accessibility tree snapshot rebuilt directly from Bevy's
/// `AccessibilityNode` components.
#[derive(Debug, Clone, Default, Resource)]
pub struct AccessibilityTreeSnapshot {
    nodes: Vec<AccessibilityNodeRecord>,
    focused_node_id: Option<accesskit::NodeId>,
}

impl AccessibilityTreeSnapshot {
    #[must_use]
    pub fn from_records(
        mut nodes: Vec<AccessibilityNodeRecord>,
        focused_node_id: Option<accesskit::NodeId>,
    ) -> Self {
        nodes.sort_by_key(|record| record.reading_order);
        Self {
            nodes,
            focused_node_id,
        }
    }

    #[must_use]
    pub fn nodes(&self) -> &[AccessibilityNodeRecord] {
        &self.nodes
    }

    #[must_use]
    pub const fn focused_node_id(&self) -> Option<accesskit::NodeId> {
        self.focused_node_id
    }

    #[must_use]
    pub fn find_by_control(&self, control: SemanticControlId) -> Option<&AccessibilityNodeRecord> {
        self.nodes.iter().find(|node| node.control == control)
    }

    /// Asserts the zero-duplicate-node invariant, unique AccessKit `NodeId`s,
    /// non-empty accessible names, monotonic reading order, and modal scoping.
    pub fn verify_zero_duplicate_nodes(
        &self,
        expected_visible_interactive: &[SemanticControlId],
        modal_state: &ModalState,
    ) -> Result<TreeAuditSummary, Vec<AccessibilityInvariantError>> {
        let mut errors = Vec::new();
        let mut seen_ids: HashMap<accesskit::NodeId, SemanticControlId> = HashMap::new();
        let mut control_counts: HashMap<SemanticControlId, usize> = HashMap::new();
        let expected_set: HashSet<SemanticControlId> =
            expected_visible_interactive.iter().copied().collect();

        let mut visible_interactive_nodes = 0;
        let mut hidden_nodes = 0;
        let mut live_regions = 0;
        let mut visible_modal_dialogs = 0;

        for (idx, node) in self.nodes.iter().enumerate() {
            if let Some(&first_control) = seen_ids.get(&node.node_id) {
                errors.push(AccessibilityInvariantError::DuplicateNodeId {
                    node_id: node.node_id,
                    first_control,
                    second_control: node.control,
                });
            } else {
                let _ = seen_ids.insert(node.node_id, node.control);
            }

            *control_counts.entry(node.control).or_insert(0) += 1;

            if node.reading_order != idx {
                errors.push(AccessibilityInvariantError::NonMonotonicReadingOrder {
                    expected_index: idx,
                    actual_order: node.reading_order,
                    control: node.control,
                });
            }

            if matches!(node.visibility, NodeVisibility::Hidden) {
                hidden_nodes += 1;
            }

            if node.live.is_some() {
                live_regions += 1;
            }

            if matches!(node.modal_role, ModalRole::ModalDialog)
                && node.role == accesskit::Role::Dialog
                && matches!(node.visibility, NodeVisibility::Visible)
            {
                visible_modal_dialogs += 1;
            }

            if matches!(node.interactivity, NodeInteractivity::Interactive) {
                if node.name.trim().is_empty() {
                    errors.push(AccessibilityInvariantError::EmptyAccessibleName {
                        control: node.control,
                        role: node.role,
                    });
                }

                if matches!(node.visibility, NodeVisibility::Visible) {
                    visible_interactive_nodes += 1;
                    if !expected_set.contains(&node.control) {
                        errors.push(
                            AccessibilityInvariantError::UnexpectedVisibleInteractiveControl {
                                control: node.control,
                            },
                        );
                    }
                    if modal_state.is_open() && !node.control.is_modal_descendant() {
                        errors.push(
                            AccessibilityInvariantError::NonModalControlVisibleDuringModal {
                                control: node.control,
                            },
                        );
                    }
                }
            }
        }

        for (&control, &count) in &control_counts {
            if count != 1 {
                errors
                    .push(AccessibilityInvariantError::DuplicateSemanticControl { control, count });
            }
        }

        for &expected in expected_visible_interactive {
            let found_visible = self.nodes.iter().any(|node| {
                node.control == expected && matches!(node.visibility, NodeVisibility::Visible)
            });
            if !found_visible {
                errors.push(AccessibilityInvariantError::MissingInteractiveControl {
                    control: expected,
                });
            }
        }

        match modal_state {
            ModalState::Open { .. } if visible_modal_dialogs != 1 => {
                errors.push(AccessibilityInvariantError::ModalDialogMissingWhenOpen);
            }
            ModalState::Closed if visible_modal_dialogs > 0 => {
                errors.push(AccessibilityInvariantError::ModalDialogExposedWhenClosed);
            }
            _ => {}
        }

        if errors.is_empty() {
            Ok(TreeAuditSummary {
                total_nodes: self.nodes.len(),
                visible_interactive_nodes,
                hidden_nodes,
                live_regions,
            })
        } else {
            Err(errors)
        }
    }
}
