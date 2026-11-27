use bevy::a11y::AccessibilityNode;
use bevy::input_focus::{FocusCause, InputFocus, InputFocusVisible};
use bevy::prelude::{
    App, Entity, IntoScheduleConfigs, Plugin, Query, Res, ResMut, Resource, Update,
};

use crate::a11y::{
    A11yNodeId, A11yNodeMarker, A11yProjection, A11yProjectionError, AriaBridgeEmitter,
    PoliteLiveQueue, ProjectedA11yNode,
};
use crate::composer::ComposerSurface;
use crate::i18n::UiLocale;
use crate::motion::{IdleRedrawGate, ReducedMotion};
use crate::shortcuts::ShortcutRouter;
use crate::theme::UiTheme;

/// Captures any accessibility projection or bridge sync error so failures never read as success.
#[derive(Debug, Default, Resource)]
pub struct CatalogA11ySyncStatus {
    pub errors: Vec<A11yProjectionError>,
}

/// Production shared Bevy `0.20.1` UI catalog plugin serving both Curie Clinic
/// and Curie Mobile.
pub struct EfUiCatalogPlugin;

impl Plugin for EfUiCatalogPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UiTheme>()
            .init_resource::<ReducedMotion>()
            .init_resource::<IdleRedrawGate>()
            .init_resource::<UiLocale>()
            .init_resource::<ComposerSurface>()
            .init_resource::<ShortcutRouter>()
            .init_resource::<PoliteLiveQueue>()
            .init_resource::<A11yProjection>()
            .init_resource::<AriaBridgeEmitter>()
            .init_resource::<CatalogA11ySyncStatus>()
            .init_resource::<InputFocus>()
            .init_resource::<InputFocusVisible>()
            .add_systems(
                Update,
                (
                    sync_composer_direction_with_locale,
                    sync_input_focus_from_router,
                    project_a11y_and_emit_bridge_patches,
                )
                    .chain(),
            );
    }
}

fn sync_composer_direction_with_locale(
    locale: Res<UiLocale>,
    mut composer: ResMut<ComposerSurface>,
) {
    let expected_direction = locale.current.direction();
    if composer.direction() != expected_direction {
        composer.set_direction(expected_direction);
    }
}

fn sync_input_focus_from_router(
    router: Res<ShortcutRouter>,
    query: Query<(Entity, &A11yNodeMarker)>,
    mut input_focus: ResMut<InputFocus>,
    mut input_focus_visible: ResMut<InputFocusVisible>,
) {
    let target_key = &router.current_focus().0;
    let found = query
        .iter()
        .find(|(_, marker)| &marker.key == target_key)
        .map(|(entity, _)| entity);

    match found {
        Some(entity) => {
            input_focus.set(entity, FocusCause::Navigated);
            input_focus_visible.0 = true;
        }
        None => {
            input_focus.clear();
        }
    }
}

fn project_a11y_and_emit_bridge_patches(
    locale: Res<UiLocale>,
    input_focus: Res<InputFocus>,
    live: Res<PoliteLiveQueue>,
    query: Query<(Entity, &A11yNodeMarker, &AccessibilityNode)>,
    mut projection: ResMut<A11yProjection>,
    mut emitter: ResMut<AriaBridgeEmitter>,
    mut status: ResMut<CatalogA11ySyncStatus>,
) {
    let mut nodes = Vec::new();
    let mut errors = Vec::new();

    for (entity, marker, a11y_node) in &query {
        match ProjectedA11yNode::from_bevy_node(entity, marker, a11y_node) {
            Ok(projected) => nodes.push(projected),
            Err(err) => errors.push(err),
        }
    }

    if !errors.is_empty() {
        status.errors = errors;
        return;
    }

    let focused_node = input_focus.get().map(A11yNodeId::from_entity);
    *projection = A11yProjection::from_nodes(nodes, focused_node, locale.current);

    match emitter.sync(&projection, &live) {
        Ok(_) => status.errors.clear(),
        Err(sync_errors) => status.errors = sync_errors,
    }
}
