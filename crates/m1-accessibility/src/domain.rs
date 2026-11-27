use std::fmt;

use bevy::prelude::Resource;

/// The four required M1 accessibility prototype surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScreenSurface {
    Navigation,
    Transcript,
    ClinicalForm,
    Modal,
}

/// Typed navigation list entries in the workspace sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NavItemId {
    Inbox,
    Transcript,
    ClinicalForm,
    LabRecords,
}

impl NavItemId {
    /// Canonical reading and arrow-key traversal order.
    pub const ALL: [Self; 4] = [
        Self::Inbox,
        Self::Transcript,
        Self::ClinicalForm,
        Self::LabRecords,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Inbox => "Clinical Inbox",
            Self::Transcript => "Streaming Transcript",
            Self::ClinicalForm => "Assessment Form",
            Self::LabRecords => "Lab Records",
        }
    }

    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Inbox => 0,
            Self::Transcript => 1,
            Self::ClinicalForm => 2,
            Self::LabRecords => 3,
        }
    }

    #[must_use]
    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Inbox),
            1 => Some(Self::Transcript),
            2 => Some(Self::ClinicalForm),
            3 => Some(Self::LabRecords),
            _ => None,
        }
    }
}

/// Interactive citation links rendered inline within the streaming transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CitationId {
    ProgressNoteSummary,
    ClinicalLabPanel,
}

impl CitationId {
    pub const ALL: [Self; 2] = [Self::ProgressNoteSummary, Self::ClinicalLabPanel];

    #[must_use]
    pub const fn marker(self) -> &'static str {
        match self {
            Self::ProgressNoteSummary => "[1]",
            Self::ClinicalLabPanel => "[2]",
        }
    }

    #[must_use]
    pub const fn accessible_name(self) -> &'static str {
        match self {
            Self::ProgressNoteSummary => "Citation 1: Synthetic Progress Note Summary",
            Self::ClinicalLabPanel => "Citation 2: Synthetic Clinical Lab Panel",
        }
    }

    #[must_use]
    pub const fn synthetic_excerpt(self) -> &'static str {
        match self {
            Self::ProgressNoteSummary => {
                "Synthetic progress note: baseline vitals stable, follow-up review scheduled."
            }
            Self::ClinicalLabPanel => {
                "Synthetic lab panel: basic metabolic indicators within reference intervals."
            }
        }
    }
}

/// Interactive controls inside the modal dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModalActionId {
    Confirm,
    Cancel,
}

impl ModalActionId {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Confirm => "Confirm sign-off",
            Self::Cancel => "Cancel and return",
        }
    }
}

/// Exhaustive set of keyboard-focusable controls across all four surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FocusTarget {
    NavItem(NavItemId),
    CitationLink(CitationId),
    AssessmentNoteInput,
    IncludeCitationsCheckbox,
    GuidanceDisclosure,
    SubmitNoteButton,
    OpenSignOffModalButton,
    ModalConfirmButton,
    ModalCancelButton,
}

impl FocusTarget {
    /// Converts a focusable control into its canonical semantic node identity.
    #[must_use]
    pub const fn semantic_id(self) -> SemanticControlId {
        match self {
            Self::NavItem(item) => SemanticControlId::NavItem(item),
            Self::CitationLink(citation) => SemanticControlId::CitationLink(citation),
            Self::AssessmentNoteInput => SemanticControlId::AssessmentNoteInput,
            Self::IncludeCitationsCheckbox => SemanticControlId::IncludeCitationsCheckbox,
            Self::GuidanceDisclosure => SemanticControlId::GuidanceDisclosure,
            Self::SubmitNoteButton => SemanticControlId::SubmitNoteButton,
            Self::OpenSignOffModalButton => SemanticControlId::OpenSignOffModalButton,
            Self::ModalConfirmButton => SemanticControlId::ModalConfirmButton,
            Self::ModalCancelButton => SemanticControlId::ModalCancelButton,
        }
    }

    #[must_use]
    pub const fn surface(self) -> ScreenSurface {
        match self {
            Self::NavItem(_) => ScreenSurface::Navigation,
            Self::CitationLink(_) => ScreenSurface::Transcript,
            Self::AssessmentNoteInput
            | Self::IncludeCitationsCheckbox
            | Self::GuidanceDisclosure
            | Self::SubmitNoteButton
            | Self::OpenSignOffModalButton => ScreenSurface::ClinicalForm,
            Self::ModalConfirmButton | Self::ModalCancelButton => ScreenSurface::Modal,
        }
    }

    #[must_use]
    pub const fn accessible_name(self) -> &'static str {
        match self {
            Self::NavItem(item) => item.label(),
            Self::CitationLink(citation) => citation.accessible_name(),
            Self::AssessmentNoteInput => "Clinical assessment note",
            Self::IncludeCitationsCheckbox => "Attach synthetic citation references",
            Self::GuidanceDisclosure => "Clinical documentation guidance",
            Self::SubmitNoteButton => "Submit clinical note",
            Self::OpenSignOffModalButton => "Review and sign off note",
            Self::ModalConfirmButton => ModalActionId::Confirm.label(),
            Self::ModalCancelButton => ModalActionId::Cancel.label(),
        }
    }
}

