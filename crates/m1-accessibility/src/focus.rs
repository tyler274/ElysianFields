use bevy::prelude::Resource;

use crate::contrast::{PalettePairId, PaletteTheme, contrast_ratio, elysian_palette};
use crate::domain::{
    ActiveCitation, CheckboxState, CitationId, ControlAvailability, DisclosureState, FocusTarget,
    FormValidationError, ModalState, NavItemId, ScreenState, SubmittedClinicalNote,
};
use crate::live_region::LiveAnnouncementQueue;

/// Keyboard-only commands supported across all four accessibility surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardCommand {
    TabForward,
    TabBackward,
    ArrowUp,
    ArrowDown,
    Home,
    End,
    ActivateEnter,
    ActivateSpace,
    Escape,
    TypeChar(char),
    Backspace,
}

/// Visible focus indicator styling synchronized with keyboard navigation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FocusRingStyle {
    pub target: FocusTarget,
    pub outline_px: f32,
    pub contrast_ratio: f64,
}

/// Focus ring presentation state (hidden until keyboard focus engagement).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FocusRingState {
    Hidden,
    Visible(FocusRingStyle),
}

/// Whether the modal was confirmed or canceled upon dismissal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalDismissReason {
    Confirmed,
    Canceled,
    Escaped,
}

/// Typed outcome of applying a [`KeyboardCommand`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FocusTransitionOutcome {
    Moved {
        from: FocusTarget,
        to: FocusTarget,
    },
    NavSelected(NavItemId),
    CitationOpened(CitationId),
    CheckboxToggled(CheckboxState),
    DisclosureToggled(DisclosureState),
    TextEdited {
        char_len: usize,
    },
    FormValidationFailed(FormValidationError),
    FormSubmitted(SubmittedClinicalNote),
    SignOffBlockedUntilSubmitted,
    ModalOpened {
        restore_focus: FocusTarget,
    },
    ModalDismissed {
        restored_focus: FocusTarget,
        reason: ModalDismissReason,
    },
    NoOp,
}

/// Ordered non-modal interactive controls in canonical reading order.
pub const NON_MODAL_FOCUS_ORDER: [FocusTarget; 11] = [
    FocusTarget::NavItem(NavItemId::Inbox),
    FocusTarget::NavItem(NavItemId::Transcript),
    FocusTarget::NavItem(NavItemId::ClinicalForm),
    FocusTarget::NavItem(NavItemId::LabRecords),
    FocusTarget::CitationLink(CitationId::ProgressNoteSummary),
    FocusTarget::CitationLink(CitationId::ClinicalLabPanel),
    FocusTarget::AssessmentNoteInput,
    FocusTarget::IncludeCitationsCheckbox,
    FocusTarget::GuidanceDisclosure,
    FocusTarget::SubmitNoteButton,
    FocusTarget::OpenSignOffModalButton,
];

/// Ordered modal-trapped interactive controls when the confirmation dialog is open.
pub const MODAL_FOCUS_ORDER: [FocusTarget; 2] = [
    FocusTarget::ModalConfirmButton,
    FocusTarget::ModalCancelButton,
];

/// Single owner for active keyboard traversal order and modal focus trapping.
#[must_use]
pub const fn ordered_focus_targets(modal: &ModalState) -> &'static [FocusTarget] {
    match modal {
        ModalState::Closed => &NON_MODAL_FOCUS_ORDER,
        ModalState::Open { .. } => &MODAL_FOCUS_ORDER,
    }
}

/// Single-owner keyboard focus manager maintaining current focus, modal focus
/// trap, exact focus restoration on dismiss, and visible focus ring state.
#[derive(Debug, Clone, Resource)]
pub struct FocusManager {
    current: FocusTarget,
    ring: FocusRingState,
}

impl Default for FocusManager {
    fn default() -> Self {
        Self::new(
            FocusTarget::NavItem(NavItemId::Transcript),
            PaletteTheme::Light,
        )
    }
}

impl FocusManager {
    #[must_use]
    pub fn new(initial: FocusTarget, theme: PaletteTheme) -> Self {
        Self {
            current: initial,
            ring: FocusRingState::Visible(build_focus_ring(initial, false, theme)),
        }
    }

    #[must_use]
    pub const fn current(&self) -> FocusTarget {
        self.current
    }

    #[must_use]
    pub const fn ring(&self) -> FocusRingState {
        self.ring
    }

    pub fn set_focus(&mut self, target: FocusTarget, modal_open: bool, theme: PaletteTheme) {
        self.current = target;
        self.ring = FocusRingState::Visible(build_focus_ring(target, modal_open, theme));
    }

