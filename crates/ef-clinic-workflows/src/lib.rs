//! Headless Curie Clinic M3 Increment 1 workflow state machines and adapters.
//!
//! Strictly free of rendering engine dependencies (`bevy` is forbidden) and
//! strictly isolated from Curie Mobile (`ef-mobile-*` is forbidden).

pub mod ehr_channels;
pub mod matching;
pub mod memory;
pub mod replies;
pub mod today;
