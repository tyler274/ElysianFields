//! Shared headless domain foundation for Elysian Fields.
//!
//! This crate is strictly free of rendering engine dependencies and owns:
//! - Validated typed identifiers (`SessionId`, `PracticeId`, `PhysicianId`, etc.)
//! - Product boundary classification (`CurieClinic` vs `CurieMobile`)
//! - Segment-bounded `/elysianfields` subpath normalization and loopback override safety
//! - Nine-language localization (`Locale` + `TextDirection`)
//! - WCAG 2.1 AA theme tokens and contrast verification
//! - Structured authentication/authorization refusal codes

pub mod error;
pub mod id;
pub mod locale;
pub mod product;
pub mod refusal;
pub mod subpath;
pub mod theme;

pub use error::{CoreError, IdError, SubpathError};
pub use id::{
    AttemptId, ChatId, CitationId, CommandId, IdempotencyKey, PatientId, PhysicianId, PracticeId,
    RequestId, ResponseId, RunId, SessionId, SourceKey, TabId, TurnId, WorkItemId, WorkspaceId,
};
pub use locale::{LOCALE_STORAGE_KEY, Locale, TextDirection};
pub use product::ProductBoundary;
pub use refusal::{RefusalCode, StructuredRefusal};
pub use subpath::{CachePolicy, LoopbackApiOverride, Subpath};
pub use theme::{RgbColor, ThemeMode, ThemePalette};
