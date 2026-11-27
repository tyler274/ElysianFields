use bevy::prelude::*;
use m1_mobile_lifecycle::{
    AppLifecycleEvent, AppLifecycleState, AuthSessionSlot, DeepLinkError, DeepLinkRoute,
    DeniedRecoveryAction, DisplayCutout, HealthCoordinator, HealthError, HealthMetricKind,
    HealthPermissionState, HealthPlatform, HealthSample, HealthSampleValue, HealthUiOutcome,
    InMemoryStateStore, LaunchMode, LayoutAxis, LayoutColumns, LayoutRect, LifecycleController,
    MobileAppShellResource, MobileCommand, MobileCommandQueue, MobileLifecyclePrototypePlugin,
    MobileLifecycleResource, MobileRuntimeError, MockHealthAdapter, MockPromptDecision,
    NotificationError, NotificationKind, NotificationPayload, Orientation, OsStateBundleHandle,
    PersistentMockKeychain, PersistentVaultHandle, PlatformAdaptersResource, RecordId,
    RenderTickOutcome, SafeAreaInsets, SampleProvenance, SavedScreenState, ScreenRoute,
    SecretValue, SecureBackend, SecureKey, SecureStorageAdapter, SecureStorageError, SignInOrigin,
    SignInToken, StateCodecError, VaultLockState, ViewportInsets, ViewportLayoutError,
    ViewportSpec, WindowPresentation,
};

fn build_headless_mobile_app(
    lifecycle: MobileLifecycleResource,
    state_bundle: OsStateBundleHandle,
    health: MockHealthAdapter,
    backend: SecureBackend,
    vault: PersistentVaultHandle,
) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, MobileLifecyclePrototypePlugin))
        .insert_resource(lifecycle)
        .insert_resource(PlatformAdaptersResource::new(
            Box::new(InMemoryStateStore::attach(state_bundle)),
            Box::new(health),
            Box::new(PersistentMockKeychain::attach(backend, vault)),
        ));
    app
}

fn enqueue(app: &mut App, command: MobileCommand) {
    app.world_mut()
        .resource_mut::<MobileCommandQueue>()
        .push(command);
}

#[test]
fn background_pause_suspends_render_loop_with_zero_frames_drawn_while_hidden() {
    let mut app = build_headless_mobile_app(
        MobileLifecycleResource::new_foreground(),
        OsStateBundleHandle::new(),
        MockHealthAdapter::new(HealthPlatform::AndroidHealthConnect),
        SecureBackend::AndroidKeystore,
        PersistentVaultHandle::new(),
    );

    // Render 3 foreground frames.
    for expected in 1..=3 {
        app.update();
        let lifecycle = app.world().resource::<MobileLifecycleResource>();
        assert_eq!(
            lifecycle.controller().state(),
            AppLifecycleState::Foreground
        );
        assert!(lifecycle.controller().should_draw());
        assert_eq!(lifecycle.controller().frames_rendered_total(), expected);
        assert_eq!(
            lifecycle.last_tick_outcome(),
            Some(RenderTickOutcome::Rendered {
                frame_index: expected
            })
        );
    }

    // Enter background and run 10 headless ticks: zero frames may be drawn.
    enqueue(
        &mut app,
        MobileCommand::Lifecycle(AppLifecycleEvent::EnterBackground),
    );
    for expected_suppressed in 1..=10 {
        app.update();
        let lifecycle = app.world().resource::<MobileLifecycleResource>();
        assert_eq!(
            lifecycle.controller().state(),
            AppLifecycleState::BackgroundPaused
        );
        assert!(!lifecycle.controller().should_draw());
        assert_eq!(lifecycle.controller().frames_rendered_total(), 3);
        assert_eq!(lifecycle.controller().frames_rendered_while_hidden(), 0);
        assert_eq!(
            lifecycle.controller().suppressed_ticks(),
            expected_suppressed
        );
        assert_eq!(
            lifecycle.last_tick_outcome(),
            Some(RenderTickOutcome::Suppressed {
                state: AppLifecycleState::BackgroundPaused
            })
        );
    }

    // Resume foreground: rendering resumes cleanly from frame 4.
    enqueue(
        &mut app,
        MobileCommand::Lifecycle(AppLifecycleEvent::ResumeForeground),
    );
    app.update();
    let lifecycle = app.world().resource::<MobileLifecycleResource>();
    assert_eq!(
        lifecycle.controller().state(),
        AppLifecycleState::Foreground
    );
    assert!(lifecycle.controller().should_draw());
    assert_eq!(lifecycle.controller().frames_rendered_total(), 4);
    assert_eq!(lifecycle.controller().frames_rendered_while_hidden(), 0);
}

