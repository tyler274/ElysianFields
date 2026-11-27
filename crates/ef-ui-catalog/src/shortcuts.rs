use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::a11y::A11ySemanticKey;

/// Stable focus target identity tracked for focus trapping and exact restoration.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FocusId(pub A11ySemanticKey);

/// Modifier chord normalized across macOS (`Cmd`) and Linux/Windows (`Ctrl`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifierChord {
    None,
    /// `Cmd` on macOS or `Ctrl` on Linux/Windows.
    Primary,
    Shift,
    /// `Cmd+Shift` on macOS or `Ctrl+Shift` on Linux/Windows.
    PrimaryShift,
}

/// Logical key pressed in the shortcut router.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutKey {
    Char(char),
    ArrowUp,
    ArrowDown,
    Enter,
    Escape,
    Tab,
}

/// Normalized keyboard event routed through [`ShortcutRouter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortcutInput {
    pub key: ShortcutKey,
    pub modifiers: ModifierChord,
}

impl ShortcutInput {
    #[must_use]
    pub const fn new(key: ShortcutKey, modifiers: ModifierChord) -> Self {
        Self { key, modifiers }
    }
}

/// Whether keyboard input is currently captured by a text input or composer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditingContext {
    #[default]
    Inactive,
    ActiveTextField,
}

/// Whether an open search/finder input currently holds non-empty text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchQueryState {
    #[default]
    Empty,
    NonEmpty,
}

/// Current input context consulted when evaluating workspace shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShortcutContext {
    pub editing: EditingContext,
    pub search_query: SearchQueryState,
}

/// Overlays, drawers, and panels managed in LIFO dismissal order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKind {
    PatientFinder,
    ShortcutsHelp,
    CitationPane,
    MobileDrawer,
    ConfirmModal,
}

/// Whether an overlay traps focus and pauses workspace shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayModality {
    Modal,
    NonModal,
}

impl OverlayKind {
    #[must_use]
    pub const fn modality(self) -> OverlayModality {
        match self {
            Self::PatientFinder | Self::ShortcutsHelp | Self::ConfirmModal => {
                OverlayModality::Modal
            }
            Self::CitationPane | Self::MobileDrawer => OverlayModality::NonModal,
        }
    }

    #[must_use]
    pub fn initial_focus(self) -> FocusId {
        match self {
            Self::PatientFinder => FocusId(A11ySemanticKey::PatientFinderInput),
            Self::ShortcutsHelp => FocusId(A11ySemanticKey::ShortcutsHelpDialog),
            Self::CitationPane => FocusId(A11ySemanticKey::CitationPanelRegion),
            Self::MobileDrawer => FocusId(A11ySemanticKey::NavigationLandmark),
            Self::ConfirmModal => FocusId(A11ySemanticKey::SendButton),
        }
    }
}

/// An active overlay entry storing the exact control to restore focus to on dismiss.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveOverlay {
    pub kind: OverlayKind,
    pub restore_focus: FocusId,
}

/// Privacy screen cover state keeping exact prior focus when activated.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PrivacyCoverState {
    #[default]
    Off,
    Active {
        restore_focus: FocusId,
    },
}

/// Action emitted by [`ShortcutRouter::route`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortcutAction {
    PrivacyCoverActivated,
    PrivacyCoverDismissed {
        restored_focus: FocusId,
    },
    OpenPatientFinder {
        restore_focus: FocusId,
    },
    OpenShortcutsHelp {
        restore_focus: FocusId,
    },
    ListMoveNext,
    ListMovePrevious,
    ActivateFocused {
        target: FocusId,
    },
    ClearSearchInput,
    DismissOverlay {
        dismissed: OverlayKind,
        restored_focus: FocusId,
    },
    ModalFocusTrapTab {
        forward: bool,
    },
    Unhandled,
}

/// Single-owner keyboard shortcut router, overlay stack, privacy screen cover,
/// and focus restoration controller matching M2 specification §4.3.
#[derive(Debug, Clone, Resource)]
pub struct ShortcutRouter {
    current_focus: FocusId,
    privacy_cover: PrivacyCoverState,
    overlays: Vec<ActiveOverlay>,
}

impl Default for ShortcutRouter {
    fn default() -> Self {
        Self::new(FocusId(A11ySemanticKey::ListRow(0)))
    }
}

impl ShortcutRouter {
    #[must_use]
    pub const fn new(initial_focus: FocusId) -> Self {
        Self {
            current_focus: initial_focus,
            privacy_cover: PrivacyCoverState::Off,
            overlays: Vec::new(),
        }
    }

    #[must_use]
    pub const fn current_focus(&self) -> &FocusId {
        &self.current_focus
    }

    pub fn set_focus(&mut self, focus: FocusId) {
        self.current_focus = focus;
    }

    #[must_use]
    pub const fn privacy_cover(&self) -> &PrivacyCoverState {
        &self.privacy_cover
    }

    #[must_use]
    pub const fn is_privacy_cover_active(&self) -> bool {
        matches!(self.privacy_cover, PrivacyCoverState::Active { .. })
    }

    #[must_use]
    pub fn overlays(&self) -> &[ActiveOverlay] {
        &self.overlays
    }

    #[must_use]
    pub fn has_modal_open(&self) -> bool {
        self.is_privacy_cover_active()
            || self
                .overlays
                .iter()
                .any(|entry| matches!(entry.kind.modality(), OverlayModality::Modal))
    }

