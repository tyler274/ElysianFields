//! Deterministic screen-state serialization and process-death restoration.
//!
//! Secrets and authentication tokens are excluded from [`SavedScreenState`] by
//! construction; they live exclusively in [`crate::secure_storage`].

use std::fmt;
use std::str::Utf8Error;
use std::sync::{Arc, Mutex};

const MAGIC: [u8; 4] = *b"EFSS";
const FORMAT_VERSION: u8 = 1;
const MAX_RECORD_ID_LEN: usize = 128;

const ROUTE_TAG_HOME: u8 = 1;
const ROUTE_TAG_SIGN_IN: u8 = 2;
const ROUTE_TAG_CONVERSATION: u8 = 3;
const ROUTE_TAG_VITALS_OVERVIEW: u8 = 4;
const ROUTE_TAG_RECORD_DETAIL: u8 = 5;

const SIGN_IN_ORIGIN_USER: u8 = 1;
const SIGN_IN_ORIGIN_DEEP_LINK_COLD: u8 = 2;
const SIGN_IN_ORIGIN_DEEP_LINK_WARM: u8 = 3;

const OPTION_NONE: u8 = 0;
const OPTION_SOME: u8 = 1;

/// Whether a route or intent arrived during process startup or while warm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LaunchMode {
    /// App process started fresh from a closed or OS-terminated state.
    ColdStart,
    /// App process was already alive (foreground or background) and resumed.
    WarmResume,
}

/// How the user arrived at the sign-in screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SignInOrigin {
    /// Opened manually from the UI.
    UserInitiated,
    /// Opened via a validated sign-in deep link.
    DeepLink { launch_mode: LaunchMode },
}

/// Validated identifier for a clinical record or conversation thread.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RecordId(String);

/// Validation failure when parsing a [`RecordId`] at a boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordIdError {
    Empty,
    TooLong { len: usize, max: usize },
    InvalidCharacter { byte: u8 },
}

impl fmt::Display for RecordIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "record ID must not be empty"),
            Self::TooLong { len, max } => {
                write!(f, "record ID length {len} exceeds maximum {max}")
            }
            Self::InvalidCharacter { byte } => {
                write!(f, "record ID contains invalid byte 0x{byte:02x}")
            }
        }
    }
}

impl std::error::Error for RecordIdError {}

impl RecordId {
    /// Parses and validates a record identifier (`[A-Za-z0-9_-]`, 1..=128 bytes).
    pub fn parse(raw: impl AsRef<str>) -> Result<Self, RecordIdError> {
        let raw = raw.as_ref();
        if raw.is_empty() {
            return Err(RecordIdError::Empty);
        }
        if raw.len() > MAX_RECORD_ID_LEN {
            return Err(RecordIdError::TooLong {
                len: raw.len(),
                max: MAX_RECORD_ID_LEN,
            });
        }
        for &byte in raw.as_bytes() {
            let valid = byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_';
            if !valid {
                return Err(RecordIdError::InvalidCharacter { byte });
            }
        }
        Ok(Self(raw.to_owned()))
    }

    /// Returns the validated record ID string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Typed mobile screen routes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ScreenRoute {
    Home,
    SignIn { origin: SignInOrigin },
    Conversation { record_id: RecordId },
    VitalsOverview,
    RecordDetail { record_id: RecordId },
}

/// Deterministic snapshot of UI screen state persisted across OS process death.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedScreenState {
    pub route: ScreenRoute,
    pub draft_text: String,
    pub scroll_offset_px: u32,
    pub active_record_id: Option<RecordId>,
}

impl Default for SavedScreenState {
    fn default() -> Self {
        Self {
            route: ScreenRoute::Home,
            draft_text: String::new(),
            scroll_offset_px: 0,
            active_record_id: None,
        }
    }
}

/// Errors produced when encoding or decoding [`SavedScreenState`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateCodecError {
    FieldTooLarge {
        field: &'static str,
        len: usize,
    },
    TruncatedInput {
        expected_bytes: usize,
        remaining_bytes: usize,
    },
    InvalidMagic {
        found: [u8; 4],
    },
    UnsupportedVersion {
        version: u8,
    },
    ChecksumMismatch {
        expected: u32,
        actual: u32,
    },
    UnknownRouteTag {
        tag: u8,
    },
    UnknownSignInOriginTag {
        tag: u8,
    },
    UnknownOptionTag {
        tag: u8,
    },
    InvalidUtf8(Utf8Error),
    InvalidRecordId(RecordIdError),
    TrailingBytes {
        remaining: usize,
    },
}