#[test]
fn invalid_lifecycle_transition_is_rejected_and_keeps_render_suspended() {
    let mut controller = LifecycleController::new_terminated();
    assert!(!controller.should_draw());

    let err = controller
        .transition(AppLifecycleEvent::ResumeForeground)
        .expect_err("cannot warm-resume a terminated process without cold launch");
    assert_eq!(err.current, AppLifecycleState::ProcessTerminated);
    assert_eq!(err.event, AppLifecycleEvent::ResumeForeground);
    assert!(!controller.should_draw());
    assert_eq!(
        controller.tick_render_frame(),
        RenderTickOutcome::Suppressed {
            state: AppLifecycleState::ProcessTerminated
        }
    );
    assert_eq!(controller.frames_rendered_total(), 0);
}

#[test]
fn os_process_kill_and_cold_relaunch_restores_exact_screen_state() {
    let os_bundle = OsStateBundleHandle::new();
    let vault = PersistentVaultHandle::new();
    let record_id = RecordId::parse("rec_synthetic_42").expect("valid record id");

    // Process 1: edit draft, scroll, background, and suffer OS process kill.
    {
        let mut app = build_headless_mobile_app(
            MobileLifecycleResource::new_foreground(),
            os_bundle.clone(),
            MockHealthAdapter::new(HealthPlatform::IosHealthKit),
            SecureBackend::IosKeychain,
            vault.clone(),
        );

        let payload = NotificationPayload::parse_from_bridge(
            "notif-001",
            NotificationKind::Conversation,
            Some(record_id.as_str()),
            LaunchMode::WarmResume,
        )
        .expect("valid notification payload");

        enqueue(&mut app, MobileCommand::TapNotification(payload));
        enqueue(
            &mut app,
            MobileCommand::UpdateDraft {
                text: "Synthetic clinical draft surviving OS termination".to_owned(),
            },
        );
        enqueue(&mut app, MobileCommand::UpdateScroll { offset_px: 1420 });
        enqueue(
            &mut app,
            MobileCommand::Lifecycle(AppLifecycleEvent::EnterBackground),
        );
        app.update();

        enqueue(
            &mut app,
            MobileCommand::Lifecycle(AppLifecycleEvent::OsProcessTerminate),
        );
        app.update();

        let lifecycle = app.world().resource::<MobileLifecycleResource>();
        assert_eq!(
            lifecycle.controller().state(),
            AppLifecycleState::ProcessTerminated
        );
        assert!(!lifecycle.controller().should_draw());
        assert!(
            app.world()
                .resource::<MobileAppShellResource>()
                .errors
                .is_empty()
        );
    }

    // Verify deterministic serialization bytes exist in the OS restoration bundle.
    let persisted_bytes = os_bundle
        .snapshot_raw_bytes()
        .expect("bundle readable")
        .expect("bundle populated before process death");

    // Process 2: brand new Bevy App cold-launches and restores state identically.
    let mut relaunched_app = build_headless_mobile_app(
        MobileLifecycleResource::new_terminated(),
        os_bundle,
        MockHealthAdapter::new(HealthPlatform::IosHealthKit),
        SecureBackend::IosKeychain,
        vault,
    );
    enqueue(
        &mut relaunched_app,
        MobileCommand::Lifecycle(AppLifecycleEvent::ColdLaunch),
    );
    relaunched_app.update();

    let shell = relaunched_app.world().resource::<MobileAppShellResource>();
    assert!(shell.errors.is_empty());
    assert_eq!(
        shell.screen_state,
        SavedScreenState {
            route: ScreenRoute::Conversation {
                record_id: record_id.clone()
            },
            draft_text: "Synthetic clinical draft surviving OS termination".to_owned(),
            scroll_offset_px: 1420,
            active_record_id: Some(record_id),
        }
    );
    assert_eq!(
        shell.screen_state.encode().expect("deterministic encode"),
        persisted_bytes
    );
}

