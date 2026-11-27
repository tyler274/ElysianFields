use std::collections::{HashMap, HashSet};
use std::fmt;

use bevy::prelude::Resource;

use crate::domain::SemanticControlId;
use crate::live_region::{AnnouncementPriority, LiveAnnouncementQueue};
use crate::tree::{
    AccessibilityNodeRecord, AccessibilityTreeSnapshot, ExpansionState, ModalRole,
    NodeAvailability, NodeInteractivity, NodeVisibility, SelectionState,
};

/// Canonical `/elysianfields` subpath prefix enforced by the Elysian Realm contract.
pub const ELYSIAN_SUBPATH_PREFIX: &str = "/elysianfields";

/// Exhaustive semantic ARIA role mapped directly from `accesskit::Role`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AriaRole {
    Navigation,
    Main,
    Form,
    Region,
    Heading,
    Link,
    Button,
    Textbox,
    Checkbox,
    Dialog,
    Alert,
    Status,
}

impl AriaRole {
    /// Converts supported AccessKit roles into semantic ARIA roles without raw string drift.
    pub const fn from_accesskit(role: accesskit::Role) -> Result<Self, UnsupportedAccessKitRole> {
        match role {
            accesskit::Role::Navigation => Ok(Self::Navigation),
            accesskit::Role::Main => Ok(Self::Main),
            accesskit::Role::Form => Ok(Self::Form),
            accesskit::Role::Region => Ok(Self::Region),
            accesskit::Role::Heading => Ok(Self::Heading),
            accesskit::Role::Link => Ok(Self::Link),
            accesskit::Role::Button => Ok(Self::Button),
            accesskit::Role::TextInput => Ok(Self::Textbox),
            accesskit::Role::CheckBox => Ok(Self::Checkbox),
            accesskit::Role::Dialog => Ok(Self::Dialog),
            accesskit::Role::Alert => Ok(Self::Alert),
            accesskit::Role::Status => Ok(Self::Status),
            other => Err(UnsupportedAccessKitRole { role: other }),
        }
    }

    #[must_use]
    pub const fn aria_token(self) -> &'static str {
        match self {
            Self::Navigation => "navigation",
            Self::Main => "main",
            Self::Form => "form",
            Self::Region => "region",
            Self::Heading => "heading",
            Self::Link => "link",
            Self::Button => "button",
            Self::Textbox => "textbox",
            Self::Checkbox => "checkbox",
            Self::Dialog => "dialog",
            Self::Alert => "alert",
            Self::Status => "status",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsupportedAccessKitRole {
    pub role: accesskit::Role,
}

impl fmt::Display for UnsupportedAccessKitRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "AccessKit role {:?} has no narrow ARIA semantic bridge mapping",
            self.role
        )
    }
}

impl std::error::Error for UnsupportedAccessKitRole {}

/// Explicit keyboard tab-order participation in the narrow semantic overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabOrderParticipation {
    Sequential,
    ProgrammaticOnly,
    Excluded,
}

/// Typed semantic ARIA state projected from a single AccessKit node.
/// Carries zero visual styling or layout properties.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeAriaState {
    pub availability: NodeAvailability,
    pub expansion: ExpansionState,
    pub selection: SelectionState,
    pub toggled: Option<accesskit::Toggled>,
    pub modal_role: ModalRole,
    pub invalid: Option<accesskit::Invalid>,
    pub live: Option<accesskit::Live>,
}

/// A single narrow semantic element projected to `web/a11y-bridge.js`.
/// Keyed by the exact `accesskit::NodeId` so duplicate DOM/AccessKit nodes are impossible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeElement {
    pub node_id: accesskit::NodeId,
    pub control: SemanticControlId,
    pub control_key: &'static str,
    pub role: AriaRole,
    pub name: String,
    pub value: Option<String>,
    pub tab_order: TabOrderParticipation,
    pub state: BridgeAriaState,
}

/// Complete narrow semantic projection of the active screen for the browser Wasm bridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeProjection {
    pub mount_subpath: &'static str,
    pub elements: Vec<BridgeElement>,
    pub active_focus: Option<accesskit::NodeId>,
    pub polite_live_announcement: Option<String>,
    pub assertive_live_announcement: Option<String>,
    pub modal_state: BridgeModalScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeModalScope {
    StandardDocument,
    ModalDialogActive { dialog_node_id: accesskit::NodeId },
}

/// Minimal differential patch sent across the Wasm <-> JS bridge boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgePatch {
    UpsertElement(BridgeElement),
    RemoveElement { node_id: accesskit::NodeId },
    SetFocus { node_id: Option<accesskit::NodeId> },
    SetPoliteLive { text: Option<String> },
    SetAssertiveLive { text: Option<String> },
    SetModalScope { scope: BridgeModalScope },
}

