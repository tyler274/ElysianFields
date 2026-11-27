//! M1 web delivery qualification crate for Elysian Fields.
//!
//! Owns typed graphics backend detection and fallback diagnostics, WebGL2/WebGPU
//! context-loss recovery without page reload, strict `/elysianfields` subpath URL
//! resolution, and strict Content-Security-Policy verification.

pub mod context_loss;
pub mod csp;
pub mod graphics;
pub mod subpath;

use bevy::prelude::*;

use crate::context_loss::{ContextLossRecovery, RenderExecutionState};
use crate::csp::{CspSurface, CspValidationError, VerifiedCspPolicy, parse_and_verify_csp};
use crate::graphics::{BrowserGraphicsCapabilities, FallbackDiagnostic, GraphicsBackend};
use crate::subpath::SubpathMount;

/// Bevy resource reflecting whether GPU rendering is currently active or paused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Resource)]
pub struct RenderScheduleGate {
    pub state: RenderExecutionState,
}

/// Top-level startup readiness state for browser delivery.
#[derive(Debug, Clone, PartialEq, Eq, Resource)]
pub enum DeliveryReadiness {
    /// Graphics backend, strict CSP, and `/elysianfields` mount are all valid.
    Ready {
        backend: GraphicsBackend,
        policy: VerifiedCspPolicy,
        bootstrap_bundle_url: String,
    },
    /// Browser lacks WebGPU and WebGL2; UI renders a readable fallback diagnostic.
    UnsupportedGraphics {
        diagnostic: FallbackDiagnostic,
        policy: VerifiedCspPolicy,
    },
    /// Host CSP header violated the strict zero-inline / zero-unsafe-eval contract.
    PolicyRejected(CspValidationError),
}

/// Marker component attached to the root M1 delivery UI status node.
#[derive(Debug, Component)]
pub struct WebDeliveryStatusNode;

/// Configuration supplied to [`M1WebDeliveryPlugin`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebDeliveryConfig {
    pub mount: SubpathMount,
    pub csp_surface: CspSurface,
    pub csp_header: String,
    pub capabilities: BrowserGraphicsCapabilities,
}

impl WebDeliveryConfig {
    /// Builds a configuration using the canonical strict CSP for the requested surface.
    #[must_use]
    pub fn canonical(
        mount: SubpathMount,
        csp_surface: CspSurface,
        capabilities: BrowserGraphicsCapabilities,
    ) -> Self {
        Self {
            mount,
            csp_surface,
            csp_header: csp_surface.canonical_header().to_owned(),
            capabilities,
        }
    }
}

/// Bevy plugin wiring web delivery readiness, context-loss render gating, and UI status nodes.
pub struct M1WebDeliveryPlugin {
    pub config: WebDeliveryConfig,
}

impl M1WebDeliveryPlugin {
    #[must_use]
    pub const fn new(config: WebDeliveryConfig) -> Self {
        Self { config }
    }
}

impl Plugin for M1WebDeliveryPlugin {
    fn build(&self, app: &mut App) {
        let backend = GraphicsBackend::detect(self.config.capabilities);
        let readiness = match parse_and_verify_csp(&self.config.csp_header, self.config.csp_surface)
        {
            Err(error) => DeliveryReadiness::PolicyRejected(error),
            Ok(policy) => match &backend {
                GraphicsBackend::WebGpu | GraphicsBackend::WebGl2 => {
                    let bootstrap_bundle_url = self
                        .config
                        .mount
                        .resolve_bundle_url("pkg/m1_web_delivery_bg.wasm")
                        .unwrap_or_else(|_| {
                            format!(
                                "{}{}/pkg/m1_web_delivery_bg.wasm",
                                self.config.mount.origin().as_str(),
                                self.config.mount.prefix()
                            )
                        });
                    DeliveryReadiness::Ready {
                        backend: backend.clone(),
                        policy,
                        bootstrap_bundle_url,
                    }
                }
                GraphicsBackend::Unsupported(diagnostic) => {
                    DeliveryReadiness::UnsupportedGraphics {
                        diagnostic: diagnostic.clone(),
                        policy,
                    }
                }
            },
        };

        let initial_gate = match ContextLossRecovery::new(backend.clone()) {
            Ok(recovery) => {
                let state = recovery.render_state();
                app.insert_resource(recovery);
                RenderScheduleGate { state }
            }
            Err(_) => RenderScheduleGate {
                state: RenderExecutionState::PausedForRecovery,
            },
        };

        app.insert_resource(backend)
            .insert_resource(readiness)
            .insert_resource(initial_gate)
            .add_systems(Startup, spawn_delivery_status_node)
            .add_systems(Update, sync_render_schedule_gate);
    }
}