#[test]
fn saved_screen_state_codec_rejects_corruption_and_truncation() {
    let sample = SavedScreenState {
        route: ScreenRoute::RecordDetail {
            record_id: RecordId::parse("rec-99").expect("valid record id"),
        },
        draft_text: "Follow-up summary".to_owned(),
        scroll_offset_px: 320,
        active_record_id: Some(RecordId::parse("rec-99").expect("valid record id")),
    };

    let encoded_a = sample.encode().expect("encode 1");
    let encoded_b = sample.encode().expect("encode 2");
    assert_eq!(encoded_a, encoded_b);
    assert_eq!(
        SavedScreenState::decode(&encoded_a).expect("decode"),
        sample
    );

    // Flip a byte in the payload -> checksum mismatch.
    let mut corrupted = encoded_a.clone();
    corrupted[6] ^= 0xFF;
    assert!(matches!(
        SavedScreenState::decode(&corrupted),
        Err(StateCodecError::ChecksumMismatch { .. })
    ));

    // Truncate input -> truncated error.
    assert!(matches!(
        SavedScreenState::decode(&encoded_a[..4]),
        Err(StateCodecError::TruncatedInput { .. })
    ));
}

#[test]
fn deep_link_cold_and_warm_sign_in_routes_valid_links_and_rejects_untrusted_uris() {
    let os_bundle = OsStateBundleHandle::new();
    let vault = PersistentVaultHandle::new();
    let cold_token = "syn_cold_token_0123456789abcdef";
    let warm_token = "syn_warm_token_fedcba9876543210";

    let mut app = build_headless_mobile_app(
        MobileLifecycleResource::new_foreground(),
        os_bundle.clone(),
        MockHealthAdapter::new(HealthPlatform::AndroidHealthConnect),
        SecureBackend::AndroidKeystore,
        vault.clone(),
    );

    // Cold-start custom scheme sign-in deep link.
    enqueue(
        &mut app,
        MobileCommand::OpenDeepLink {
            raw_uri: format!("curie://sign-in?token={cold_token}"),
            launch_mode: LaunchMode::ColdStart,
        },
    );
    app.update();

    {
        let shell = app.world().resource::<MobileAppShellResource>();
        assert!(shell.errors.is_empty());
        assert_eq!(
            shell.screen_state.route,
            ScreenRoute::SignIn {
                origin: SignInOrigin::DeepLink {
                    launch_mode: LaunchMode::ColdStart
                }
            }
        );
        assert_eq!(
            shell.auth_slot,
            AuthSessionSlot::SignInTokenStoredInKeychain
        );

        // Secret must NEVER appear in Debug output, diagnostics, or saved screen state.
        let debug_dump = format!("{shell:?}");
        assert!(!debug_dump.contains(cold_token));
        let encoded_state = shell.screen_state.encode().expect("encode state");
        assert!(!String::from_utf8_lossy(&encoded_state).contains(cold_token));

        let adapters = app.world().resource::<PlatformAdaptersResource>();
        let stored = adapters
            .secure_storage()
            .load(SecureKey::SignInExchangeToken)
            .expect("keychain load")
            .expect("token stored");
        assert_eq!(stored.as_utf8().expect("utf8"), cold_token);
    }

    // Warm-resume HTTPS App Link / Universal Link under `/elysianfields/`.
    enqueue(
        &mut app,
        MobileCommand::OpenDeepLink {
            raw_uri: format!("https://dabney.moe/elysianfields/sign-in?token={warm_token}"),
            launch_mode: LaunchMode::WarmResume,
        },
    );
    app.update();

    {
        let shell = app.world().resource::<MobileAppShellResource>();
        assert!(shell.errors.is_empty());
        assert_eq!(
            shell.screen_state.route,
            ScreenRoute::SignIn {
                origin: SignInOrigin::DeepLink {
                    launch_mode: LaunchMode::WarmResume
                }
            }
        );
        let adapters = app.world().resource::<PlatformAdaptersResource>();
        let stored = adapters
            .secure_storage()
            .load(SecureKey::SignInExchangeToken)
            .expect("keychain load")
            .expect("token updated");
        assert_eq!(stored.as_utf8().expect("utf8"), warm_token);
    }

    // Malformed / untrusted URIs must all be rejected cleanly.
    let invalid_cases = [
        "http://dabney.moe/elysianfields/sign-in?token=syn_token_1234567890",
        "javascript:alert(1)",
        "https://evil.example/elysianfields/sign-in?token=syn_token_1234567890",
        "https://dabney.moe/sign-in?token=syn_token_1234567890",
        "https://dabney.moe/elysianfields/../sign-in?token=syn_token_1234567890",
        "curie://sign-in?token=too_short",
        "curie://sign-in?token=syn_token_1234567890&token=syn_token_duplicate_1234",
        "curie://unknown-route",
    ];
    for uri in invalid_cases {
        assert!(
            DeepLinkRoute::parse(uri).is_err(),
            "expected deep link `{uri}` to be rejected"
        );
    }

    // Dispatching an untrusted link through Bevy records a typed error and preserves prior route.
    enqueue(
        &mut app,
        MobileCommand::OpenDeepLink {
            raw_uri: "http://dabney.moe/elysianfields/sign-in?token=leaked_secret_token_99999"
                .to_owned(),
            launch_mode: LaunchMode::WarmResume,
        },
    );
    app.update();
    let shell = app.world().resource::<MobileAppShellResource>();
    assert!(matches!(
        shell.errors.last(),
        Some(MobileRuntimeError::DeepLink(
            DeepLinkError::UntrustedScheme { .. }
        ))
    ));
    let debug_dump = format!("{shell:?}");
    assert!(!debug_dump.contains("leaked_secret_token_99999"));
}

