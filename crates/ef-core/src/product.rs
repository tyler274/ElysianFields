//! Product boundary classification enforcing strict Curie Clinic / Curie Mobile isolation.

use serde::{Deserialize, Serialize};

/// Separate products in the Elysian Fields suite.
///
/// Each product owns separate accounts, clinical types, and persistence, sharing
/// code only through explicitly shared crates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProductBoundary {
    CurieClinic,
    CurieMobile,
}

impl ProductBoundary {
    /// Canonical lowercase kebab-case product identifier.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::CurieClinic => "curie-clinic",
            Self::CurieMobile => "curie-mobile",
        }
    }

    /// Returns true only when both operands belong to the exact same product boundary.
    #[must_use]
    pub const fn shares_storage_with(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::CurieClinic, Self::CurieClinic) | (Self::CurieMobile, Self::CurieMobile)
        )
    }
}