    /// Executes a keyboard command against authoritative [`ScreenState`] and
    /// emits accessible announcements into [`LiveAnnouncementQueue`].
    pub fn apply_keyboard_command(
        &mut self,
        screen: &mut ScreenState,
        live: &mut LiveAnnouncementQueue,
        theme: PaletteTheme,
        now_ms: u64,
        command: KeyboardCommand,
    ) -> FocusTransitionOutcome {
        match command {
            KeyboardCommand::TabForward => {
                self.step_focus(&screen.modal, theme, StepDirection::Forward)
            }
            KeyboardCommand::TabBackward => {
                self.step_focus(&screen.modal, theme, StepDirection::Backward)
            }
            KeyboardCommand::ArrowUp => self.navigate_list(screen, theme, ListMove::Previous),
            KeyboardCommand::ArrowDown => self.navigate_list(screen, theme, ListMove::Next),
            KeyboardCommand::Home => self.navigate_list(screen, theme, ListMove::First),
            KeyboardCommand::End => self.navigate_list(screen, theme, ListMove::Last),
            KeyboardCommand::Escape => {
                self.dismiss_modal(screen, live, theme, now_ms, ModalDismissReason::Escaped)
            }
            KeyboardCommand::TypeChar(ch) => self.type_char(screen, ch),
            KeyboardCommand::Backspace => self.backspace(screen),
            KeyboardCommand::ActivateEnter => {
                self.activate_current(screen, live, theme, now_ms, ActivationKey::Enter)
            }
            KeyboardCommand::ActivateSpace => {
                self.activate_current(screen, live, theme, now_ms, ActivationKey::Space)
            }
        }
    }

    fn step_focus(
        &mut self,
        modal: &ModalState,
        theme: PaletteTheme,
        direction: StepDirection,
    ) -> FocusTransitionOutcome {
        let order = ordered_focus_targets(modal);
        let from = self.current;
        let current_idx = order.iter().position(|&item| item == from);

        let next_idx = match (current_idx, direction) {
            (Some(idx), StepDirection::Forward) => (idx + 1) % order.len(),
            (Some(idx), StepDirection::Backward) => {
                if idx == 0 {
                    order.len() - 1
                } else {
                    idx - 1
                }
            }
            (None, StepDirection::Forward) => 0,
            (None, StepDirection::Backward) => order.len() - 1,
        };

        let to = order[next_idx];
        self.set_focus(to, modal.is_open(), theme);
        FocusTransitionOutcome::Moved { from, to }
    }

    fn navigate_list(
        &mut self,
        screen: &mut ScreenState,
        theme: PaletteTheme,
        movement: ListMove,
    ) -> FocusTransitionOutcome {
        if screen.modal.is_open() {
            return FocusTransitionOutcome::NoOp;
        }
        let FocusTarget::NavItem(current_item) = self.current else {
            return FocusTransitionOutcome::NoOp;
        };

        let last_idx = NavItemId::ALL.len() - 1;
        let next_idx = match movement {
            ListMove::Previous => current_item.index().saturating_sub(1),
            ListMove::Next => (current_item.index() + 1).min(last_idx),
            ListMove::First => 0,
            ListMove::Last => last_idx,
        };

        let next_item = NavItemId::from_index(next_idx).unwrap_or(current_item);
        let from = self.current;
        let to = FocusTarget::NavItem(next_item);
        screen.active_nav = next_item;
        self.set_focus(to, false, theme);
        FocusTransitionOutcome::Moved { from, to }
    }

    fn type_char(&mut self, screen: &mut ScreenState, ch: char) -> FocusTransitionOutcome {
        if screen.modal.is_open() || self.current != FocusTarget::AssessmentNoteInput {
            return FocusTransitionOutcome::NoOp;
        }
        screen.note_draft.push(ch);
        FocusTransitionOutcome::TextEdited {
            char_len: screen.note_draft.chars().count(),
        }
    }

    fn backspace(&mut self, screen: &mut ScreenState) -> FocusTransitionOutcome {
        if screen.modal.is_open() || self.current != FocusTarget::AssessmentNoteInput {
            return FocusTransitionOutcome::NoOp;
        }
        let _ = screen.note_draft.pop();
        FocusTransitionOutcome::TextEdited {
            char_len: screen.note_draft.chars().count(),
        }
    }