fn spawn_delivery_status_node(mut commands: Commands, readiness: Res<DeliveryReadiness>) {
    let status_text = match readiness.as_ref() {
        DeliveryReadiness::Ready {
            backend,
            bootstrap_bundle_url,
            ..
        } => format!("Elysian Fields M1 Web Delivery Ready ({backend}) @ {bootstrap_bundle_url}"),
        DeliveryReadiness::UnsupportedGraphics { diagnostic, .. } => diagnostic.render_message(),
        DeliveryReadiness::PolicyRejected(error) => error.to_string(),
    };

    commands.spawn((
        WebDeliveryStatusNode,
        Node {
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            ..default()
        },
        Text::new(status_text),
    ));
}

fn sync_render_schedule_gate(
    recovery: Option<Res<ContextLossRecovery>>,
    mut gate: ResMut<RenderScheduleGate>,
) {
    if let Some(recovery) = recovery {
        gate.state = recovery.render_state();
    }
}

/// Runs a headless Bevy bootstrap pass and returns `0` when the canonical delivery setup is ready.
#[must_use]
pub fn run_headless_delivery_probe(capabilities: BrowserGraphicsCapabilities) -> u32 {
    let Ok(mount) = SubpathMount::from_origin("https://elysian.local") else {
        return 1;
    };
    let config = WebDeliveryConfig::canonical(mount, CspSurface::WebApp, capabilities);

    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(M1WebDeliveryPlugin::new(config));
    app.update();

    match app.world().resource::<DeliveryReadiness>() {
        DeliveryReadiness::Ready { .. } => 0,
        DeliveryReadiness::UnsupportedGraphics { .. } => 2,
        DeliveryReadiness::PolicyRejected(_) => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_loss::{
        ContextRecoveryError, ContextRecoveryPhase, PreservedUiSnapshot, ReloadPolicy,
        SurfaceLossCause,
    };
    use crate::csp::{CspViolation, MV3_POPUP_CSP, WEB_APP_CSP};
    use crate::graphics::{UnsupportedGraphicsReason, WebGl2Availability, WebGpuAvailability};
    use crate::subpath::{MOUNT_PREFIX, SubpathError};

    fn sample_snapshot() -> PreservedUiSnapshot {
        PreservedUiSnapshot {
            active_route: "/elysianfields/clinic/thread/42".to_owned(),
            draft_composer_text: "Order follow-up metabolic panel in 2 weeks.".to_owned(),
            scroll_offset_px: 1_280,
            focused_control: Some("composer-input".to_owned()),
        }
    }

    #[test]
    fn graphics_backend_prefers_webgpu_falls_back_to_webgl2_and_diagnoses_unsupported() {
        assert_eq!(
            GraphicsBackend::detect(BrowserGraphicsCapabilities::webgpu_and_webgl2()),
            GraphicsBackend::WebGpu
        );
        assert_eq!(
            GraphicsBackend::detect(BrowserGraphicsCapabilities {
                webgpu: WebGpuAvailability::Available,
                webgl2: WebGl2Availability::Unavailable,
            }),
            GraphicsBackend::WebGpu
        );
        assert_eq!(
            GraphicsBackend::detect(BrowserGraphicsCapabilities::webgl2_only()),
            GraphicsBackend::WebGl2
        );

        let unsupported = GraphicsBackend::detect(BrowserGraphicsCapabilities::unsupported());
        assert!(!unsupported.is_renderable());
        let diagnostic = unsupported
            .fallback_diagnostic()
            .expect("unsupported backend must carry a diagnostic");
        assert_eq!(
            diagnostic.reason,
            UnsupportedGraphicsReason::MissingWebGpuAndWebGl2
        );
        let rendered = diagnostic.render_message();
        assert!(rendered.contains("WebGL2"));
        assert!(rendered.contains("Recommended actions:"));
    }

    #[test]
    fn context_loss_recovery_pauses_preserves_ui_and_restores_without_page_reload() {
        let mut recovery = ContextLossRecovery::new(GraphicsBackend::WebGl2)
            .expect("WebGL2 must initialize recovery state machine");
        assert_eq!(recovery.render_state(), RenderExecutionState::Active);
        assert_eq!(recovery.reload_policy(), ReloadPolicy::NoPageReload);
        assert_eq!(recovery.surface_generation(), 1);

        let snapshot = sample_snapshot();
        recovery
            .on_context_lost(SurfaceLossCause::WebGlContextLost, snapshot.clone())
            .expect("context loss from healthy state must succeed");
        assert_eq!(
            recovery.render_state(),
            RenderExecutionState::PausedForRecovery
        );
        assert_eq!(recovery.preserved_ui(), Some(&snapshot));

        // Duplicate loss while already suspended is rejected.
        assert_eq!(
            recovery.on_context_lost(SurfaceLossCause::WebGlContextLost, snapshot.clone()),
            Err(ContextRecoveryError::NotHealthyForContextLoss)
        );

        // Browser fires webglcontextrestored.
        recovery
            .on_context_restored(BrowserGraphicsCapabilities::webgl2_only())
            .expect("context restore while suspended must succeed");
        assert_eq!(
            recovery.render_state(),
            RenderExecutionState::PausedForRecovery
        );
        assert_eq!(recovery.surface_generation(), 2);

        // GPU pipelines rebuilt; UI state restored without page reload.
        let restored_ui = recovery
            .complete_surface_rebuild()
            .expect("surface rebuild must finish restoration");
        assert_eq!(restored_ui, snapshot);
        assert_eq!(recovery.render_state(), RenderExecutionState::Active);
        assert_eq!(recovery.reload_policy(), ReloadPolicy::NoPageReload);
        assert_eq!(recovery.preserved_ui(), None);
        assert_eq!(
            recovery.phase(),
            &ContextRecoveryPhase::Healthy {
                backend: GraphicsBackend::WebGl2,
                surface_generation: 2,
                total_recoveries: 1,
            }
        );
    }

    #[test]
    fn context_loss_can_downgrade_from_webgpu_to_webgl2_or_enter_unrecoverable_safely() {
        let mut recovery = ContextLossRecovery::new(GraphicsBackend::WebGpu)
            .expect("WebGPU must initialize recovery state machine");
        let snapshot = sample_snapshot();

        recovery
            .on_context_lost(SurfaceLossCause::WebGpuDeviceLost, snapshot.clone())
            .expect("WebGPU loss must pause rendering");

        // First restoration attempt fails because no backend is currently available.
        recovery
            .on_context_restored(BrowserGraphicsCapabilities::unsupported())
            .expect("restoration probe with no GPU enters unrecoverable safely");
        assert!(matches!(
            recovery.phase(),
            ContextRecoveryPhase::Unrecoverable { .. }
        ));
        assert_eq!(recovery.preserved_ui(), Some(&snapshot));
        assert_eq!(
            recovery.render_state(),
            RenderExecutionState::PausedForRecovery
        );

        // Later recovery succeeds on WebGL2 fallback without losing the UI snapshot.
        recovery
            .on_context_restored(BrowserGraphicsCapabilities::webgl2_only())
            .expect("retry from unrecoverable should succeed once WebGL2 returns");
        let restored_ui = recovery
            .complete_surface_rebuild()
            .expect("rebuild should return preserved snapshot");
        assert_eq!(restored_ui, snapshot);
        assert_eq!(
            recovery.phase(),
            &ContextRecoveryPhase::Healthy {
                backend: GraphicsBackend::WebGl2,
                surface_generation: 2,
                total_recoveries: 1,
            }
        );
    }

    #[test]
    fn subpath_mount_enforces_segment_boundaries_and_blocks_traversal() {
        let mount = SubpathMount::from_origin("https://clinic.example.org/")
            .expect("valid https origin must parse");
        assert_eq!(mount.origin().as_str(), "https://clinic.example.org");
        assert_eq!(mount.prefix(), MOUNT_PREFIX);

        assert!(SubpathMount::is_within_mount("/elysianfields"));
        assert!(SubpathMount::is_within_mount("/elysianfields/"));
        assert!(SubpathMount::is_within_mount("/elysianfields/clinic/inbox"));
        assert!(!SubpathMount::is_within_mount("/"));
        assert!(!SubpathMount::is_within_mount("/elysianfields-evil/app.js"));
        assert!(!SubpathMount::is_within_mount("/elysianfields2"));

        assert_eq!(
            mount.resolve_app_path("").expect("root route"),
            "/elysianfields/"
        );
        assert_eq!(
            mount
                .resolve_app_url("/clinic/inbox?tab=active")
                .expect("app route"),
            "https://clinic.example.org/elysianfields/clinic/inbox?tab=active"
        );
        assert_eq!(
            mount
                .resolve_asset_url("fonts/inter.woff2")
                .expect("asset url"),
            "https://clinic.example.org/elysianfields/assets/fonts/inter.woff2"
        );
        assert_eq!(
            mount
                .resolve_bundle_url("/elysianfields/pkg/app.wasm")
                .expect("already-mounted bundle path"),
            "https://clinic.example.org/elysianfields/pkg/app.wasm"
        );
        assert_eq!(
            mount
                .resolve_api_url("/v1/session/health")
                .expect("api url"),
            "https://clinic.example.org/elysianfields/api/v1/session/health"
        );
        assert_eq!(
            SubpathMount::strip_mount_prefix("/elysianfields/clinic/inbox?q=1")
                .expect("strip valid mounted path"),
            "/clinic/inbox?q=1"
        );

        assert_eq!(
            SubpathMount::strip_mount_prefix("/elysianfields-other/clinic"),
            Err(SubpathError::OutsideSubpathMount {
                path: "/elysianfields-other/clinic".to_owned(),
            })
        );
        assert_eq!(
            mount.resolve_asset_url("../secrets.txt"),
            Err(SubpathError::PathTraversalDetected)
        );
        assert_eq!(
            mount.resolve_asset_url("fonts/%2e%2e/keys.json"),
            Err(SubpathError::PathTraversalDetected)
        );
        assert_eq!(
            mount.resolve_api_url("https://attacker.example/steal"),
            Err(SubpathError::ExternalUrlNotAllowed)
        );
        assert_eq!(
            mount.resolve_api_url("//attacker.example/steal"),
            Err(SubpathError::ExternalUrlNotAllowed)
        );
        assert_eq!(
            SubpathMount::from_origin("https://clinic.example.org/extra"),
            Err(SubpathError::OriginContainsPathOrQuery)
        );
    }

    #[test]
    fn csp_verifier_accepts_canonical_policies_and_rejects_unsafe_directives() {
        let web_policy = parse_and_verify_csp(WEB_APP_CSP, CspSurface::WebApp)
            .expect("canonical web CSP must pass");
        assert_eq!(web_policy.surface(), CspSurface::WebApp);
        assert!(web_policy.directive("frame-ancestors").is_some());

        let mv3_policy = parse_and_verify_csp(MV3_POPUP_CSP, CspSurface::Mv3ExtensionPopup)
            .expect("canonical MV3 CSP must pass");
        assert_eq!(mv3_policy.surface(), CspSurface::Mv3ExtensionPopup);

        let unsafe_inline_err = parse_and_verify_csp(
            "default-src 'self'; script-src 'self' 'wasm-unsafe-eval' 'unsafe-inline'; style-src 'self'; frame-ancestors 'none'",
            CspSurface::WebApp,
        )
        .expect_err("unsafe-inline must be rejected");
        assert!(
            unsafe_inline_err
                .violations
                .contains(&CspViolation::ForbiddenSource {
                    directive: "script-src".to_owned(),
                    source: "'unsafe-inline'".to_owned(),
                })
        );

        let unsafe_eval_err = parse_and_verify_csp(
            "default-src 'self'; script-src 'self' 'unsafe-eval'; style-src 'self'; frame-ancestors 'none'",
            CspSurface::WebApp,
        )
        .expect_err("unsafe-eval must be rejected");
        assert!(
            unsafe_eval_err
                .violations
                .contains(&CspViolation::ForbiddenSource {
                    directive: "script-src".to_owned(),
                    source: "'unsafe-eval'".to_owned(),
                })
        );
        assert!(
            unsafe_eval_err
                .violations
                .contains(&CspViolation::MissingRequiredSource {
                    directive: "script-src",
                    source: "'wasm-unsafe-eval'",
                })
        );

        let missing_frame_ancestors = parse_and_verify_csp(
            "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'",
            CspSurface::WebApp,
        )
        .expect_err("missing frame-ancestors must fail");
        assert!(missing_frame_ancestors.violations.contains(
            &CspViolation::MissingRequiredDirective {
                directive: "frame-ancestors",
            }
        ));
    }

    #[test]
    fn headless_bevy_app_runs_context_loss_recovery_and_render_gating() {
        let mount =
            SubpathMount::from_origin("https://clinic.example.org").expect("origin should parse");
        let config = WebDeliveryConfig::canonical(
            mount,
            CspSurface::WebApp,
            BrowserGraphicsCapabilities::webgl2_only(),
        );

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(M1WebDeliveryPlugin::new(config));
        app.update();

        assert_eq!(
            app.world().resource::<RenderScheduleGate>().state,
            RenderExecutionState::Active
        );
        assert!(matches!(
            app.world().resource::<DeliveryReadiness>(),
            DeliveryReadiness::Ready {
                backend: GraphicsBackend::WebGl2,
                ..
            }
        ));

        let mut query = app
            .world_mut()
            .query_filtered::<&Text, With<WebDeliveryStatusNode>>();
        let status_texts: Vec<&str> = query
            .iter(app.world())
            .map(|text| text.0.as_str())
            .collect();
        assert_eq!(status_texts.len(), 1);
        assert!(status_texts[0].contains("/elysianfields/pkg/m1_web_delivery_bg.wasm"));

        // Trigger context loss inside ECS world and verify render gate pauses on next update.
        let snapshot = sample_snapshot();
        app.world_mut()
            .resource_mut::<ContextLossRecovery>()
            .on_context_lost(SurfaceLossCause::WebGlContextLost, snapshot.clone())
            .expect("context loss should pause recovery tracker");
        app.update();
        assert_eq!(
            app.world().resource::<RenderScheduleGate>().state,
            RenderExecutionState::PausedForRecovery
        );

        // Restore surface and verify render gate resumes cleanly without page reload.
        {
            let mut recovery = app.world_mut().resource_mut::<ContextLossRecovery>();
            recovery
                .on_context_restored(BrowserGraphicsCapabilities::webgl2_only())
                .expect("context restored should enter restoring");
            let restored = recovery
                .complete_surface_rebuild()
                .expect("surface rebuild should restore snapshot");
            assert_eq!(restored, snapshot);
        }
        app.update();
        assert_eq!(
            app.world().resource::<RenderScheduleGate>().state,
            RenderExecutionState::Active
        );
    }

    #[test]
    fn headless_bevy_app_surfaces_unsupported_gpu_fallback_diagnostic() {
        let mount =
            SubpathMount::from_origin("https://clinic.example.org").expect("origin should parse");
        let config = WebDeliveryConfig::canonical(
            mount,
            CspSurface::WebApp,
            BrowserGraphicsCapabilities::unsupported(),
        );

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(M1WebDeliveryPlugin::new(config));
        app.update();

        assert_eq!(
            app.world().resource::<RenderScheduleGate>().state,
            RenderExecutionState::PausedForRecovery
        );
        assert!(matches!(
            app.world().resource::<DeliveryReadiness>(),
            DeliveryReadiness::UnsupportedGraphics { .. }
        ));

        let mut query = app
            .world_mut()
            .query_filtered::<&Text, With<WebDeliveryStatusNode>>();
        let status = query
            .single(app.world())
            .expect("fallback status node should exist");
        assert!(
            status
                .0
                .contains("Hardware-accelerated browser graphics unavailable")
        );
        assert_eq!(
            run_headless_delivery_probe(BrowserGraphicsCapabilities::unsupported()),
            2
        );
        assert_eq!(
            run_headless_delivery_probe(BrowserGraphicsCapabilities::webgl2_only()),
            0
        );
    }
}
