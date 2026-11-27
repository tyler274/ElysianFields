//! Typed health data adapter contract for iOS HealthKit and Android Health Connect.
//!
//! Handles both permission grant (reading an authorized synthetic vitals sample)
//! and permission denial (non-blocking UI state without crashing or looping).

use std::collections::BTreeMap;
use std::fmt;

/// Underlying OS health framework.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HealthPlatform {
    IosHealthKit,
    AndroidHealthConnect,
}

/// Supported health metric categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HealthMetricKind {
    HeartRateBpm,
    StepsCount,
    BloodPressureMmHg,
    SleepDurationMinutes,
}

/// Explicit permission state for a requested health metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HealthPermissionState {
    /// User has not yet been prompted for this metric.
    NotDetermined,
    /// User granted read access for this metric.
    Granted,
    /// User denied access; UI must remain non-blocking and avoid prompt loops.
    Denied,
    /// Platform health store is unavailable on this device or OS configuration.
    Unavailable,
}

/// Provenance tag keeping test fixtures explicitly synthetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SampleProvenance {
    /// 100% synthetic fixture data used in M1 qualification tests.
    SyntheticFixture,
    /// Sample originating from an OS health store adapter on a physical device.
    PlatformHealthStore,
}

/// Strongly typed vitals measurement value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthSampleValue {
    HeartRateBpm(u16),
    StepsCount(u32),
    BloodPressureMmHg { systolic: u16, diastolic: u16 },
    SleepDurationMinutes(u32),
}

/// Single authorized vitals sample returned by a [`HealthAccessAdapter`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthSample {
    pub metric: HealthMetricKind,
    pub value: HealthSampleValue,
    pub recorded_at_epoch_secs: u64,
    pub source_platform: HealthPlatform,
    pub provenance: SampleProvenance,
}

/// Recovery guidance shown to the user when health permission is denied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeniedRecoveryAction {
    /// Direct user to OS Settings (Health Connect / iOS Settings -> Health).
    OpenOsHealthSettings,
    /// Offer manual vitals entry without blocking navigation.
    ContinueWithoutHealthSync,
}

/// Non-blocking UI state derived from a health permission + read workflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthUiOutcome {
    /// Permission has not been requested yet.
    ReadyToRequest { metric: HealthMetricKind },
    /// Permission granted and authorized sample returned.
    AuthorizedSample { sample: HealthSample },
    /// Permission granted, but no sample exists yet for this metric.
    AuthorizedNoData { metric: HealthMetricKind },
    /// Permission denied; UI remains responsive and does not loop prompts.
    PermissionDeniedNotice {
        metric: HealthMetricKind,
        recovery: DeniedRecoveryAction,
    },
    /// Health store is unavailable on this device.
    PlatformUnavailableNotice { platform: HealthPlatform },
}

/// Errors produced by a [`HealthAccessAdapter`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthError {
    PermissionRequired {
        metric: HealthMetricKind,
        state: HealthPermissionState,
    },
    PlatformUnavailable {
        platform: HealthPlatform,
    },
    AdapterFailure {
        platform: HealthPlatform,
        detail: String,
    },
}

impl fmt::Display for HealthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PermissionRequired { metric, state } => write!(
                f,
                "cannot read health metric {metric:?} while permission state is {state:?}"
            ),
            Self::PlatformUnavailable { platform } => {
                write!(
                    f,
                    "health platform {platform:?} is unavailable on this device"
                )
            }
            Self::AdapterFailure { platform, detail } => {
                write!(f, "health adapter failure on {platform:?}: {detail}")
            }
        }
    }
}

impl std::error::Error for HealthError {}

/// Platform adapter trait bridging iOS HealthKit and Android Health Connect.
pub trait HealthAccessAdapter: Send + Sync {
    /// Identifies which OS health framework backs this adapter.
    fn platform(&self) -> HealthPlatform;

    /// Queries the current permission state without showing a system prompt.
    fn permission_state(&self, metric: HealthMetricKind) -> HealthPermissionState;

    /// Requests permission from the OS. Calling this when already `Denied` must
    /// return `Ok(HealthPermissionState::Denied)` without re-prompting in a loop.
    fn request_permission(
        &mut self,
        metric: HealthMetricKind,
    ) -> Result<HealthPermissionState, HealthError>;

    /// Reads the latest sample for `metric`. Fails closed with
    /// [`HealthError::PermissionRequired`] if permission is not [`HealthPermissionState::Granted`].
    fn read_latest_sample(
        &self,
        metric: HealthMetricKind,
    ) -> Result<Option<HealthSample>, HealthError>;
}

/// Single workflow owner coordinating permission requests and sample reads.
pub struct HealthCoordinator;

