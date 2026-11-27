use std::collections::HashMap;

use bevy::a11y::AccessibilityNode;
use bevy::input_focus::{FocusCause, InputFocus, InputFocusVisible};
use bevy::prelude::{
    App, Commands, Entity, IntoScheduleConfigs, Node, Plugin, Query, Res, ResMut, Resource,
    Startup, Update,
};

use crate::bridge::{BridgeProjectionError, NarrowSemanticBridge};
use crate::domain::{
    CitationId, ControlAvailability, FieldValidation, ModalState, NavItemId, ScreenState,
    SemanticControlId,
};
use crate::focus::{FocusManager, FocusTransitionOutcome, KeyboardCommand};
use crate::live_region::{AnnouncementPriority, LiveAnnouncementQueue};
use crate::preferences::{AccessibilityPreferences, compute_scaled_layout};
use crate::tree::{
    AccessibilityNodeRecord, AccessibilityTreeSnapshot, SemanticControlMarker, node_id_for_entity,
};

/// Single-owner mapping from [`SemanticControlId`] to its unique Bevy UI `Entity`.
#[derive(Debug, Default, Resource)]
pub struct SemanticEntityRegistry {
    entities: HashMap<SemanticControlId, Entity>,
}

impl SemanticEntityRegistry {
    #[must_use]
    pub fn entity_for(&self, control: SemanticControlId) -> Option<Entity> {
        self.entities.get(&control).copied()
    }
}

/// Command inbox processed deterministically inside Bevy `Update` schedules.
#[derive(Debug, Default, Resource)]
pub struct KeyboardCommandQueue {
    pending: Vec<(u64, KeyboardCommand)>,
    last_outcomes: Vec<FocusTransitionOutcome>,
}

impl KeyboardCommandQueue {
    pub fn push(&mut self, now_ms: u64, command: KeyboardCommand) {
        self.pending.push((now_ms, command));
    }

    #[must_use]
    pub fn last_outcomes(&self) -> &[FocusTransitionOutcome] {
        &self.last_outcomes
    }
}

/// Captures the latest bridge sync result so failures never silently read as success.
#[derive(Debug, Default, Resource)]
pub struct BridgeSyncStatus {
    pub last_error: Option<BridgeProjectionError>,
}

/// Bevy 0.20.1 accessibility prototype plugin wiring the four surfaces,
/// single-owner AccessKit nodes, keyboard focus manager, preferences, live
/// announcement queue, and narrow browser semantic bridge.
pub struct AccessibilityPrototypePlugin;

impl Plugin for AccessibilityPrototypePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScreenState>()
            .init_resource::<FocusManager>()
            .init_resource::<AccessibilityPreferences>()
            .init_resource::<LiveAnnouncementQueue>()
            .init_resource::<SemanticEntityRegistry>()
            .init_resource::<KeyboardCommandQueue>()
            .init_resource::<AccessibilityTreeSnapshot>()
            .init_resource::<NarrowSemanticBridge>()
            .init_resource::<BridgeSyncStatus>()
            .init_resource::<InputFocus>()
            .init_resource::<InputFocusVisible>()
            .add_systems(Startup, spawn_accessibility_surfaces)
            .add_systems(
                Update,
                (
                    process_keyboard_commands,
                    sync_input_focus,
                    sync_bevy_accessibility_nodes,
                    rebuild_snapshot_and_bridge,
                )
                    .chain(),
            );
    }
}

fn spawn_accessibility_surfaces(
    mut commands: Commands,
    mut registry: ResMut<SemanticEntityRegistry>,
) {
    for (reading_order, control) in SemanticControlId::ALL_IN_READING_ORDER
        .into_iter()
        .enumerate()
    {
        let initial_node = accesskit::Node::new(default_role_for_control(control));
        let entity = commands
            .spawn((
                Node::default(),
                SemanticControlMarker {
                    control,
                    reading_order,
                },
                AccessibilityNode(initial_node),
            ))
            .id();
        let _ = registry.entities.insert(control, entity);
    }
}

fn process_keyboard_commands(
    mut queue: ResMut<KeyboardCommandQueue>,
    mut screen: ResMut<ScreenState>,
    mut focus: ResMut<FocusManager>,
    mut live: ResMut<LiveAnnouncementQueue>,
    preferences: Res<AccessibilityPreferences>,
) {
    queue.last_outcomes.clear();
    let drained = std::mem::take(&mut queue.pending);
    for (now_ms, command) in drained {
        let outcome = focus.apply_keyboard_command(
            &mut screen,
            &mut live,
            preferences.theme,
            now_ms,
            command,
        );
        queue.last_outcomes.push(outcome);
    }
}

