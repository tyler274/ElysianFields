//! Strict single-owner platform input adapter boundary.
//!
//! Platform bridges (browser hidden `<textarea>` IME proxy, Android `InputConnection`,
//! iOS `UITextInput`, desktop `winit` IME) emit typed [`TextInputCommand`]s into
//! [`ComposerDocument`] and receive read-only [`AdapterReadback`] geometry and focus
//! snapshots. By construction, [`AdapterReadback`] never carries a duplicate editable
//! copy of the document text.

use std::collections::VecDeque;

use crate::composer::{
    CaretRect, ComposerDocument, CursorState, DocumentRevision, FocusState, ImeComposition,
    SoftKeyboardState, TextInputCommand,
};
use crate::locale::TextDirection;

/// Canonical M1 target platforms enumerated in `specs/m1-platforms.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlatformTarget {
    BrowserWasmDesktop,
    BrowserWasmMobile,
    DesktopLinux,
    DesktopMacos,
    DesktopWindows,
    AndroidNative,
    IosNative,
    ChromeExtensionPopup,
    HostedServices,
    StaticPublicWeb,
}

/// Narrow OS/browser text input and IME integration mechanism per platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AdapterMechanism {
    /// Native desktop `winit` IME (`WindowEvent::Ime`) + OS clipboard bridge.
    WinitNativeIme,
    /// Browser Wasm hidden/positioned `<textarea>` IME, clipboard, and dictation bridge.
    HiddenTextareaImeBridge,
    /// Android `InputConnection` + `WindowInsets` soft-keyboard bridge.
    AndroidInputConnectionBridge,
    /// iOS `UITextInput` + keyboard frame notification bridge.
    IosUiTextInputBridge,
    /// Semantic HTML surface (MV3 popup D2 fallback or static public web).
    SemanticHtmlSurface,
    /// Headless backend service without interactive text input.
    NotApplicableHeadless,
}

impl PlatformTarget {
    /// Every platform in the M1 canonical platform matrix.
    pub const ALL: [PlatformTarget; 10] = [
        Self::BrowserWasmDesktop,
        Self::BrowserWasmMobile,
        Self::DesktopLinux,
        Self::DesktopMacos,
        Self::DesktopWindows,
        Self::AndroidNative,
        Self::IosNative,
        Self::ChromeExtensionPopup,
        Self::HostedServices,
        Self::StaticPublicWeb,
    ];

    /// Canonical requirement-ledger key matching `specs/m1-platforms.md`.
    #[must_use]
    pub const fn ledger_key(self) -> &'static str {
        match self {
            Self::BrowserWasmDesktop => "browser-wasm-desktop",
            Self::BrowserWasmMobile => "browser-wasm-mobile",
            Self::DesktopLinux => "desktop-linux",
            Self::DesktopMacos => "desktop-macos",
            Self::DesktopWindows => "desktop-windows",
            Self::AndroidNative => "android-native",
            Self::IosNative => "ios-native",
            Self::ChromeExtensionPopup => "chrome-extension-popup",
            Self::HostedServices => "hosted-services",
            Self::StaticPublicWeb => "static-public-web",
        }
    }

    /// Designated narrow single-owner text/IME adapter mechanism for this target.
    #[must_use]
    pub const fn adapter_mechanism(self) -> AdapterMechanism {
        match self {
            Self::BrowserWasmDesktop | Self::BrowserWasmMobile => {
                AdapterMechanism::HiddenTextareaImeBridge
            }
            Self::DesktopLinux | Self::DesktopMacos | Self::DesktopWindows => {
                AdapterMechanism::WinitNativeIme
            }
            Self::AndroidNative => AdapterMechanism::AndroidInputConnectionBridge,
            Self::IosNative => AdapterMechanism::IosUiTextInputBridge,
            Self::ChromeExtensionPopup | Self::StaticPublicWeb => {
                AdapterMechanism::SemanticHtmlSurface
            }
            Self::HostedServices => AdapterMechanism::NotApplicableHeadless,
        }
    }
}

/// Whether the composer currently has a non-empty text selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionPresence {
    None,
    Active,
}

/// Whether an OS IME preedit composition session is currently active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositionPresence {
    Idle,
    Active,
}

/// Read-only projection returned to platform adapters for OS candidate-window placement
/// and soft-keyboard synchronization.
///
/// Intentionally omits any copy of the committed document text so platform adapters
/// cannot diverge into a second editable text store.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AdapterReadback {
    pub revision: DocumentRevision,
    pub focus: FocusState,
    pub direction: TextDirection,
    pub caret_rect: CaretRect,
    pub preedit_anchor: Option<CaretRect>,
    pub soft_keyboard: SoftKeyboardState,
    pub selection: SelectionPresence,
    pub composition: CompositionPresence,
}

impl AdapterReadback {
    /// Derive the read-only adapter projection directly from the single-owner document.
    #[must_use]
    pub fn from_document(doc: &ComposerDocument) -> Self {
        let caret_rect = doc.caret_rect();
        let (composition, preedit_anchor) = match doc.ime() {
            ImeComposition::Idle => (CompositionPresence::Idle, None),
            ImeComposition::Composing {
                candidate_anchor, ..
            } => (CompositionPresence::Active, Some(*candidate_anchor)),
        };
        let selection = match doc.cursor() {
            CursorState::Caret(_) => SelectionPresence::None,
            CursorState::Selection { .. } => SelectionPresence::Active,
        };
        Self {
            revision: doc.revision(),
            focus: doc.focus(),
            direction: doc.effective_direction(),
            caret_rect,
            preedit_anchor,
            soft_keyboard: doc.soft_keyboard(),
            selection,
            composition,
        }
    }
}

/// Contract implemented by every narrow platform text-input adapter.
pub trait PlatformInputAdapter {
    fn mechanism(&self) -> AdapterMechanism;
    fn drain_commands(&mut self) -> Vec<TextInputCommand>;
    fn sync_readback(&mut self, readback: AdapterReadback);
}

/// Deterministic in-memory adapter for headless verification and integration tests.
#[derive(Debug, Default)]
pub struct HeadlessSimulatedAdapter {
    mechanism: Option<AdapterMechanism>,
    outbound: VecDeque<TextInputCommand>,
    last_readback: Option<AdapterReadback>,
    sync_count: u64,
}

impl HeadlessSimulatedAdapter {
    #[must_use]
    pub const fn new(mechanism: AdapterMechanism) -> Self {
        Self {
            mechanism: Some(mechanism),
            outbound: VecDeque::new(),
            last_readback: None,
            sync_count: 0,
        }
    }

    pub fn enqueue(&mut self, command: TextInputCommand) {
        self.outbound.push_back(command);
    }

    #[must_use]
    pub const fn last_readback(&self) -> Option<AdapterReadback> {
        self.last_readback
    }

    #[must_use]
    pub const fn sync_count(&self) -> u64 {
        self.sync_count
    }
}

impl PlatformInputAdapter for HeadlessSimulatedAdapter {
    fn mechanism(&self) -> AdapterMechanism {
        self.mechanism
            .unwrap_or(AdapterMechanism::HiddenTextareaImeBridge)
    }

    fn drain_commands(&mut self) -> Vec<TextInputCommand> {
        self.outbound.drain(..).collect()
    }

    fn sync_readback(&mut self, readback: AdapterReadback) {
        self.last_readback = Some(readback);
        self.sync_count = self.sync_count.saturating_add(1);
    }
}