impl fmt::Display for StateCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FieldTooLarge { field, len } => {
                write!(f, "field `{field}` length {len} exceeds u32 capacity")
            }
            Self::TruncatedInput {
                expected_bytes,
                remaining_bytes,
            } => write!(
                f,
                "truncated saved state: expected {expected_bytes} bytes, found {remaining_bytes}"
            ),
            Self::InvalidMagic { found } => {
                write!(f, "invalid saved state magic header: {found:?}")
            }
            Self::UnsupportedVersion { version } => {
                write!(f, "unsupported saved state schema version {version}")
            }
            Self::ChecksumMismatch { expected, actual } => write!(
                f,
                "saved state checksum mismatch: expected 0x{expected:08x}, got 0x{actual:08x}"
            ),
            Self::UnknownRouteTag { tag } => write!(f, "unknown screen route tag {tag}"),
            Self::UnknownSignInOriginTag { tag } => {
                write!(f, "unknown sign-in origin tag {tag}")
            }
            Self::UnknownOptionTag { tag } => write!(f, "unknown option tag {tag}"),
            Self::InvalidUtf8(err) => write!(f, "invalid UTF-8 in saved state: {err}"),
            Self::InvalidRecordId(err) => write!(f, "invalid record ID in saved state: {err}"),
            Self::TrailingBytes { remaining } => {
                write!(f, "saved state contains {remaining} trailing bytes")
            }
        }
    }
}

impl std::error::Error for StateCodecError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidUtf8(err) => Some(err),
            Self::InvalidRecordId(err) => Some(err),
            Self::FieldTooLarge { .. }
            | Self::TruncatedInput { .. }
            | Self::InvalidMagic { .. }
            | Self::UnsupportedVersion { .. }
            | Self::ChecksumMismatch { .. }
            | Self::UnknownRouteTag { .. }
            | Self::UnknownSignInOriginTag { .. }
            | Self::UnknownOptionTag { .. }
            | Self::TrailingBytes { .. } => None,
        }
    }
}

impl SavedScreenState {
    /// Serializes this screen state into a deterministic, checksummed binary frame.
    pub fn encode(&self) -> Result<Vec<u8>, StateCodecError> {
        let mut out = Vec::with_capacity(64 + self.draft_text.len());
        out.extend_from_slice(&MAGIC);
        out.push(FORMAT_VERSION);

        match &self.route {
            ScreenRoute::Home => out.push(ROUTE_TAG_HOME),
            ScreenRoute::SignIn { origin } => {
                out.push(ROUTE_TAG_SIGN_IN);
                let origin_tag = match origin {
                    SignInOrigin::UserInitiated => SIGN_IN_ORIGIN_USER,
                    SignInOrigin::DeepLink {
                        launch_mode: LaunchMode::ColdStart,
                    } => SIGN_IN_ORIGIN_DEEP_LINK_COLD,
                    SignInOrigin::DeepLink {
                        launch_mode: LaunchMode::WarmResume,
                    } => SIGN_IN_ORIGIN_DEEP_LINK_WARM,
                };
                out.push(origin_tag);
            }
            ScreenRoute::Conversation { record_id } => {
                out.push(ROUTE_TAG_CONVERSATION);
                write_str(&mut out, "route.record_id", record_id.as_str())?;
            }
            ScreenRoute::VitalsOverview => out.push(ROUTE_TAG_VITALS_OVERVIEW),
            ScreenRoute::RecordDetail { record_id } => {
                out.push(ROUTE_TAG_RECORD_DETAIL);
                write_str(&mut out, "route.record_id", record_id.as_str())?;
            }
        }

        write_str(&mut out, "draft_text", &self.draft_text)?;
        out.extend_from_slice(&self.scroll_offset_px.to_be_bytes());

        match &self.active_record_id {
            None => out.push(OPTION_NONE),
            Some(record_id) => {
                out.push(OPTION_SOME);
                write_str(&mut out, "active_record_id", record_id.as_str())?;
            }
        }

        let checksum = fnv1a_32(&out);
        out.extend_from_slice(&checksum.to_be_bytes());
        Ok(out)
    }