/// Single-owner identity for every landmark, heading, live region, alert, and
/// interactive control exposed in the AccessKit tree and narrow browser bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SemanticControlId {
    WorkspaceNavigationLandmark,
    NavItem(NavItemId),
    TranscriptMainLandmark,
    TranscriptHeading,
    TranscriptLiveRegion,
    CitationLink(CitationId),
    ClinicalFormLandmark,
    ClinicalFormHeading,
    AssessmentNoteInput,
    IncludeCitationsCheckbox,
    GuidanceDisclosure,
    FormValidationAlert,
    SubmitNoteButton,
    OpenSignOffModalButton,
    SignOffModalDialog,
    SignOffModalHeading,
    ModalConfirmButton,
    ModalCancelButton,
}

impl SemanticControlId {
    /// Stable DOM key suffix used by the narrow browser adapter (`data-ef-control`).
    #[must_use]
    pub const fn stable_key(self) -> &'static str {
        match self {
            Self::WorkspaceNavigationLandmark => "landmark-nav",
            Self::NavItem(NavItemId::Inbox) => "nav-inbox",
            Self::NavItem(NavItemId::Transcript) => "nav-transcript",
            Self::NavItem(NavItemId::ClinicalForm) => "nav-clinical-form",
            Self::NavItem(NavItemId::LabRecords) => "nav-lab-records",
            Self::TranscriptMainLandmark => "landmark-transcript",
            Self::TranscriptHeading => "heading-transcript",
            Self::TranscriptLiveRegion => "live-transcript",
            Self::CitationLink(CitationId::ProgressNoteSummary) => "citation-1",
            Self::CitationLink(CitationId::ClinicalLabPanel) => "citation-2",
            Self::ClinicalFormLandmark => "landmark-form",
            Self::ClinicalFormHeading => "heading-form",
            Self::AssessmentNoteInput => "input-assessment-note",
            Self::IncludeCitationsCheckbox => "checkbox-include-citations",
            Self::GuidanceDisclosure => "disclosure-guidance",
            Self::FormValidationAlert => "alert-form-validation",
            Self::SubmitNoteButton => "button-submit-note",
            Self::OpenSignOffModalButton => "button-open-sign-off-modal",
            Self::SignOffModalDialog => "dialog-sign-off",
            Self::SignOffModalHeading => "heading-sign-off",
            Self::ModalConfirmButton => "button-modal-confirm",
            Self::ModalCancelButton => "button-modal-cancel",
        }
    }

    #[must_use]
    pub const fn is_interactive(self) -> bool {
        matches!(
            self,
            Self::NavItem(_)
                | Self::CitationLink(_)
                | Self::AssessmentNoteInput
                | Self::IncludeCitationsCheckbox
                | Self::GuidanceDisclosure
                | Self::SubmitNoteButton
                | Self::OpenSignOffModalButton
                | Self::ModalConfirmButton
                | Self::ModalCancelButton
        )
    }

    #[must_use]
    pub const fn is_modal_descendant(self) -> bool {
        matches!(
            self,
            Self::SignOffModalDialog
                | Self::SignOffModalHeading
                | Self::ModalConfirmButton
                | Self::ModalCancelButton
        )
    }

    /// Canonical reading order across all surfaces.
    pub const ALL_IN_READING_ORDER: [Self; 22] = [
        Self::WorkspaceNavigationLandmark,
        Self::NavItem(NavItemId::Inbox),
        Self::NavItem(NavItemId::Transcript),
        Self::NavItem(NavItemId::ClinicalForm),
        Self::NavItem(NavItemId::LabRecords),
        Self::TranscriptMainLandmark,
        Self::TranscriptHeading,
        Self::TranscriptLiveRegion,
        Self::CitationLink(CitationId::ProgressNoteSummary),
        Self::CitationLink(CitationId::ClinicalLabPanel),
        Self::ClinicalFormLandmark,
        Self::ClinicalFormHeading,
        Self::AssessmentNoteInput,
        Self::IncludeCitationsCheckbox,
        Self::GuidanceDisclosure,
        Self::FormValidationAlert,
        Self::SubmitNoteButton,
        Self::OpenSignOffModalButton,
        Self::SignOffModalDialog,
        Self::SignOffModalHeading,
        Self::ModalConfirmButton,
        Self::ModalCancelButton,
    ];
}

/// Modal dialog lifecycle state capturing exact prior focus for restoration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalState {
    Closed,
    Open {
        restore_focus: FocusTarget,
        title: &'static str,
        description: &'static str,
    },
}

impl ModalState {
    #[must_use]
    pub const fn is_open(&self) -> bool {
        matches!(self, Self::Open { .. })
    }
}

/// Typed toggle state for the clinical citation attachment checkbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckboxState {
    Unchecked,
    Checked,
}