    /// Opens a modal dialog, citation pane, or mobile drawer, moving focus inside
    /// it and recording the exact triggering control for restoration on dismiss.
    pub fn open_overlay(&mut self, kind: OverlayKind) -> FocusId {
        let restore_focus = self.current_focus.clone();
        self.overlays.push(ActiveOverlay {
            kind,
            restore_focus: restore_focus.clone(),
        });
        self.current_focus = kind.initial_focus();
        restore_focus
    }

    /// Dismisses the privacy screen cover explicitly (for example, via button click).
    pub fn dismiss_privacy_cover(&mut self) -> Option<FocusId> {
        let prev = std::mem::replace(&mut self.privacy_cover, PrivacyCoverState::Off);
        let PrivacyCoverState::Active { restore_focus } = prev else {
            return None;
        };
        self.current_focus = restore_focus.clone();
        Some(restore_focus)
    }

    /// Routes a keyboard shortcut according to the M2 §4.3 scope matrix.
    pub fn route(&mut self, input: ShortcutInput, context: ShortcutContext) -> ShortcutAction {
        // 1. Global Privacy Cover shortcut (`Cmd/Ctrl+Shift+L`) is ALWAYS active,
        //    even while editing text or inside a modal dialog.
        if matches!(input.modifiers, ModifierChord::PrimaryShift)
            && matches!(input.key, ShortcutKey::Char('l' | 'L'))
        {
            return self.toggle_privacy_cover();
        }

        // While privacy cover is active, block all workspace shortcuts beneath it.
        if self.is_privacy_cover_active() {
            if matches!(input.key, ShortcutKey::Tab) {
                return ShortcutAction::ModalFocusTrapTab {
                    forward: !matches!(input.modifiers, ModifierChord::Shift),
                };
            }
            return ShortcutAction::Unhandled;
        }

        // 2. Escape: first clears non-empty search input, then dismisses topmost overlay
        //    and restores exact triggering focus.
        if matches!(input.key, ShortcutKey::Escape)
            && matches!(input.modifiers, ModifierChord::None)
        {
            if matches!(context.search_query, SearchQueryState::NonEmpty) {
                return ShortcutAction::ClearSearchInput;
            }
            if let Some(top) = self.overlays.pop() {
                self.current_focus = top.restore_focus.clone();
                return ShortcutAction::DismissOverlay {
                    dismissed: top.kind,
                    restored_focus: top.restore_focus,
                };
            }
            return ShortcutAction::Unhandled;
        }

        let modal_open = self.has_modal_open();

        // Modal focus trap for Tab / Shift+Tab.
        if modal_open && matches!(input.key, ShortcutKey::Tab) {
            return ShortcutAction::ModalFocusTrapTab {
                forward: !matches!(input.modifiers, ModifierChord::Shift),
            };
        }

        // 3. `Cmd/Ctrl+K`: workspace shortcut paused inside modal dialogs
        //    (works even when composer text field has focus).
        if matches!(input.modifiers, ModifierChord::Primary)
            && matches!(input.key, ShortcutKey::Char('k' | 'K'))
        {
            if modal_open {
                return ShortcutAction::Unhandled;
            }
            let restore_focus = self.open_overlay(OverlayKind::PatientFinder);
            return ShortcutAction::OpenPatientFinder { restore_focus };
        }

        // Remaining single-key workspace shortcuts pause while editing text or inside a modal.
        if modal_open || matches!(context.editing, EditingContext::ActiveTextField) {
            return ShortcutAction::Unhandled;
        }

        match (input.modifiers, input.key) {
            (ModifierChord::None, ShortcutKey::Char('/')) => {
                let restore_focus = self.open_overlay(OverlayKind::PatientFinder);
                ShortcutAction::OpenPatientFinder { restore_focus }
            }
            (ModifierChord::None | ModifierChord::Shift, ShortcutKey::Char('?')) => {
                let restore_focus = self.open_overlay(OverlayKind::ShortcutsHelp);
                ShortcutAction::OpenShortcutsHelp { restore_focus }
            }
            (ModifierChord::None, ShortcutKey::Char('j') | ShortcutKey::ArrowDown) => {
                ShortcutAction::ListMoveNext
            }
            (ModifierChord::None, ShortcutKey::Char('k') | ShortcutKey::ArrowUp) => {
                ShortcutAction::ListMovePrevious
            }
            (ModifierChord::None, ShortcutKey::Enter) => ShortcutAction::ActivateFocused {
                target: self.current_focus.clone(),
            },
            _ => ShortcutAction::Unhandled,
        }
    }

    fn toggle_privacy_cover(&mut self) -> ShortcutAction {
        match std::mem::replace(&mut self.privacy_cover, PrivacyCoverState::Off) {
            PrivacyCoverState::Off => {
                let restore_focus = self.current_focus.clone();
                self.privacy_cover = PrivacyCoverState::Active { restore_focus };
                self.current_focus = FocusId(A11ySemanticKey::PrivacyCoverDismissButton);
                ShortcutAction::PrivacyCoverActivated
            }
            PrivacyCoverState::Active { restore_focus } => {
                self.current_focus = restore_focus.clone();
                ShortcutAction::PrivacyCoverDismissed {
                    restored_focus: restore_focus,
                }
            }
        }
    }
}