    /// Decodes and validates a binary frame produced by [`Self::encode`].
    pub fn decode(bytes: &[u8]) -> Result<Self, StateCodecError> {
        if bytes.len() < MAGIC.len() + 1 + 4 {
            return Err(StateCodecError::TruncatedInput {
                expected_bytes: MAGIC.len() + 1 + 4,
                remaining_bytes: bytes.len(),
            });
        }

        let (payload, checksum_slice) = bytes.split_at(bytes.len() - 4);
        let expected_checksum = u32::from_be_bytes([
            checksum_slice[0],
            checksum_slice[1],
            checksum_slice[2],
            checksum_slice[3],
        ]);
        let actual_checksum = fnv1a_32(payload);
        if expected_checksum != actual_checksum {
            return Err(StateCodecError::ChecksumMismatch {
                expected: expected_checksum,
                actual: actual_checksum,
            });
        }

        let mut cursor = ByteCursor::new(payload);
        let magic = cursor.read_array::<4>()?;
        if magic != MAGIC {
            return Err(StateCodecError::InvalidMagic { found: magic });
        }

        let version = cursor.read_u8()?;
        if version != FORMAT_VERSION {
            return Err(StateCodecError::UnsupportedVersion { version });
        }

        let route_tag = cursor.read_u8()?;
        let route = match route_tag {
            ROUTE_TAG_HOME => ScreenRoute::Home,
            ROUTE_TAG_SIGN_IN => {
                let origin_tag = cursor.read_u8()?;
                let origin = match origin_tag {
                    SIGN_IN_ORIGIN_USER => SignInOrigin::UserInitiated,
                    SIGN_IN_ORIGIN_DEEP_LINK_COLD => SignInOrigin::DeepLink {
                        launch_mode: LaunchMode::ColdStart,
                    },
                    SIGN_IN_ORIGIN_DEEP_LINK_WARM => SignInOrigin::DeepLink {
                        launch_mode: LaunchMode::WarmResume,
                    },
                    unknown => {
                        return Err(StateCodecError::UnknownSignInOriginTag { tag: unknown });
                    }
                };
                ScreenRoute::SignIn { origin }
            }
            ROUTE_TAG_CONVERSATION => {
                let raw = cursor.read_str()?;
                let record_id = RecordId::parse(raw).map_err(StateCodecError::InvalidRecordId)?;
                ScreenRoute::Conversation { record_id }
            }
            ROUTE_TAG_VITALS_OVERVIEW => ScreenRoute::VitalsOverview,
            ROUTE_TAG_RECORD_DETAIL => {
                let raw = cursor.read_str()?;
                let record_id = RecordId::parse(raw).map_err(StateCodecError::InvalidRecordId)?;
                ScreenRoute::RecordDetail { record_id }
            }
            unknown => return Err(StateCodecError::UnknownRouteTag { tag: unknown }),
        };

        let draft_text = cursor.read_str()?.to_owned();
        let scroll_offset_px = cursor.read_u32()?;

        let active_tag = cursor.read_u8()?;
        let active_record_id = match active_tag {
            OPTION_NONE => None,
            OPTION_SOME => {
                let raw = cursor.read_str()?;
                let id = RecordId::parse(raw).map_err(StateCodecError::InvalidRecordId)?;
                Some(id)
            }
            unknown => return Err(StateCodecError::UnknownOptionTag { tag: unknown }),
        };

        if !cursor.is_empty() {
            return Err(StateCodecError::TrailingBytes {
                remaining: cursor.remaining(),
            });
        }

        Ok(Self {
            route,
            draft_text,
            scroll_offset_px,
            active_record_id,
        })
    }
}

fn write_str(out: &mut Vec<u8>, field: &'static str, value: &str) -> Result<(), StateCodecError> {
    let len = u32::try_from(value.len()).map_err(|_overflow| StateCodecError::FieldTooLarge {
        field,
        len: value.len(),
    })?;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

fn fnv1a_32(bytes: &[u8]) -> u32 {
    const OFFSET_BASIS: u32 = 0x811c_9dc5;
    const PRIME: u32 = 0x0100_0193;
    let mut hash = OFFSET_BASIS;
    for &byte in bytes {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

struct ByteCursor<'a> {
    slice: &'a [u8],
}

impl<'a> ByteCursor<'a> {
    const fn new(slice: &'a [u8]) -> Self {
        Self { slice }
    }

    const fn is_empty(&self) -> bool {
        self.slice.is_empty()
    }

    const fn remaining(&self) -> usize {
        self.slice.len()
    }

    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], StateCodecError> {
        if self.slice.len() < len {
            return Err(StateCodecError::TruncatedInput {
                expected_bytes: len,
                remaining_bytes: self.slice.len(),
            });
        }
        let (head, tail) = self.slice.split_at(len);
        self.slice = tail;
        Ok(head)
    }

    fn read_u8(&mut self) -> Result<u8, StateCodecError> {
        let bytes = self.read_bytes(1)?;
        Ok(bytes[0])
    }

    fn read_u32(&mut self) -> Result<u32, StateCodecError> {
        let bytes = self.read_array::<4>()?;
        Ok(u32::from_be_bytes(bytes))
    }

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], StateCodecError> {
        let bytes = self.read_bytes(N)?;
        let mut array = [0u8; N];
        array.copy_from_slice(bytes);
        Ok(array)
    }

    fn read_str(&mut self) -> Result<&'a str, StateCodecError> {
        let len = self.read_u32()? as usize;
        let raw = self.read_bytes(len)?;
        std::str::from_utf8(raw).map_err(StateCodecError::InvalidUtf8)
    }
}

