//! The document locale of a letter: its wording locale and its formatting region as two choices.
//!
//! The rule is cletter's ([`cletter::resolve_document_locale`]); ccvl passes the explicit choices of
//! the person (the picked language, an explicit region, the workspace default locale) and the job's
//! location through, and holds no locale table of its own. The web calls the same function through
//! `@corbet-labs/cletter`, so the CLI, the TUI and the web agree.

pub use cletter::DocumentLocale;

/// Resolve a document's locale: `language` is the pick (a region inside it, `de-ch`, is explicit and
/// wins), `region` an explicit country, `location` the job's free-text location and `default_locale`
/// the workspace default (only its region counts). Locale IDs are lowercase, as cletter returns them.
#[must_use]
pub fn document_locale(
    language: Option<&str>,
    location: Option<&str>,
    region: Option<&str>,
    default_locale: Option<&str>,
) -> DocumentLocale {
    cletter::resolve_document_locale(language, location, region, default_locale)
}

#[cfg(test)]
mod tests {
    use super::document_locale;

    #[test]
    fn english_in_zurich_is_english_wording_with_swiss_formatting() {
        let d = document_locale(Some("en"), Some("Zürich, Zug"), None, None);
        assert_eq!(d.locale, "en-ch");
        assert_eq!(d.date_locale.as_deref(), Some("en-ch"));
        assert_eq!(d.region.as_deref(), Some("CH"));
    }

    #[test]
    fn an_explicit_region_beats_the_job_location() {
        let d = document_locale(Some("en-US"), Some("Zürich, Zug"), None, Some("de-CH"));
        assert_eq!(
            (d.locale.as_str(), d.region.as_deref()),
            ("en-us", Some("US"))
        );
    }

    #[test]
    fn german_in_the_us_keeps_german_wording_and_us_dates() {
        let d = document_locale(Some("de"), Some("New York, USA"), None, None);
        assert_eq!(d.locale, "de");
        assert_eq!(d.date_locale.as_deref(), Some("de-us"));
    }
}