#[test]
fn closed_app_notification_tap_opens_target_screen_directly() {
    let mut app = build_headless_mobile_app(
        MobileLifecycleResource::new_terminated(),
        OsStateBundleHandle::new(),
        MockHealthAdapter::new(HealthPlatform::IosHealthKit),
        SecureBackend::IosKeychain,
        PersistentVaultHandle::new(),
    );

    let payload = NotificationPayload::parse_from_bridge(
        "apns-msg-77",
        NotificationKind::RecordDetail,
        Some("record_lab_500"),
        LaunchMode::ColdStart,
    )
    .expect("valid notification payload");

    enqueue(&mut app, MobileCommand::TapNotification(payload));
    app.update();

    let lifecycle = app.world().resource::<MobileLifecycleResource>();
    assert_eq!(
        lifecycle.controller().state(),
        AppLifecycleState::Foreground
    );
    assert_eq!(lifecycle.controller().frames_rendered_total(), 1);

    let shell = app.world().resource::<MobileAppShellResource>();
    let expected_id = RecordId::parse("record_lab_500").expect("valid record id");
    assert!(shell.errors.is_empty());
    assert_eq!(
        shell.screen_state.route,
        ScreenRoute::RecordDetail {
            record_id: expected_id.clone()
        }
    );
    assert_eq!(shell.screen_state.active_record_id, Some(expected_id));

    // Malformed notification payloads fail with typed boundary errors.
    assert!(matches!(
        NotificationPayload::parse_from_bridge(
            "apns-missing-id",
            NotificationKind::Conversation,
            None,
            LaunchMode::ColdStart
        ),
        Err(NotificationError::MissingTargetRecordId {
            kind: NotificationKind::Conversation
        })
    ));
}