fn sync_input_focus(
    focus: Res<FocusManager>,
    registry: Res<SemanticEntityRegistry>,
    mut input_focus: ResMut<InputFocus>,
    mut input_focus_visible: ResMut<InputFocusVisible>,
) {
    match registry.entity_for(focus.current().semantic_id()) {
        Some(entity) => input_focus.set(entity, FocusCause::Navigated),
        None => input_focus.clear(),
    }
    input_focus_visible.0 = true;
}

fn sync_bevy_accessibility_nodes(
    screen: Res<ScreenState>,
    preferences: Res<AccessibilityPreferences>,
    live: Res<LiveAnnouncementQueue>,
    mut query: Query<(&SemanticControlMarker, &mut AccessibilityNode)>,
) {
    let layout_rects =
        compute_scaled_layout(preferences.text_scale, preferences.viewport, &screen.modal);
    let rect_map: HashMap<SemanticControlId, accesskit::Rect> = layout_rects
        .into_iter()
        .map(|rect| (rect.control, rect.to_accesskit_rect()))
        .collect();

    for (marker, mut a11y_node) in &mut query {
        let built = build_accesskit_node(
            marker.control,
            &screen,
            &live,
            rect_map.get(&marker.control).copied(),
        );
        a11y_node.0 = built;
    }
}

fn rebuild_snapshot_and_bridge(
    focus: Res<FocusManager>,
    registry: Res<SemanticEntityRegistry>,
    live: Res<LiveAnnouncementQueue>,
    query: Query<(Entity, &SemanticControlMarker, &AccessibilityNode)>,
    mut snapshot: ResMut<AccessibilityTreeSnapshot>,
    mut bridge: ResMut<NarrowSemanticBridge>,
    mut sync_status: ResMut<BridgeSyncStatus>,
) {
    let records: Vec<AccessibilityNodeRecord> = query
        .iter()
        .map(|(entity, &marker, node)| {
            AccessibilityNodeRecord::from_bevy_node(entity, marker, node)
        })
        .collect();

    let focused_node_id = registry
        .entity_for(focus.current().semantic_id())
        .map(node_id_for_entity);

    *snapshot = AccessibilityTreeSnapshot::from_records(records, focused_node_id);

    match bridge.sync_from_snapshot(&snapshot, &live) {
        Ok(_) => {
            sync_status.last_error = None;
        }
        Err(err) => {
            sync_status.last_error = Some(err);
        }
    }
}

