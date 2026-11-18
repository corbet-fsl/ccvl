//! The allow-list for URLs a workspace stores and a client renders as links.
//!
//! One rule, mirrored by `@cv/schemas/safe-url` of the `CareerVector` monorepo and proven identical by
//! `tests/vectors/safe-urls.json`: `http`, `https` and `mailto` may be stored and linked (people apply by
//! email; none of them runs script), `careervector-local` (the placeholder of a manual job) may be stored but
//! is never a link. Everything else (`javascript:`, `data:`, `vbscript:`, `file:`, `ftp:`, ...) is refused, as is
//! any text the URL parser would silently repair: leading or trailing whitespace and control characters
//! (a browser drops tabs and newlines inside `java\tscript:`).
use url::Url;

const STORED: &[&str] = &["http", "https", "mailto", "careervector-local"];
const LINK: &[&str] = &["http", "https", "mailto"];

fn scheme_of(value: &str) -> Option<String> {
    if value.is_empty()
        || value != value.trim()
        || value.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}')
    {
        return None;
    }
    Url::parse(value).ok().map(|u| u.scheme().to_string())
}

/// May this URL be stored on a job?
#[must_use]
pub fn is_storable_job_url(value: &str) -> bool {
    scheme_of(value).is_some_and(|s| STORED.contains(&s.as_str()))
}

/// May this URL be rendered as an href or opened?
#[must_use]
pub fn is_safe_link_url(value: &str) -> bool {
    scheme_of(value).is_some_and(|s| LINK.contains(&s.as_str()))
}

/// The refusal text of `job.create`/`job.update` (identical in the TypeScript applier).
pub const UNSAFE_JOB_URL: &str = "url must be an http(s), mailto or careervector-local address";