impl CheckboxState {
    #[must_use]
    pub const fn toggle(self) -> Self {
        match self {
            Self::Unchecked => Self::Checked,
            Self::Checked => Self::Unchecked,
        }
    }

    #[must_use]
    pub const fn to_accesskit(self) -> accesskit::Toggled {
        match self {
            Self::Unchecked => accesskit::Toggled::False,
            Self::Checked => accesskit::Toggled::True,
        }
    }
}

/// Typed expansion state for the clinical guidance disclosure control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisclosureState {
    Collapsed,
    Expanded,
}

impl DisclosureState {
    #[must_use]
    pub const fn toggle(self) -> Self {
        match self {
            Self::Collapsed => Self::Expanded,
            Self::Expanded => Self::Collapsed,
        }
    }

    #[must_use]
    pub const fn is_expanded(self) -> bool {
        matches!(self, Self::Expanded)
    }
}

/// Typed validation errors for the clinical assessment form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormValidationError {
    EmptyNote,
    NoteTooShort {
        min_chars: usize,
        actual_chars: usize,
    },
}

impl fmt::Display for FormValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyNote => write!(f, "Clinical assessment note is required before submitting."),
            Self::NoteTooShort {
                min_chars,
                actual_chars,
            } => write!(
                f,
                "Clinical assessment note must be at least {min_chars} characters (currently {actual_chars})."
            ),
        }
    }
}

impl std::error::Error for FormValidationError {}

/// Validation state of the assessment note field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldValidation {
    Untouched,
    Valid,
    Invalid(FormValidationError),
}

/// Explicit control availability rather than raw booleans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlAvailability {
    Enabled,
    Disabled(DisabledReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisabledReason {
    RequiresSubmittedNote,
}

/// Confirmed submission record stored in authoritative domain state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmittedClinicalNote {
    pub body: String,
    pub citations_attached: CheckboxState,
    pub signed_off: SignOffStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignOffStatus {
    PendingReview,
    Confirmed,
}

/// Form submission lifecycle state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmissionState {
    Idle,
    Submitted(SubmittedClinicalNote),
}

/// Active citation inspection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveCitation {
    None,
    Inspecting(CitationId),
}

/// Authoritative domain state for the 4-surface M1 accessibility prototype.
#[derive(Debug, Clone, Resource)]
pub struct ScreenState {
    pub active_nav: NavItemId,
    pub transcript_body: String,
    pub active_citation: ActiveCitation,
    pub note_draft: String,
    pub include_citations: CheckboxState,
    pub guidance_disclosure: DisclosureState,
    pub validation: FieldValidation,
    pub submission: SubmissionState,
    pub modal: ModalState,
}

impl Default for ScreenState {
    fn default() -> Self {
        Self {
            active_nav: NavItemId::Transcript,
            transcript_body: String::from(
                "Synthetic assistant summary: baseline clinical review ready with inline references [1] and [2].",
            ),
            active_citation: ActiveCitation::None,
            note_draft: String::new(),
            include_citations: CheckboxState::Checked,
            guidance_disclosure: DisclosureState::Collapsed,
            validation: FieldValidation::Untouched,
            submission: SubmissionState::Idle,
            modal: ModalState::Closed,
        }
    }
}

impl ScreenState {
    pub const MIN_NOTE_CHARS: usize = 8;

    /// Single owner for whether the sign-off modal trigger button is enabled.
    #[must_use]
    pub const fn sign_off_availability(&self) -> ControlAvailability {
        match self.submission {
            SubmissionState::Idle => {
                ControlAvailability::Disabled(DisabledReason::RequiresSubmittedNote)
            }
            SubmissionState::Submitted(_) => ControlAvailability::Enabled,
        }
    }

    /// Validates and submits the clinical assessment note, updating domain state.
    pub fn submit_assessment_note(&mut self) -> Result<SubmittedClinicalNote, FormValidationError> {
        let trimmed = self.note_draft.trim();
        let char_count = trimmed.chars().count();
        if char_count == 0 {
            let err = FormValidationError::EmptyNote;
            self.validation = FieldValidation::Invalid(err);
            return Err(err);
        }
        if char_count < Self::MIN_NOTE_CHARS {
            let err = FormValidationError::NoteTooShort {
                min_chars: Self::MIN_NOTE_CHARS,
                actual_chars: char_count,
            };
            self.validation = FieldValidation::Invalid(err);
            return Err(err);
        }

        let record = SubmittedClinicalNote {
            body: trimmed.to_owned(),
            citations_attached: self.include_citations,
            signed_off: SignOffStatus::PendingReview,
        };
        self.validation = FieldValidation::Valid;
        self.submission = SubmissionState::Submitted(record.clone());
        Ok(record)
    }

    /// Marks the currently submitted note as confirmed upon modal confirmation.
    pub fn confirm_sign_off(&mut self) {
        if let SubmissionState::Submitted(ref mut note) = self.submission {
            note.signed_off = SignOffStatus::Confirmed;
        }
    }
}