/// Errors proving a bridge projection violated the narrow single-owner invariant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeProjectionError {
    UnsupportedRole {
        control: SemanticControlId,
        source: UnsupportedAccessKitRole,
    },
    DuplicateProjectedNodeId {
        node_id: accesskit::NodeId,
    },
    NodeCountMismatch {
        visible_accesskit_nodes: usize,
        projected_bridge_nodes: usize,
    },
    MissingProjectedNode {
        node_id: accesskit::NodeId,
        control: SemanticControlId,
    },
    FocusNodeNotProjected {
        node_id: accesskit::NodeId,
    },
    InvalidSubpath {
        mount_subpath: String,
    },
}

impl fmt::Display for BridgeProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedRole { control, source } => {
                write!(f, "failed to project control {control:?}: {source}")
            }
            Self::DuplicateProjectedNodeId { node_id } => {
                write!(f, "browser bridge duplicated AccessKit NodeId {node_id:?}")
            }
            Self::NodeCountMismatch {
                visible_accesskit_nodes,
                projected_bridge_nodes,
            } => write!(
                f,
                "browser bridge projected {projected_bridge_nodes} elements vs {visible_accesskit_nodes} visible AccessKit nodes"
            ),
            Self::MissingProjectedNode { node_id, control } => write!(
                f,
                "visible AccessKit node {node_id:?} ({control:?}) missing from browser bridge projection"
            ),
            Self::FocusNodeNotProjected { node_id } => write!(
                f,
                "active focus target {node_id:?} is not present in projected browser elements"
            ),
            Self::InvalidSubpath { mount_subpath } => write!(
                f,
                "browser bridge mount subpath '{mount_subpath}' must start with '{ELYSIAN_SUBPATH_PREFIX}'"
            ),
        }
    }
}

impl std::error::Error for BridgeProjectionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::UnsupportedRole { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Quantitative audit proving the browser adapter stays narrow and never becomes a second UI stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeBudgetReport {
    pub projected_elements: usize,
    pub interactive_elements: usize,
    pub structural_landmarks: usize,
    pub duplicated_nodes: usize,
    pub mirrored_css_rules: usize,
}

/// Typed browser accessibility projection trait decoupling the single AccessKit
/// tree from any platform-specific DOM/ARIA implementation.
pub trait BrowserA11yBridge {
    fn project(
        &self,
        snapshot: &AccessibilityTreeSnapshot,
        live_queue: &LiveAnnouncementQueue,
    ) -> Result<BridgeProjection, BridgeProjectionError>;

    fn diff(
        &self,
        previous: Option<&BridgeProjection>,
        next: &BridgeProjection,
    ) -> Vec<BridgePatch>;
}

/// Concrete narrow semantic bridge resource owned by the Bevy prototype plugin.
#[derive(Debug, Clone, Default, Resource)]
pub struct NarrowSemanticBridge {
    last_projection: Option<BridgeProjection>,
    last_patches: Vec<BridgePatch>,
}

impl NarrowSemanticBridge {
    #[must_use]
    pub fn last_projection(&self) -> Option<&BridgeProjection> {
        self.last_projection.as_ref()
    }

    #[must_use]
    pub fn last_patches(&self) -> &[BridgePatch] {
        &self.last_patches
    }

    /// Re-projects the active AccessKit snapshot and computes minimal DOM patches.
    pub fn sync_from_snapshot(
        &mut self,
        snapshot: &AccessibilityTreeSnapshot,
        live_queue: &LiveAnnouncementQueue,
    ) -> Result<&[BridgePatch], BridgeProjectionError> {
        let next = self.project(snapshot, live_queue)?;
        let patches = self.diff(self.last_projection.as_ref(), &next);
        self.last_projection = Some(next);
        self.last_patches = patches;
        Ok(&self.last_patches)
    }
}

impl BrowserA11yBridge for NarrowSemanticBridge {
    fn project(
        &self,
        snapshot: &AccessibilityTreeSnapshot,
        live_queue: &LiveAnnouncementQueue,
    ) -> Result<BridgeProjection, BridgeProjectionError> {
        let mut elements = Vec::new();
        let mut modal_state = BridgeModalScope::StandardDocument;

        for record in snapshot.nodes() {
            if matches!(record.visibility, NodeVisibility::Hidden) {
                continue;
            }
            let element = project_single_record(record)?;
            if matches!(record.modal_role, ModalRole::ModalDialog) {
                modal_state = BridgeModalScope::ModalDialogActive {
                    dialog_node_id: record.node_id,
                };
            }
            elements.push(element);
        }

        Ok(BridgeProjection {
            mount_subpath: ELYSIAN_SUBPATH_PREFIX,
            elements,
            active_focus: snapshot.focused_node_id(),
            polite_live_announcement: live_queue
                .latest_text(AnnouncementPriority::Polite)
                .map(ToOwned::to_owned),
            assertive_live_announcement: live_queue
                .latest_text(AnnouncementPriority::Assertive)
                .map(ToOwned::to_owned),
            modal_state,
        })
    }