    fn activate_current(
        &mut self,
        screen: &mut ScreenState,
        live: &mut LiveAnnouncementQueue,
        theme: PaletteTheme,
        now_ms: u64,
        key: ActivationKey,
    ) -> FocusTransitionOutcome {
        match self.current {
            FocusTarget::NavItem(item) => {
                screen.active_nav = item;
                live.announce_polite_event(format!("Selected {} view.", item.label()), now_ms);
                FocusTransitionOutcome::NavSelected(item)
            }
            FocusTarget::CitationLink(citation) => {
                screen.active_citation = ActiveCitation::Inspecting(citation);
                live.announce_polite_event(
                    format!(
                        "Opened {}. {}",
                        citation.accessible_name(),
                        citation.synthetic_excerpt()
                    ),
                    now_ms,
                );
                FocusTransitionOutcome::CitationOpened(citation)
            }
            FocusTarget::AssessmentNoteInput => match key {
                ActivationKey::Space => self.type_char(screen, ' '),
                ActivationKey::Enter => FocusTransitionOutcome::NoOp,
            },
            FocusTarget::IncludeCitationsCheckbox => {
                screen.include_citations = screen.include_citations.toggle();
                let state_word = match screen.include_citations {
                    CheckboxState::Checked => "checked",
                    CheckboxState::Unchecked => "unchecked",
                };
                live.announce_polite_event(
                    format!("Attach synthetic citation references {state_word}."),
                    now_ms,
                );
                FocusTransitionOutcome::CheckboxToggled(screen.include_citations)
            }
            FocusTarget::GuidanceDisclosure => {
                screen.guidance_disclosure = screen.guidance_disclosure.toggle();
                let state_word = match screen.guidance_disclosure {
                    DisclosureState::Expanded => "expanded",
                    DisclosureState::Collapsed => "collapsed",
                };
                live.announce_polite_event(
                    format!("Clinical documentation guidance {state_word}."),
                    now_ms,
                );
                FocusTransitionOutcome::DisclosureToggled(screen.guidance_disclosure)
            }
            FocusTarget::SubmitNoteButton => match screen.submit_assessment_note() {
                Ok(submitted) => {
                    live.announce_polite_event(
                        "Clinical assessment note submitted. Sign-off confirmation is now available.",
                        now_ms,
                    );
                    FocusTransitionOutcome::FormSubmitted(submitted)
                }
                Err(err) => {
                    live.announce_assertive(err.to_string(), now_ms);
                    FocusTransitionOutcome::FormValidationFailed(err)
                }
            },
            FocusTarget::OpenSignOffModalButton => {
                if !matches!(screen.sign_off_availability(), ControlAvailability::Enabled) {
                    live.announce_assertive(
                        "Submit a valid clinical assessment note before signing off.",
                        now_ms,
                    );
                    return FocusTransitionOutcome::SignOffBlockedUntilSubmitted;
                }
                let restore_focus = self.current;
                screen.modal = ModalState::Open {
                    restore_focus,
                    title: "Confirm clinical sign-off",
                    description: "Review synthetic assessment note before finalizing sign-off.",
                };
                self.set_focus(FocusTarget::ModalConfirmButton, true, theme);
                live.announce_assertive(
                    "Dialog opened: Confirm clinical sign-off. Focus moved to Confirm sign-off.",
                    now_ms,
                );
                FocusTransitionOutcome::ModalOpened { restore_focus }
            }
            FocusTarget::ModalConfirmButton => {
                screen.confirm_sign_off();
                self.dismiss_modal(screen, live, theme, now_ms, ModalDismissReason::Confirmed)
            }
            FocusTarget::ModalCancelButton => {
                self.dismiss_modal(screen, live, theme, now_ms, ModalDismissReason::Canceled)
            }
        }
    }

    fn dismiss_modal(
        &mut self,
        screen: &mut ScreenState,
        live: &mut LiveAnnouncementQueue,
        theme: PaletteTheme,
        now_ms: u64,
        reason: ModalDismissReason,
    ) -> FocusTransitionOutcome {
        let ModalState::Open { restore_focus, .. } = screen.modal else {
            return FocusTransitionOutcome::NoOp;
        };

        screen.modal = ModalState::Closed;
        self.set_focus(restore_focus, false, theme);

        let message = match reason {
            ModalDismissReason::Confirmed => "Clinical sign-off confirmed. Dialog closed.",
            ModalDismissReason::Canceled | ModalDismissReason::Escaped => {
                "Sign-off dialog dismissed. Focus restored."
            }
        };
        live.announce_polite_event(message, now_ms);

        FocusTransitionOutcome::ModalDismissed {
            restored_focus: restore_focus,
            reason,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepDirection {
    Forward,
    Backward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListMove {
    Previous,
    Next,
    First,
    Last,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivationKey {
    Enter,
    Space,
}

fn build_focus_ring(target: FocusTarget, modal_open: bool, theme: PaletteTheme) -> FocusRingStyle {
    let pair_id = if modal_open {
        PalettePairId::FocusRingOnModal
    } else {
        PalettePairId::FocusRingOnSurface
    };
    let ratio = elysian_palette(theme)
        .iter()
        .find(|pair| pair.id == pair_id)
        .map_or(0.0, |pair| contrast_ratio(pair.foreground, pair.background));

    FocusRingStyle {
        target,
        outline_px: 3.0,
        contrast_ratio: ratio,
    }
}
