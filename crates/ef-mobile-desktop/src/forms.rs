//! Typed multi-field form focus, clipboard buffer, and input parsers for `ef-mobile-desktop`.

use bevy::prelude::Resource;
use ef_connected_care::{BridgeError, DateOfBirth, InviteCredential, InviteShortCode, InviteToken};
use ef_core::IdError;
use ef_ui_catalog::{
    A11ySemanticKey, ComposerAction, ComposerOutcome, ComposerSurface, MemoryTextAdapter,
    PlatformTextAdapter,
};
use thiserror::Error;

/// Closed 4-field focus ring for interactive Curie Mobile desktop forms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MobileInputFocus {
    Email,
    InviteCode,
    DateOfBirth,
    #[default]
    CuratedNote,
}

impl MobileInputFocus {
    /// Canonical Tab focus traversal order across all 4 interactive fields.
    pub const ALL: [Self; 4] = [
        Self::Email,
        Self::InviteCode,
        Self::DateOfBirth,
        Self::CuratedNote,
    ];

    /// Advances focus forward (`Tab`).
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Email => Self::InviteCode,
            Self::InviteCode => Self::DateOfBirth,
            Self::DateOfBirth => Self::CuratedNote,
            Self::CuratedNote => Self::Email,
        }
    }

    /// Moves focus backward (`Shift+Tab`).
    #[must_use]
    pub const fn prev(self) -> Self {
        match self {
            Self::Email => Self::CuratedNote,
            Self::InviteCode => Self::Email,
            Self::DateOfBirth => Self::InviteCode,
            Self::CuratedNote => Self::DateOfBirth,
        }
    }

    /// Parses a CLI `--focus` target (`email`, `invite`, `dob`, or `note`).
    #[must_use]
    pub fn parse_cli(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "email" => Some(Self::Email),
            "invite" => Some(Self::InviteCode),
            "dob" => Some(Self::DateOfBirth),
            "note" => Some(Self::CuratedNote),
            _ => None,
        }
    }

    /// Returns `true` only for multiline inputs (`CuratedNote`).
    #[must_use]
    pub const fn is_multiline(self) -> bool {
        matches!(self, Self::CuratedNote)
    }

    /// Returns the unique AccessKit semantic key assigned to this interactive field.
    #[must_use]
    pub const fn a11y_key(self) -> A11ySemanticKey {
        match self {
            Self::Email => A11ySemanticKey::TableRow(0),
            Self::InviteCode => A11ySemanticKey::TableRow(1),
            Self::DateOfBirth => A11ySemanticKey::TableRow(2),
            Self::CuratedNote => A11ySemanticKey::TableRow(3),
        }
    }
}

/// Typed zero-dependency clipboard buffer shared across all 4 interactive desktop fields.
#[derive(Debug, Default, Clone, Resource)]
pub struct ClipboardBuffer {
    adapter: MemoryTextAdapter,
}

impl ClipboardBuffer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Writes `text` into the clipboard buffer.
    pub fn copy(&mut self, text: impl Into<String>) {
        self.adapter.clipboard = text.into();
    }

    /// Returns the current clipboard text when non-empty.
    #[must_use]
    pub fn paste(&self) -> Option<&str> {
        if self.adapter.clipboard.is_empty() {
            None
        } else {
            Some(self.adapter.clipboard.as_str())
        }
    }

    /// Clears the clipboard buffer.
    pub fn clear(&mut self) {
        self.adapter.clipboard.clear();
    }

    /// Exposes the underlying [`MemoryTextAdapter`] for [`ComposerSurface`] clipboard bridges.
    pub fn adapter_mut(&mut self) -> &mut MemoryTextAdapter {
        &mut self.adapter
    }
}

/// Typed validation errors when parsing user-edited form fields before dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FormParseError {
    #[error("enter an email address before signing in")]
    EmptyEmail,
    #[error("enter a valid email address containing '@'")]
    InvalidEmail,
    #[error("enter a practice invitation code or link token")]
    EmptyInviteCode,
    #[error("invalid invitation credential: {0}")]
    InvalidInviteCredential(#[from] IdError),
    #[error("enter date of birth as YYYY-MM-DD (for example 1985-04-12)")]
    InvalidDobFormat,
    #[error("enter numeric YYYY-MM-DD date of birth: {0}")]
    InvalidDobNumber(#[from] std::num::ParseIntError),
    #[error("invalid calendar date of birth: {0}")]
    InvalidCalendarDate(#[from] BridgeError),
}

/// Validates and normalizes an interactive email field value.
pub fn parse_email(raw: &str) -> Result<String, FormParseError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(FormParseError::EmptyEmail);
    }
    let Some((local, domain)) = trimmed.split_once('@') else {
        return Err(FormParseError::InvalidEmail);
    };
    if local.is_empty()
        || domain.is_empty()
        || domain.contains('@')
        || trimmed.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(FormParseError::InvalidEmail);
    }
    Ok(trimmed.to_owned())
}

