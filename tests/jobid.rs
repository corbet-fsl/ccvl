//! The job id language under the `jobid` feature alone: golden values of `@cv/jobcache` `ids.ts`.
#![cfg(feature = "jobid")]

use ccvl::{job_id_from_url, normalize_url};

const URL: &str = "https://EXAMPLE.com/jobs/42/?utm_source=news#top";

#[test]
fn normalizes_like_typescript() {
    assert_eq!(normalize_url(URL).unwrap(), "https://example.com/jobs/42");
}

#[test]
fn job_id_matches_typescript_golden_vector() {
    assert_eq!(job_id_from_url(URL).unwrap(), "job_ttiaykbb4hwpf7ff");
}