    fn diff(
        &self,
        previous: Option<&BridgeProjection>,
        next: &BridgeProjection,
    ) -> Vec<BridgePatch> {
        let mut patches = Vec::new();

        let prev_map: HashMap<accesskit::NodeId, &BridgeElement> = previous
            .map(|proj| {
                proj.elements
                    .iter()
                    .map(|elem| (elem.node_id, elem))
                    .collect()
            })
            .unwrap_or_default();

        let next_ids: HashSet<accesskit::NodeId> =
            next.elements.iter().map(|elem| elem.node_id).collect();

        for &prev_id in prev_map.keys() {
            if !next_ids.contains(&prev_id) {
                patches.push(BridgePatch::RemoveElement { node_id: prev_id });
            }
        }

        for elem in &next.elements {
            let unchanged = prev_map
                .get(&elem.node_id)
                .is_some_and(|prev_elem| *prev_elem == elem);
            if !unchanged {
                patches.push(BridgePatch::UpsertElement(elem.clone()));
            }
        }

        if previous.map(|p| p.modal_state) != Some(next.modal_state) {
            patches.push(BridgePatch::SetModalScope {
                scope: next.modal_state,
            });
        }

        if previous.map(|p| p.active_focus) != Some(next.active_focus) {
            patches.push(BridgePatch::SetFocus {
                node_id: next.active_focus,
            });
        }

        if previous.and_then(|p| p.polite_live_announcement.as_deref())
            != next.polite_live_announcement.as_deref()
        {
            patches.push(BridgePatch::SetPoliteLive {
                text: next.polite_live_announcement.clone(),
            });
        }

        if previous.and_then(|p| p.assertive_live_announcement.as_deref())
            != next.assertive_live_announcement.as_deref()
        {
            patches.push(BridgePatch::SetAssertiveLive {
                text: next.assertive_live_announcement.clone(),
            });
        }

        patches
    }
}

fn project_single_record(
    record: &AccessibilityNodeRecord,
) -> Result<BridgeElement, BridgeProjectionError> {
    let role = AriaRole::from_accesskit(record.role).map_err(|source| {
        BridgeProjectionError::UnsupportedRole {
            control: record.control,
            source,
        }
    })?;

    let tab_order = match (record.interactivity, record.availability) {
        (NodeInteractivity::Interactive, NodeAvailability::Enabled) => {
            TabOrderParticipation::Sequential
        }
        (NodeInteractivity::Interactive, NodeAvailability::Disabled) => {
            TabOrderParticipation::ProgrammaticOnly
        }
        (NodeInteractivity::Structural, _) => TabOrderParticipation::Excluded,
    };

    Ok(BridgeElement {
        node_id: record.node_id,
        control: record.control,
        control_key: record.control.stable_key(),
        role,
        name: record.name.clone(),
        value: record.value.clone(),
        tab_order,
        state: BridgeAriaState {
            availability: record.availability,
            expansion: record.expansion,
            selection: record.selection,
            toggled: record.toggled,
            modal_role: record.modal_role,
            invalid: record.invalid,
            live: record.live,
        },
    })
}

/// Verifies a strict 1:1 bijection between visible AccessKit nodes and browser
/// bridge elements, guaranteeing zero duplicated nodes and `/elysianfields` scoping.
pub fn verify_bijective_projection(
    snapshot: &AccessibilityTreeSnapshot,
    projection: &BridgeProjection,
) -> Result<BridgeBudgetReport, Vec<BridgeProjectionError>> {
    let mut errors = Vec::new();

    if !projection.mount_subpath.starts_with(ELYSIAN_SUBPATH_PREFIX) {
        errors.push(BridgeProjectionError::InvalidSubpath {
            mount_subpath: projection.mount_subpath.to_owned(),
        });
    }

    let visible_nodes: Vec<&AccessibilityNodeRecord> = snapshot
        .nodes()
        .iter()
        .filter(|node| matches!(node.visibility, NodeVisibility::Visible))
        .collect();

    if visible_nodes.len() != projection.elements.len() {
        errors.push(BridgeProjectionError::NodeCountMismatch {
            visible_accesskit_nodes: visible_nodes.len(),
            projected_bridge_nodes: projection.elements.len(),
        });
    }

    let mut seen_projected_ids = HashSet::new();
    for elem in &projection.elements {
        if !seen_projected_ids.insert(elem.node_id) {
            errors.push(BridgeProjectionError::DuplicateProjectedNodeId {
                node_id: elem.node_id,
            });
        }
    }

    for node in &visible_nodes {
        if !seen_projected_ids.contains(&node.node_id) {
            errors.push(BridgeProjectionError::MissingProjectedNode {
                node_id: node.node_id,
                control: node.control,
            });
        }
    }

    if let Some(focus_id) = projection.active_focus
        && !seen_projected_ids.contains(&focus_id)
    {
        errors.push(BridgeProjectionError::FocusNodeNotProjected { node_id: focus_id });
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    let interactive_elements = projection
        .elements
        .iter()
        .filter(|elem| !matches!(elem.tab_order, TabOrderParticipation::Excluded))
        .count();

    Ok(BridgeBudgetReport {
        projected_elements: projection.elements.len(),
        interactive_elements,
        structural_landmarks: projection.elements.len() - interactive_elements,
        duplicated_nodes: 0,
        mirrored_css_rules: 0,
    })
}