#[test]
fn health_permissions_handle_grant_denial_without_loop_and_unavailability() {
    // 1. Android Health Connect: HeartRate granted, Steps denied.
    let mut health = MockHealthAdapter::new(HealthPlatform::AndroidHealthConnect);
    health.set_prompt_decision(HealthMetricKind::HeartRateBpm, MockPromptDecision::Grant);
    health.set_prompt_decision(HealthMetricKind::StepsCount, MockPromptDecision::Deny);
    let synthetic_hr = HealthSample {
        metric: HealthMetricKind::HeartRateBpm,
        value: HealthSampleValue::HeartRateBpm(72),
        recorded_at_epoch_secs: 1_760_000_000,
        source_platform: HealthPlatform::AndroidHealthConnect,
        provenance: SampleProvenance::SyntheticFixture,
    };
    health.insert_synthetic_sample(synthetic_hr.clone());

    let mut app = build_headless_mobile_app(
        MobileLifecycleResource::new_foreground(),
        OsStateBundleHandle::new(),
        health,
        SecureBackend::AndroidKeystore,
        PersistentVaultHandle::new(),
    );

    // Grant path: reads authorized synthetic sample.
    enqueue(
        &mut app,
        MobileCommand::RequestHealthMetric {
            metric: HealthMetricKind::HeartRateBpm,
        },
    );
    app.update();
    {
        let shell = app.world().resource::<MobileAppShellResource>();
        assert!(shell.errors.is_empty());
        assert_eq!(
            shell.last_health_outcome,
            Some(HealthUiOutcome::AuthorizedSample {
                sample: synthetic_hr
            })
        );
    }

    // Denial path: returns graceful non-blocking UI notice and never loops prompts.
    for _ in 0..3 {
        enqueue(
            &mut app,
            MobileCommand::RequestHealthMetric {
                metric: HealthMetricKind::StepsCount,
            },
        );
        app.update();
        let shell = app.world().resource::<MobileAppShellResource>();
        assert!(shell.errors.is_empty());
        assert_eq!(
            shell.last_health_outcome,
            Some(HealthUiOutcome::PermissionDeniedNotice {
                metric: HealthMetricKind::StepsCount,
                recovery: DeniedRecoveryAction::OpenOsHealthSettings,
            })
        );
    }

    // Direct read on a denied metric fails closed.
    let adapters = app.world().resource::<PlatformAdaptersResource>();
    assert_eq!(
        adapters
            .health_adapter()
            .permission_state(HealthMetricKind::StepsCount),
        HealthPermissionState::Denied
    );
    assert!(matches!(
        adapters
            .health_adapter()
            .read_latest_sample(HealthMetricKind::StepsCount),
        Err(HealthError::PermissionRequired {
            metric: HealthMetricKind::StepsCount,
            state: HealthPermissionState::Denied,
        })
    ));

    // 2. iOS HealthKit unavailable (e.g., unsupported device).
    let mut unavailable_app = build_headless_mobile_app(
        MobileLifecycleResource::new_foreground(),
        OsStateBundleHandle::new(),
        MockHealthAdapter::unavailable(HealthPlatform::IosHealthKit),
        SecureBackend::IosKeychain,
        PersistentVaultHandle::new(),
    );
    enqueue(
        &mut unavailable_app,
        MobileCommand::RequestHealthMetric {
            metric: HealthMetricKind::BloodPressureMmHg,
        },
    );
    unavailable_app.update();
    let shell = unavailable_app.world().resource::<MobileAppShellResource>();
    assert_eq!(
        shell.last_health_outcome,
        Some(HealthUiOutcome::PlatformUnavailableNotice {
            platform: HealthPlatform::IosHealthKit
        })
    );
}