/// Errors produced by a platform state-persistence adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatePersistenceError {
    Codec(StateCodecError),
    StorageLockPoisoned,
}

impl fmt::Display for StatePersistenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codec(err) => write!(f, "state codec failure: {err}"),
            Self::StorageLockPoisoned => write!(f, "OS state bundle storage lock poisoned"),
        }
    }
}

impl std::error::Error for StatePersistenceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Codec(err) => Some(err),
            Self::StorageLockPoisoned => None,
        }
    }
}

/// Platform adapter contract for persisting [`SavedScreenState`] across OS process death
/// (Android `onSaveInstanceState` / iOS `NSUserActivity` state restoration).
pub trait StatePersistenceAdapter: Send + Sync {
    /// Persists an encoded snapshot to the OS-managed restoration bundle.
    fn save_state(&mut self, state: &SavedScreenState) -> Result<(), StatePersistenceError>;

    /// Loads and decodes the persisted snapshot, if present.
    fn load_state(&self) -> Result<Option<SavedScreenState>, StatePersistenceError>;

    /// Clears the persisted snapshot.
    fn clear_state(&mut self) -> Result<(), StatePersistenceError>;
}

/// Shared backing store representing an OS restoration bundle that outlives app process death.
#[derive(Debug, Clone, Default)]
pub struct OsStateBundleHandle {
    encoded_bytes: Arc<Mutex<Option<Vec<u8>>>>,
}

impl OsStateBundleHandle {
    /// Creates an empty OS restoration bundle handle.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a copy of the raw encoded bytes currently held by the OS bundle.
    pub fn snapshot_raw_bytes(&self) -> Result<Option<Vec<u8>>, StatePersistenceError> {
        let guard = self
            .encoded_bytes
            .lock()
            .map_err(|_poison| StatePersistenceError::StorageLockPoisoned)?;
        Ok(guard.clone())
    }
}

/// In-memory test adapter backed by [`OsStateBundleHandle`] so persisted screen
/// state survives simulated process kill and Bevy `App` recreation.
#[derive(Debug, Clone)]
pub struct InMemoryStateStore {
    handle: OsStateBundleHandle,
}

impl InMemoryStateStore {
    /// Connects a new process instance to an OS restoration bundle handle.
    #[must_use]
    pub fn attach(handle: OsStateBundleHandle) -> Self {
        Self { handle }
    }
}

impl StatePersistenceAdapter for InMemoryStateStore {
    fn save_state(&mut self, state: &SavedScreenState) -> Result<(), StatePersistenceError> {
        let encoded = state.encode().map_err(StatePersistenceError::Codec)?;
        let mut guard = self
            .handle
            .encoded_bytes
            .lock()
            .map_err(|_poison| StatePersistenceError::StorageLockPoisoned)?;
        *guard = Some(encoded);
        Ok(())
    }

    fn load_state(&self) -> Result<Option<SavedScreenState>, StatePersistenceError> {
        let guard = self
            .handle
            .encoded_bytes
            .lock()
            .map_err(|_poison| StatePersistenceError::StorageLockPoisoned)?;
        match guard.as_deref() {
            None => Ok(None),
            Some(bytes) => SavedScreenState::decode(bytes)
                .map(Some)
                .map_err(StatePersistenceError::Codec),
        }
    }

    fn clear_state(&mut self) -> Result<(), StatePersistenceError> {
        let mut guard = self
            .handle
            .encoded_bytes
            .lock()
            .map_err(|_poison| StatePersistenceError::StorageLockPoisoned)?;
        *guard = None;
        Ok(())
    }
}
