//! Typed secure credential storage adapter contract for iOS Keychain and Android Keystore.
//!
//! Secrets are wrapped in [`SecretValue`], which redacts [`fmt::Debug`], omits
//! [`fmt::Display`], and zeroes its buffer on [`Drop`].

use std::collections::BTreeMap;
use std::fmt;
use std::str::Utf8Error;
use std::sync::{Arc, Mutex};

/// Underlying OS hardware-backed credential store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SecureBackend {
    IosKeychain,
    AndroidKeystore,
}

/// Finite, typed credential slot keys (never arbitrary raw strings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SecureKey {
    SessionRefreshToken,
    SignInExchangeToken,
    DeviceBindingKey,
}

/// Opaque secret bytes that redact `Debug` output and scrub memory on drop.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue {
    bytes: Vec<u8>,
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretValue([REDACTED])")
    }
}

impl Drop for SecretValue {
    fn drop(&mut self) {
        self.bytes.fill(0);
        self.bytes.clear();
    }
}

/// Errors produced by [`SecureStorageAdapter`] implementations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecureStorageError {
    EmptySecret,
    DeviceLocked { backend: SecureBackend },
    BackendUnavailable { backend: SecureBackend },
    StorageLockPoisoned,
}

impl fmt::Display for SecureStorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySecret => write!(f, "refusing to store an empty secret"),
            Self::DeviceLocked { backend } => {
                write!(
                    f,
                    "secure storage {backend:?} is inaccessible while device is locked"
                )
            }
            Self::BackendUnavailable { backend } => {
                write!(f, "secure storage backend {backend:?} is unavailable")
            }
            Self::StorageLockPoisoned => write!(f, "secure storage vault lock poisoned"),
        }
    }
}

impl std::error::Error for SecureStorageError {}

impl SecretValue {
    /// Wraps non-empty secret bytes.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, SecureStorageError> {
        if bytes.is_empty() {
            return Err(SecureStorageError::EmptySecret);
        }
        Ok(Self { bytes })
    }

    /// Wraps a non-empty UTF-8 secret string.
    pub fn from_utf8(secret: impl Into<String>) -> Result<Self, SecureStorageError> {
        Self::from_bytes(secret.into().into_bytes())
    }

    /// Exposes the raw secret bytes strictly at the boundary that requires them.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Exposes the secret as UTF-8 strictly at the boundary that requires it.
    pub fn as_utf8(&self) -> Result<&str, Utf8Error> {
        std::str::from_utf8(&self.bytes)
    }
}

/// Platform adapter trait bridging iOS Keychain and Android Keystore.
pub trait SecureStorageAdapter: Send + Sync {
    /// Returns which OS secure storage backend owns this adapter.
    fn backend(&self) -> SecureBackend;

    /// Stores or overwrites a secret under `key`.
    fn store(&mut self, key: SecureKey, value: &SecretValue) -> Result<(), SecureStorageError>;

    /// Retrieves the secret stored under `key`, or `Ok(None)` if absent.
    fn load(&self, key: SecureKey) -> Result<Option<SecretValue>, SecureStorageError>;

    /// Deletes any secret stored under `key`.
    fn delete(&mut self, key: SecureKey) -> Result<(), SecureStorageError>;
}

/// Lock state of a mock OS credential vault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultLockState {
    Unlocked,
    Locked,
}

#[derive(Debug)]
struct VaultInner {
    entries: BTreeMap<SecureKey, Vec<u8>>,
    lock_state: VaultLockState,
}

/// Persistent handle modeling an OS Keychain / Keystore vault that outlives app restarts.
#[derive(Debug, Clone)]
pub struct PersistentVaultHandle {
    inner: Arc<Mutex<VaultInner>>,
}

impl Default for PersistentVaultHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl PersistentVaultHandle {
    /// Creates an unlocked, empty persistent vault handle.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(VaultInner {
                entries: BTreeMap::new(),
                lock_state: VaultLockState::Unlocked,
            })),
        }
    }

    /// Transitions the mock vault lock state (for testing locked-device error handling).
    pub fn set_lock_state(&self, state: VaultLockState) -> Result<(), SecureStorageError> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_poison| SecureStorageError::StorageLockPoisoned)?;
        guard.lock_state = state;
        Ok(())
    }
}

/// Deterministic mock [`SecureStorageAdapter`] backed by [`PersistentVaultHandle`].
#[derive(Debug, Clone)]
pub struct PersistentMockKeychain {
    backend: SecureBackend,
    vault: PersistentVaultHandle,
}

impl PersistentMockKeychain {
    /// Attaches an app process instance to a persistent OS keychain/keystore vault.
    #[must_use]
    pub fn attach(backend: SecureBackend, vault: PersistentVaultHandle) -> Self {
        Self { backend, vault }
    }
}

impl SecureStorageAdapter for PersistentMockKeychain {
    fn backend(&self) -> SecureBackend {
        self.backend
    }

    fn store(&mut self, key: SecureKey, value: &SecretValue) -> Result<(), SecureStorageError> {
        let mut guard = self
            .vault
            .inner
            .lock()
            .map_err(|_poison| SecureStorageError::StorageLockPoisoned)?;
        if guard.lock_state == VaultLockState::Locked {
            return Err(SecureStorageError::DeviceLocked {
                backend: self.backend,
            });
        }
        guard.entries.insert(key, value.as_bytes().to_vec());
        Ok(())
    }

    fn load(&self, key: SecureKey) -> Result<Option<SecretValue>, SecureStorageError> {
        let guard = self
            .vault
            .inner
            .lock()
            .map_err(|_poison| SecureStorageError::StorageLockPoisoned)?;
        if guard.lock_state == VaultLockState::Locked {
            return Err(SecureStorageError::DeviceLocked {
                backend: self.backend,
            });
        }
        match guard.entries.get(&key) {
            None => Ok(None),
            Some(bytes) => SecretValue::from_bytes(bytes.clone()).map(Some),
        }
    }

    fn delete(&mut self, key: SecureKey) -> Result<(), SecureStorageError> {
        let mut guard = self
            .vault
            .inner
            .lock()
            .map_err(|_poison| SecureStorageError::StorageLockPoisoned)?;
        if guard.lock_state == VaultLockState::Locked {
            return Err(SecureStorageError::DeviceLocked {
                backend: self.backend,
            });
        }
        if let Some(mut old) = guard.entries.remove(&key) {
            old.fill(0);
        }
        Ok(())
    }
}