#[test]
fn secure_storage_persists_across_restart_and_never_leaks_plaintext() {
    let vault = PersistentVaultHandle::new();
    let secret_plaintext = "synthetic_refresh_secret_9f8e7d6c5b4a";

    // Process 1 writes to Keychain.
    {
        let mut keychain =
            PersistentMockKeychain::attach(SecureBackend::IosKeychain, vault.clone());
        let secret = SecretValue::from_utf8(secret_plaintext).expect("valid secret");
        assert_eq!(format!("{secret:?}"), "SecretValue([REDACTED])");
        keychain
            .store(SecureKey::SessionRefreshToken, &secret)
            .expect("store secret");
    }

    // Process 2 reads back identically after restart.
    let mut relaunched_keychain =
        PersistentMockKeychain::attach(SecureBackend::IosKeychain, vault.clone());
    let loaded = relaunched_keychain
        .load(SecureKey::SessionRefreshToken)
        .expect("load secret")
        .expect("secret survived restart");
    assert_eq!(loaded.as_utf8().expect("utf8"), secret_plaintext);

    // Locked device reports typed error rather than empty data.
    vault
        .set_lock_state(VaultLockState::Locked)
        .expect("lock vault");
    assert_eq!(
        relaunched_keychain.load(SecureKey::SessionRefreshToken),
        Err(SecureStorageError::DeviceLocked {
            backend: SecureBackend::IosKeychain
        })
    );

    // Unlock and delete cleanly.
    vault
        .set_lock_state(VaultLockState::Unlocked)
        .expect("unlock vault");
    relaunched_keychain
        .delete(SecureKey::SessionRefreshToken)
        .expect("delete secret");
    assert_eq!(
        relaunched_keychain
            .load(SecureKey::SessionRefreshToken)
            .expect("load after delete"),
        None
    );

    // Verify SignInToken also redacts Debug output.
    let token = SignInToken::parse("syn_token_redaction_check_0001").expect("valid token");
    assert_eq!(format!("{token:?}"), "SignInToken([REDACTED])");
}

#[test]
fn viewport_insets_handle_rotation_split_screen_cutouts_and_soft_keyboard() {
    let mut app = build_headless_mobile_app(
        MobileLifecycleResource::new_foreground(),
        OsStateBundleHandle::new(),
        MockHealthAdapter::new(HealthPlatform::IosHealthKit),
        SecureBackend::IosKeychain,
        PersistentVaultHandle::new(),
    );

    // 1. Portrait with Dynamic Island + home bar.
    enqueue(
        &mut app,
        MobileCommand::UpdateViewport(ViewportSpec {
            width_px: 393,
            height_px: 852,
            scale_factor_milli: 3000,
            orientation: Orientation::Portrait,
            presentation: WindowPresentation::FullScreen,
            system_bars: SafeAreaInsets {
                top: 44,
                right: 0,
                bottom: 34,
                left: 0,
            },
            cutout: DisplayCutout::DynamicIsland { top_inset_px: 59 },
            soft_keyboard_height_px: 0,
        }),
    );
    app.update();
    {
        let viewport = app.world().resource::<MobileAppShellResource>().viewport;
        assert_eq!(viewport.effective_insets.top, 59);
        assert_eq!(viewport.effective_insets.bottom, 34);
        assert_eq!(
            viewport.safe_content_rect,
            LayoutRect {
                x: 0,
                y: 59,
                width: 393,
                height: 759,
            }
        );
        assert_eq!(viewport.layout_columns, LayoutColumns::SinglePane);
    }

    // 2. Portrait with soft keyboard raised (312 px from bottom).
    enqueue(
        &mut app,
        MobileCommand::UpdateViewport(ViewportSpec {
            width_px: 393,
            height_px: 852,
            scale_factor_milli: 3000,
            orientation: Orientation::Portrait,
            presentation: WindowPresentation::FullScreen,
            system_bars: SafeAreaInsets {
                top: 44,
                right: 0,
                bottom: 34,
                left: 0,
            },
            cutout: DisplayCutout::DynamicIsland { top_inset_px: 59 },
            soft_keyboard_height_px: 312,
        }),
    );
    app.update();
    {
        let viewport = app.world().resource::<MobileAppShellResource>().viewport;
        assert_eq!(viewport.effective_insets.bottom, 312);
        assert_eq!(viewport.safe_content_rect.height, 852 - 59 - 312);
    }

    // 3. Landscape rotation with side cutout -> widens into SplitPane.
    enqueue(
        &mut app,
        MobileCommand::UpdateViewport(ViewportSpec {
            width_px: 852,
            height_px: 393,
            scale_factor_milli: 3000,
            orientation: Orientation::LandscapeLeft,
            presentation: WindowPresentation::FullScreen,
            system_bars: SafeAreaInsets {
                top: 0,
                right: 20,
                bottom: 21,
                left: 20,
            },
            cutout: DisplayCutout::LandscapeSideCutout {
                left_px: 59,
                right_px: 0,
            },
            soft_keyboard_height_px: 0,
        }),
    );
    app.update();
    {
        let viewport = app.world().resource::<MobileAppShellResource>().viewport;
        assert_eq!(viewport.orientation, Orientation::LandscapeLeft);
        assert_eq!(viewport.effective_insets.left, 59);
        assert_eq!(viewport.effective_insets.right, 20);
        assert_eq!(
            viewport.safe_content_rect,
            LayoutRect {
                x: 59,
                y: 0,
                width: 773,
                height: 372,
            }
        );
        assert_eq!(viewport.layout_columns, LayoutColumns::SplitPane);
    }

    // 4. Android split-screen resize narrows to SinglePane.
    enqueue(
        &mut app,
        MobileCommand::UpdateViewport(ViewportSpec {
            width_px: 412,
            height_px: 430,
            scale_factor_milli: 2625,
            orientation: Orientation::Portrait,
            presentation: WindowPresentation::SplitScreen {
                pane_fraction_percent: 50,
            },
            system_bars: SafeAreaInsets {
                top: 24,
                right: 0,
                bottom: 16,
                left: 0,
            },
            cutout: DisplayCutout::None,
            soft_keyboard_height_px: 0,
        }),
    );
    app.update();
    {
        let viewport = app.world().resource::<MobileAppShellResource>().viewport;
        assert_eq!(
            viewport.presentation,
            WindowPresentation::SplitScreen {
                pane_fraction_percent: 50
            }
        );
        assert_eq!(viewport.layout_columns, LayoutColumns::SinglePane);
        assert_eq!(
            viewport.safe_content_rect,
            LayoutRect {
                x: 0,
                y: 24,
                width: 412,
                height: 390,
            }
        );
    }

    // 5. Invalid inset overflow fails with typed error.
    let overflow = ViewportInsets::compute(&ViewportSpec {
        width_px: 200,
        height_px: 400,
        scale_factor_milli: 2000,
        orientation: Orientation::Portrait,
        presentation: WindowPresentation::FullScreen,
        system_bars: SafeAreaInsets {
            top: 24,
            right: 120,
            bottom: 24,
            left: 100,
        },
        cutout: DisplayCutout::None,
        soft_keyboard_height_px: 0,
    });
    assert_eq!(
        overflow,
        Err(ViewportLayoutError::InsetsExceedViewport {
            axis: LayoutAxis::Horizontal,
            viewport_px: 200,
            inset_sum_px: 220,
        })
    );
}

