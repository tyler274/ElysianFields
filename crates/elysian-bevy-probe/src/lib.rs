//! Minimal headless Bevy UI, text, and accessibility probe.
//!
//! Exercises Bevy's `MinimalPlugins`, ECS state transitions, UI layout nodes,
//! text components, and AccessKit accessibility metadata on both native host
//! and `wasm32-unknown-unknown` targets without 3D, audio, or OS window bloat.

use bevy::MinimalPlugins;
use bevy::a11y::AccessibilityNode;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use elysian_smoke::{BuildTarget, WEB_SUBPATH, strip_web_subpath};

/// Lifecycle phase of the headless Bevy UI probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, States)]
pub enum ProbeState {
    #[default]
    Booting,
    Streaming,
    Settled,
}

/// Typed transcript streaming resource independent of ECS view entity IDs.
#[derive(Debug, Resource)]
pub struct TranscriptModel {
    mount_route: &'static str,
    target: BuildTarget,
    chunks_received: u32,
    latest_line: String,
}

impl TranscriptModel {
    /// Creates a new transcript model anchored under the `/elysianfields` mount.
    #[must_use]
    pub fn new(target: BuildTarget) -> Self {
        Self {
            mount_route: WEB_SUBPATH,
            target,
            chunks_received: 0,
            latest_line: String::new(),
        }
    }

    /// Appends a deterministic streamed chunk and advances the chunk counter.
    pub fn push_chunk(&mut self, chunk: &str) {
        self.chunks_received = self.chunks_received.saturating_add(1);
        self.latest_line.push_str(chunk);
    }

    /// Returns the validated mount route for this session.
    #[must_use]
    pub const fn mount_route(&self) -> &'static str {
        self.mount_route
    }

    /// Returns the target classification for this probe session.
    #[must_use]
    pub const fn target(&self) -> BuildTarget {
        self.target
    }

    /// Returns the number of streamed chunks processed so far.
    #[must_use]
    pub const fn chunks_received(&self) -> u32 {
        self.chunks_received
    }

    /// Returns the accumulated transcript line.
    #[must_use]
    pub fn latest_line(&self) -> &str {
        &self.latest_line
    }
}

/// Marker component for the primary accessible transcript view node.
#[derive(Debug, Component)]
pub struct TranscriptViewNode;

/// Plugin registering headless UI, text, accessibility, and state systems.
pub struct ProbePlugin {
    pub target: BuildTarget,
}

impl Plugin for ProbePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(TranscriptModel::new(self.target))
            .init_state::<ProbeState>()
            .add_systems(Startup, spawn_transcript_surface)
            .add_systems(Update, sync_transcript_view);
    }
}

fn spawn_transcript_surface(mut commands: Commands, model: Res<TranscriptModel>) {
    let mut a11y = accesskit::Node::new(accesskit::Role::Document);
    a11y.set_label(format!(
        "Elysian Fields transcript ({})",
        model.mount_route()
    ));

    commands.spawn((
        TranscriptViewNode,
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            ..default()
        },
        Text::new(model.latest_line().to_owned()),
        AccessibilityNode(a11y),
    ));
}

fn sync_transcript_view(
    model: Res<TranscriptModel>,
    mut query: Query<(&mut Text, &mut AccessibilityNode), With<TranscriptViewNode>>,
) {
    if !model.is_changed() {
        return;
    }

    for (mut text, mut a11y) in &mut query {
        text.0.clear();
        text.0.push_str(model.latest_line());
        a11y.0.set_value(model.latest_line().to_owned());
    }
}

/// Builds a headless Bevy application configured with [`MinimalPlugins`] and [`ProbePlugin`].
pub fn build_headless_probe_app(target: BuildTarget) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, ProbePlugin { target }));
    app
}

/// Validates a probe route against the `/elysianfields` web subpath contract.
#[must_use]
pub fn is_valid_probe_route(route: &str) -> bool {
    strip_web_subpath(route).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_probe_boots_and_syncs_text_and_a11y() {
        let mut app = build_headless_probe_app(BuildTarget::BrowserWasm);
        app.update();

        assert_eq!(
            *app.world().resource::<State<ProbeState>>().get(),
            ProbeState::Booting
        );
        assert!(is_valid_probe_route("/elysianfields/probe"));
        assert!(!is_valid_probe_route("/root-only/probe"));

        {
            let mut model = app.world_mut().resource_mut::<TranscriptModel>();
            assert_eq!(model.target(), BuildTarget::BrowserWasm);
            model.push_chunk("Clinical summary: ");
            model.push_chunk("citation [1] verified.");
        }
        app.world_mut()
            .resource_mut::<NextState<ProbeState>>()
            .set(ProbeState::Streaming);
        app.update();

        assert_eq!(
            *app.world().resource::<State<ProbeState>>().get(),
            ProbeState::Streaming
        );

        let mut query = app
            .world_mut()
            .query_filtered::<(&Text, &AccessibilityNode), With<TranscriptViewNode>>();
        let (text, a11y) = query.single(app.world()).expect("transcript node spawned");
        assert_eq!(text.0, "Clinical summary: citation [1] verified.");
        assert_eq!(
            a11y.0.value(),
            Some("Clinical summary: citation [1] verified.")
        );
        assert_eq!(a11y.0.role(), accesskit::Role::Document);
        assert_eq!(
            app.world().resource::<TranscriptModel>().chunks_received(),
            2
        );
    }

    #[test]
    fn headless_probe_transitions_to_settled_without_dropping_transcript() {
        let mut app = build_headless_probe_app(BuildTarget::NativeLinux);
        app.update();

        app.world_mut()
            .resource_mut::<TranscriptModel>()
            .push_chunk("Final assessment ready.");
        app.world_mut()
            .resource_mut::<NextState<ProbeState>>()
            .set(ProbeState::Settled);
        app.update();

        assert_eq!(
            *app.world().resource::<State<ProbeState>>().get(),
            ProbeState::Settled
        );
        assert_eq!(
            app.world().resource::<TranscriptModel>().latest_line(),
            "Final assessment ready."
        );
    }
}