/// Parses an interactive invitation field into a typed [`InviteCredential`].
///
/// Accepts short manual codes (e.g. `ABCD-1234`), opaque link tokens (`tok-...` / `invtok-...`),
/// or pasted `/invite?token=...` links. When `preset` matches the trimmed input verbatim,
/// preserves its exact variant.
pub fn parse_invite_credential(
    raw: &str,
    preset: Option<&InviteCredential>,
) -> Result<InviteCredential, FormParseError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "---" {
        return Err(FormParseError::EmptyInviteCode);
    }

    if let Some(existing) = preset {
        let existing_str = match existing {
            InviteCredential::LinkToken(token) => token.as_str(),
            InviteCredential::ShortCode(code) => code.as_str(),
        };
        if existing_str == trimmed {
            return Ok(existing.clone());
        }
    }

    if let Some(token_part) = extract_invite_query_token(trimmed) {
        return Ok(InviteCredential::LinkToken(InviteToken::new(token_part)?));
    }

    if trimmed.starts_with("tok-") || trimmed.starts_with("invtok-") {
        return Ok(InviteCredential::LinkToken(InviteToken::new(trimmed)?));
    }

    Ok(InviteCredential::ShortCode(InviteShortCode::new(trimmed)?))
}

fn extract_invite_query_token(raw: &str) -> Option<&str> {
    let (_, query) = raw.split_once('?')?;
    for pair in query.split('&') {
        if let Some(value) = pair.strip_prefix("token=") {
            let clean = value.split('#').next().unwrap_or(value).trim();
            if !clean.is_empty() {
                return Some(clean);
            }
        }
    }
    None
}

/// Parses an ISO `YYYY-MM-DD` date string into a validated [`DateOfBirth`].
pub fn parse_dob_iso(raw: &str) -> Result<DateOfBirth, FormParseError> {
    let trimmed = raw.trim();
    let mut parts = trimmed.split('-');
    let (Some(y_str), Some(m_str), Some(d_str), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(FormParseError::InvalidDobFormat);
    };
    if y_str.len() != 4
        || m_str.is_empty()
        || m_str.len() > 2
        || d_str.is_empty()
        || d_str.len() > 2
    {
        return Err(FormParseError::InvalidDobFormat);
    }
    let year = y_str.parse::<u16>()?;
    let month = m_str.parse::<u8>()?;
    let day = d_str.parse::<u8>()?;
    Ok(DateOfBirth::new(year, month, day)?)
}

/// Replaces all text in a [`ComposerSurface`] while keeping cursor at end of text.
pub fn replace_surface_text(surface: &mut ComposerSurface, text: &str) {
    let _ = surface.apply(ComposerAction::SelectAll);
    let _ = surface.apply(ComposerAction::Backspace);
    if !text.is_empty() {
        let _ = surface.apply(ComposerAction::InsertText(text.to_owned()));
    }
}

/// Copies the active selection (or the entire field text when selection is collapsed) to [`ClipboardBuffer`].
pub fn copy_surface_to_clipboard(
    surface: &ComposerSurface,
    clipboard: &mut ClipboardBuffer,
) -> bool {
    if !surface.selected_text().is_empty() {
        return surface
            .copy_to_adapter(clipboard.adapter_mut())
            .unwrap_or(false);
    }
    if surface.text().is_empty() {
        return false;
    }
    clipboard
        .adapter_mut()
        .write_clipboard(surface.text())
        .is_ok()
}

/// Cuts the active selection (or the entire field text when selection is collapsed) to [`ClipboardBuffer`].
pub fn cut_surface_to_clipboard(
    surface: &mut ComposerSurface,
    clipboard: &mut ClipboardBuffer,
) -> bool {
    if !surface.selected_text().is_empty() {
        return surface
            .cut_to_adapter(clipboard.adapter_mut())
            .unwrap_or(false);
    }
    if surface.text().is_empty() {
        return false;
    }
    let _ = surface.apply(ComposerAction::SelectAll);
    surface
        .cut_to_adapter(clipboard.adapter_mut())
        .unwrap_or(false)
}

/// Pastes from [`ClipboardBuffer`] into `surface`, stripping newlines on single-line fields.
pub fn paste_clipboard_into_surface(
    surface: &mut ComposerSurface,
    clipboard: &ClipboardBuffer,
    single_line: bool,
) -> ComposerOutcome {
    let Some(raw) = clipboard.paste() else {
        return ComposerOutcome::NoOp;
    };
    let text = if single_line {
        raw.chars()
            .filter(|ch| *ch != '\r' && *ch != '\n')
            .collect::<String>()
    } else {
        raw.to_owned()
    };
    if text.is_empty() {
        return ComposerOutcome::NoOp;
    }
    surface.apply(ComposerAction::InsertText(text))
}