#[test]
fn ios_healthkit_grant_no_data_and_denial_prompt_count_invariant() {
    let mut health = MockHealthAdapter::new(HealthPlatform::IosHealthKit);
    health.set_prompt_decision(HealthMetricKind::HeartRateBpm, MockPromptDecision::Grant);
    health.set_prompt_decision(HealthMetricKind::StepsCount, MockPromptDecision::Grant);
    health.set_prompt_decision(
        HealthMetricKind::SleepDurationMinutes,
        MockPromptDecision::Deny,
    );

    let synthetic_hr = HealthSample {
        metric: HealthMetricKind::HeartRateBpm,
        value: HealthSampleValue::HeartRateBpm(68),
        recorded_at_epoch_secs: 1_760_000_120,
        source_platform: HealthPlatform::IosHealthKit,
        provenance: SampleProvenance::SyntheticFixture,
    };
    health.insert_synthetic_sample(synthetic_hr.clone());

    let granted_outcome =
        HealthCoordinator::sync_metric(&mut health, HealthMetricKind::HeartRateBpm)
            .expect("sync heart rate");
    assert_eq!(
        granted_outcome,
        HealthUiOutcome::AuthorizedSample {
            sample: synthetic_hr
        }
    );

    // Granted metric with zero samples returns AuthorizedNoData (models HealthKit read-empty semantics).
    let no_data_outcome = HealthCoordinator::sync_metric(&mut health, HealthMetricKind::StepsCount)
        .expect("sync steps");
    assert_eq!(
        no_data_outcome,
        HealthUiOutcome::AuthorizedNoData {
            metric: HealthMetricKind::StepsCount
        }
    );

    // Denied metric invokes OS prompt strictly once across repeated retries.
    let prompts_before_denial = health.prompt_count();
    for _ in 0..4 {
        let denied =
            HealthCoordinator::sync_metric(&mut health, HealthMetricKind::SleepDurationMinutes)
                .expect("sync denied sleep");
        assert_eq!(
            denied,
            HealthUiOutcome::PermissionDeniedNotice {
                metric: HealthMetricKind::SleepDurationMinutes,
                recovery: DeniedRecoveryAction::OpenOsHealthSettings,
            }
        );
    }
    assert_eq!(health.prompt_count(), prompts_before_denial + 1);
}

