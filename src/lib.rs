//! Storage-independent CCVL style contracts and versioned asset bundles.
//! Hosts supply records and bytes; the core never opens a project directory.

/// Style contract validation lives in `cvst`.
pub mod contract {
    pub use cvst::validate_contract;
}
#[cfg(feature = "doc")]
pub mod doc;
pub mod edit;
#[cfg(feature = "render")]
pub mod pdf;
#[cfg(feature = "render")]
pub mod render;
#[cfg(feature = "doc")]
pub mod session;
pub mod style;
pub mod transfer;

pub use style::{StyleBundle, StyleDocument};

pub use cvst::normalize_locale;