/// Single owner defining the AccessKit `Node` role, accessible name, state,
/// visibility, live-region mode, and bounds for every semantic control.
#[must_use]
pub fn build_accesskit_node(
    control: SemanticControlId,
    screen: &ScreenState,
    live: &LiveAnnouncementQueue,
    bounds: Option<accesskit::Rect>,
) -> accesskit::Node {
    let mut node = accesskit::Node::new(default_role_for_control(control));
    let modal_open = screen.modal.is_open();

    if let Some(rect) = bounds {
        node.set_bounds(rect);
    }

    match control {
        SemanticControlId::WorkspaceNavigationLandmark => {
            node.set_label("Workspace navigation");
        }
        SemanticControlId::NavItem(item) => {
            node.set_label(item.label());
            node.set_selected(screen.active_nav == item);
            node.add_action(accesskit::Action::Focus);
            node.add_action(accesskit::Action::Click);
        }
        SemanticControlId::TranscriptMainLandmark => {
            node.set_label("Streaming clinical transcript");
        }
        SemanticControlId::TranscriptHeading => {
            node.set_label("Assistant clinical review");
        }
        SemanticControlId::TranscriptLiveRegion => {
            node.set_label("Streaming assistant updates");
            node.set_live(AnnouncementPriority::Polite.to_accesskit());
            if let Some(latest) = live.latest_text(AnnouncementPriority::Polite) {
                node.set_value(latest);
            }
        }
        SemanticControlId::CitationLink(citation) => {
            node.set_label(citation.accessible_name());
            node.set_description(citation.marker());
            node.add_action(accesskit::Action::Focus);
            node.add_action(accesskit::Action::Click);
        }
        SemanticControlId::ClinicalFormLandmark => {
            node.set_label("Clinical assessment form");
        }
        SemanticControlId::ClinicalFormHeading => {
            node.set_label("Document synthetic assessment");
        }
        SemanticControlId::AssessmentNoteInput => {
            node.set_label("Clinical assessment note");
            node.set_value(screen.note_draft.as_str());
            node.add_action(accesskit::Action::Focus);
            node.add_action(accesskit::Action::SetValue);
            if matches!(screen.validation, FieldValidation::Invalid(_)) {
                node.set_invalid(accesskit::Invalid::True);
            }
        }
        SemanticControlId::IncludeCitationsCheckbox => {
            node.set_label("Attach synthetic citation references");
            node.set_toggled(screen.include_citations.to_accesskit());
            node.add_action(accesskit::Action::Focus);
            node.add_action(accesskit::Action::Click);
        }
        SemanticControlId::GuidanceDisclosure => {
            node.set_label("Clinical documentation guidance");
            node.set_expanded(screen.guidance_disclosure.is_expanded());
            node.add_action(accesskit::Action::Focus);
            node.add_action(accesskit::Action::Click);
        }
        SemanticControlId::FormValidationAlert => {
            node.set_label("Form validation alert");
            node.set_live(AnnouncementPriority::Assertive.to_accesskit());
            match screen.validation {
                FieldValidation::Invalid(err) => {
                    node.set_value(err.to_string());
                }
                FieldValidation::Untouched | FieldValidation::Valid => {
                    node.set_hidden();
                }
            }
        }
        SemanticControlId::SubmitNoteButton => {
            node.set_label("Submit clinical note");
            node.add_action(accesskit::Action::Focus);
            node.add_action(accesskit::Action::Click);
        }
        SemanticControlId::OpenSignOffModalButton => {
            node.set_label("Review and sign off note");
            node.add_action(accesskit::Action::Focus);
            if matches!(screen.sign_off_availability(), ControlAvailability::Enabled) {
                node.add_action(accesskit::Action::Click);
            } else {
                node.set_disabled();
            }
        }
        SemanticControlId::SignOffModalDialog => match screen.modal {
            ModalState::Open {
                title, description, ..
            } => {
                node.set_label(title);
                node.set_description(description);
                node.set_modal();
            }
            ModalState::Closed => {
                node.set_label("Confirm clinical sign-off");
                node.set_hidden();
            }
        },
        SemanticControlId::SignOffModalHeading => {
            node.set_label("Confirm clinical sign-off");
            if !modal_open {
                node.set_hidden();
            }
        }
        SemanticControlId::ModalConfirmButton => {
            node.set_label("Confirm sign-off");
            node.add_action(accesskit::Action::Focus);
            node.add_action(accesskit::Action::Click);
            if !modal_open {
                node.set_hidden();
            }
        }
        SemanticControlId::ModalCancelButton => {
            node.set_label("Cancel and return");
            node.add_action(accesskit::Action::Focus);
            node.add_action(accesskit::Action::Click);
            if !modal_open {
                node.set_hidden();
            }
        }
    }

    // When the modal dialog is open, hide non-modal controls so assistive
    // technology cannot traverse behind the focus trap.
    if modal_open && !control.is_modal_descendant() {
        node.set_hidden();
    }

    node
}

#[must_use]
const fn default_role_for_control(control: SemanticControlId) -> accesskit::Role {
    match control {
        SemanticControlId::WorkspaceNavigationLandmark => accesskit::Role::Navigation,
        SemanticControlId::NavItem(
            NavItemId::Inbox
            | NavItemId::Transcript
            | NavItemId::ClinicalForm
            | NavItemId::LabRecords,
        ) => accesskit::Role::Button,
        SemanticControlId::TranscriptMainLandmark => accesskit::Role::Main,
        SemanticControlId::TranscriptHeading
        | SemanticControlId::ClinicalFormHeading
        | SemanticControlId::SignOffModalHeading => accesskit::Role::Heading,
        SemanticControlId::TranscriptLiveRegion => accesskit::Role::Status,
        SemanticControlId::CitationLink(
            CitationId::ProgressNoteSummary | CitationId::ClinicalLabPanel,
        ) => accesskit::Role::Link,
        SemanticControlId::ClinicalFormLandmark => accesskit::Role::Form,
        SemanticControlId::AssessmentNoteInput => accesskit::Role::TextInput,
        SemanticControlId::IncludeCitationsCheckbox => accesskit::Role::CheckBox,
        SemanticControlId::GuidanceDisclosure
        | SemanticControlId::SubmitNoteButton
        | SemanticControlId::OpenSignOffModalButton
        | SemanticControlId::ModalConfirmButton
        | SemanticControlId::ModalCancelButton => accesskit::Role::Button,
        SemanticControlId::FormValidationAlert => accesskit::Role::Alert,
        SemanticControlId::SignOffModalDialog => accesskit::Role::Dialog,
    }
}