#[test]
fn android_keystore_persists_multiple_keys_across_process_restart() {
    let vault = PersistentVaultHandle::new();

    // Empty secret must be rejected at the boundary.
    assert_eq!(
        SecretValue::from_bytes(Vec::new()),
        Err(SecureStorageError::EmptySecret)
    );

    {
        let mut keystore =
            PersistentMockKeychain::attach(SecureBackend::AndroidKeystore, vault.clone());
        let refresh = SecretValue::from_utf8("syn_android_refresh_000111222333").expect("secret");
        let binding = SecretValue::from_bytes(vec![0xAA, 0xBB, 0xCC, 0xDD]).expect("secret");
        keystore
            .store(SecureKey::SessionRefreshToken, &refresh)
            .expect("store refresh");
        keystore
            .store(SecureKey::DeviceBindingKey, &binding)
            .expect("store binding");
    }

    let relaunched = PersistentMockKeychain::attach(SecureBackend::AndroidKeystore, vault);
    assert_eq!(
        relaunched
            .load(SecureKey::SessionRefreshToken)
            .expect("load refresh")
            .expect("present")
            .as_utf8()
            .expect("utf8"),
        "syn_android_refresh_000111222333"
    );
    assert_eq!(
        relaunched
            .load(SecureKey::DeviceBindingKey)
            .expect("load binding")
            .expect("present")
            .as_bytes(),
        &[0xAA, 0xBB, 0xCC, 0xDD]
    );
}

#[test]
fn deep_link_record_and_vitals_routes_and_warm_notification_routing() {
    let mut app = build_headless_mobile_app(
        MobileLifecycleResource::new_foreground(),
        OsStateBundleHandle::new(),
        MockHealthAdapter::new(HealthPlatform::AndroidHealthConnect),
        SecureBackend::AndroidKeystore,
        PersistentVaultHandle::new(),
    );

    enqueue(
        &mut app,
        MobileCommand::OpenDeepLink {
            raw_uri: "curie://record?id=rec_panel_07".to_owned(),
            launch_mode: LaunchMode::WarmResume,
        },
    );
    app.update();
    {
        let shell = app.world().resource::<MobileAppShellResource>();
        let expected = RecordId::parse("rec_panel_07").expect("valid record id");
        assert!(shell.errors.is_empty());
        assert_eq!(
            shell.screen_state.route,
            ScreenRoute::RecordDetail {
                record_id: expected.clone()
            }
        );
        assert_eq!(shell.screen_state.active_record_id, Some(expected));
    }

    enqueue(
        &mut app,
        MobileCommand::OpenDeepLink {
            raw_uri: "https://dabney.moe/elysianfields/vitals".to_owned(),
            launch_mode: LaunchMode::WarmResume,
        },
    );
    app.update();
    {
        let shell = app.world().resource::<MobileAppShellResource>();
        assert!(shell.errors.is_empty());
        assert_eq!(shell.screen_state.route, ScreenRoute::VitalsOverview);
    }

    let warm_notif = NotificationPayload::parse_from_bridge(
        "fcm-vitals-88",
        NotificationKind::VitalsOverview,
        None,
        LaunchMode::WarmResume,
    )
    .expect("valid warm notification");
    enqueue(&mut app, MobileCommand::TapNotification(warm_notif));
    app.update();
    {
        let shell = app.world().resource::<MobileAppShellResource>();
        assert!(shell.errors.is_empty());
        assert_eq!(shell.screen_state.route, ScreenRoute::VitalsOverview);
        assert_eq!(shell.screen_state.active_record_id, None);
    }
}
