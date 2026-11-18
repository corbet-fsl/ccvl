//! CCVL: style contracts, versioned asset bundles, the Yrs document engine and the job id language
//! (feature `jobid`: `normalize_url`, `job_id_from_url`).
//! Hosts supply records and bytes; the core never opens a project directory.

/// Style contract validation lives in `cvst`.
#[cfg(feature = "core")]
pub mod contract {
    pub use cvst::validate_contract;
}
#[cfg(feature = "doc")]
pub mod doc;
#[cfg(feature = "core")]
pub mod edit;
#[cfg(feature = "jobid")]
pub mod jobid;
#[cfg(feature = "locale")]
pub mod locale;
#[cfg(feature = "offline")]
pub mod offline;
#[cfg(feature = "render")]
pub mod pdf;
#[cfg(feature = "render")]
pub mod render;
#[cfg(feature = "doc")]
#[cfg(feature = "jobid")]
pub mod safeurl;
pub mod session;
#[cfg(feature = "core")]
pub mod style;
#[cfg(feature = "core")]
pub mod transfer;

#[cfg(feature = "jobid")]
pub use jobid::{job_id_from_url, normalize_url};
#[cfg(feature = "core")]
pub use style::{StyleBundle, StyleDocument};

#[cfg(feature = "core")]
pub use cvst::normalize_locale;
