//! M1 Bevy 0.20.1 mobile lifecycle prototype and platform adapter contracts
//! for Android and iOS.
//!
//! Domain state machines and platform adapter traits remain strictly independent
//! of rendering entities and view lifetimes; [`plugin::MobileLifecyclePrototypePlugin`]
//! binds them into a headless Bevy 0.20.1 application for deterministic verification.

pub mod deep_link;
pub mod health;
pub mod lifecycle;
pub mod notifications;
pub mod plugin;
pub mod secure_storage;
pub mod state_restore;
pub mod viewport;

pub use deep_link::{
    DeepLinkError, DeepLinkIntent, DeepLinkResolution, DeepLinkRoute, SignInToken,
    TokenValidationError,
};
pub use health::{
    DeniedRecoveryAction, HealthAccessAdapter, HealthCoordinator, HealthError, HealthMetricKind,
    HealthPermissionState, HealthPlatform, HealthSample, HealthSampleValue, HealthUiOutcome,
    MockHealthAdapter, MockPromptDecision, SampleProvenance,
};
pub use lifecycle::{
    AppLifecycleEvent, AppLifecycleState, LifecycleController, LifecycleTransitionError,
    RenderTickOutcome,
};
pub use notifications::{
    NotificationDestination, NotificationError, NotificationId, NotificationIdError,
    NotificationKind, NotificationPayload, NotificationRouteResult, NotificationRouter,
};
pub use plugin::{
    AuthSessionSlot, MobileAppShellResource, MobileCommand, MobileCommandQueue,
    MobileLifecyclePrototypePlugin, MobileLifecycleResource, MobileRuntimeError,
    PlatformAdaptersResource,
};
pub use secure_storage::{
    PersistentMockKeychain, PersistentVaultHandle, SecretValue, SecureBackend, SecureKey,
    SecureStorageAdapter, SecureStorageError, VaultLockState,
};
pub use state_restore::{
    InMemoryStateStore, LaunchMode, OsStateBundleHandle, RecordId, RecordIdError, SavedScreenState,
    ScreenRoute, SignInOrigin, StateCodecError, StatePersistenceAdapter, StatePersistenceError,
};
pub use viewport::{
    DisplayCutout, LayoutAxis, LayoutColumns, LayoutRect, Orientation, SafeAreaInsets,
    ViewportInsets, ViewportLayoutError, ViewportSpec, WindowPresentation,
};