impl HealthCoordinator {
    /// Ensures permission is evaluated once and returns a non-blocking [`HealthUiOutcome`].
    ///
    /// - If `NotDetermined`, triggers a single OS permission request.
    /// - If `Granted`, reads the latest authorized sample.
    /// - If `Denied`, returns [`HealthUiOutcome::PermissionDeniedNotice`] without re-prompting.
    /// - If `Unavailable`, returns [`HealthUiOutcome::PlatformUnavailableNotice`].
    pub fn sync_metric(
        adapter: &mut dyn HealthAccessAdapter,
        metric: HealthMetricKind,
    ) -> Result<HealthUiOutcome, HealthError> {
        let state = match adapter.permission_state(metric) {
            HealthPermissionState::NotDetermined => adapter.request_permission(metric)?,
            existing => existing,
        };

        match state {
            HealthPermissionState::NotDetermined => Ok(HealthUiOutcome::ReadyToRequest { metric }),
            HealthPermissionState::Granted => match adapter.read_latest_sample(metric)? {
                Some(sample) => Ok(HealthUiOutcome::AuthorizedSample { sample }),
                None => Ok(HealthUiOutcome::AuthorizedNoData { metric }),
            },
            HealthPermissionState::Denied => Ok(HealthUiOutcome::PermissionDeniedNotice {
                metric,
                recovery: DeniedRecoveryAction::OpenOsHealthSettings,
            }),
            HealthPermissionState::Unavailable => Ok(HealthUiOutcome::PlatformUnavailableNotice {
                platform: adapter.platform(),
            }),
        }
    }
}

/// Scripted decision a mock OS permission dialog returns when prompted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockPromptDecision {
    Grant,
    Deny,
}

/// Deterministic mock [`HealthAccessAdapter`] for headless M1 qualification tests.
#[derive(Debug, Clone)]
pub struct MockHealthAdapter {
    platform: HealthPlatform,
    availability: AdapterAvailability,
    permissions: BTreeMap<HealthMetricKind, HealthPermissionState>,
    prompt_decisions: BTreeMap<HealthMetricKind, MockPromptDecision>,
    synthetic_samples: BTreeMap<HealthMetricKind, HealthSample>,
    prompt_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdapterAvailability {
    Available,
    Unavailable,
}

impl MockHealthAdapter {
    /// Creates an available mock health adapter for the specified platform.
    #[must_use]
    pub fn new(platform: HealthPlatform) -> Self {
        Self {
            platform,
            availability: AdapterAvailability::Available,
            permissions: BTreeMap::new(),
            prompt_decisions: BTreeMap::new(),
            synthetic_samples: BTreeMap::new(),
            prompt_count: 0,
        }
    }

    /// Creates a mock adapter representing a device without HealthKit / Health Connect.
    #[must_use]
    pub fn unavailable(platform: HealthPlatform) -> Self {
        Self {
            platform,
            availability: AdapterAvailability::Unavailable,
            permissions: BTreeMap::new(),
            prompt_decisions: BTreeMap::new(),
            synthetic_samples: BTreeMap::new(),
            prompt_count: 0,
        }
    }

    /// Configures how the mock OS dialog responds when `metric` is requested.
    pub fn set_prompt_decision(&mut self, metric: HealthMetricKind, decision: MockPromptDecision) {
        self.prompt_decisions.insert(metric, decision);
    }

    /// Seeds a 100% synthetic vitals sample to return once permission is granted.
    pub fn insert_synthetic_sample(&mut self, sample: HealthSample) {
        self.synthetic_samples.insert(sample.metric, sample);
    }

    /// Returns how many times the OS permission dialog was actually invoked.
    #[must_use]
    pub const fn prompt_count(&self) -> u32 {
        self.prompt_count
    }
}

impl HealthAccessAdapter for MockHealthAdapter {
    fn platform(&self) -> HealthPlatform {
        self.platform
    }

    fn permission_state(&self, metric: HealthMetricKind) -> HealthPermissionState {
        if self.availability == AdapterAvailability::Unavailable {
            return HealthPermissionState::Unavailable;
        }
        self.permissions
            .get(&metric)
            .copied()
            .unwrap_or(HealthPermissionState::NotDetermined)
    }

    fn request_permission(
        &mut self,
        metric: HealthMetricKind,
    ) -> Result<HealthPermissionState, HealthError> {
        if self.availability == AdapterAvailability::Unavailable {
            return Ok(HealthPermissionState::Unavailable);
        }

        let current = self.permission_state(metric);
        match current {
            HealthPermissionState::Granted => Ok(HealthPermissionState::Granted),
            HealthPermissionState::Denied => {
                // Never re-prompt in a loop once denied.
                Ok(HealthPermissionState::Denied)
            }
            HealthPermissionState::Unavailable => Ok(HealthPermissionState::Unavailable),
            HealthPermissionState::NotDetermined => {
                self.prompt_count = self.prompt_count.saturating_add(1);
                let next = match self.prompt_decisions.get(&metric).copied() {
                    Some(MockPromptDecision::Grant) => HealthPermissionState::Granted,
                    Some(MockPromptDecision::Deny) | None => HealthPermissionState::Denied,
                };
                self.permissions.insert(metric, next);
                Ok(next)
            }
        }
    }

    fn read_latest_sample(
        &self,
        metric: HealthMetricKind,
    ) -> Result<Option<HealthSample>, HealthError> {
        let state = self.permission_state(metric);
        match state {
            HealthPermissionState::Granted => Ok(self.synthetic_samples.get(&metric).cloned()),
            HealthPermissionState::Unavailable => Err(HealthError::PlatformUnavailable {
                platform: self.platform,
            }),
            HealthPermissionState::NotDetermined | HealthPermissionState::Denied => {
                Err(HealthError::PermissionRequired { metric, state })
            }
        }
    }
}
