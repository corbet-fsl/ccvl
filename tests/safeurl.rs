//! The URL allow-list: the vectors come from `@cv/schemas/safe-url` of the `CareerVector` monorepo
//! (`lib/schemas/src/safe-url.vectors.json` is the same file); both sides must give the same answer.
#![cfg(feature = "jobid")]
use ccvl::safeurl::{is_safe_link_url, is_storable_job_url};
use serde_json::Value;

fn cases() -> Vec<Value> {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/vectors/safe-urls.json"
    ))
    .unwrap();
    serde_json::from_str::<Value>(&text).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn story_a_script_link_pasted_into_a_shared_workspace_is_refused_like_the_server_does() {
    for c in cases() {
        let url = c["url"].as_str().unwrap();
        assert_eq!(
            is_storable_job_url(url),
            c["storable"].as_bool().unwrap(),
            "storable {url:?}"
        );
        assert_eq!(
            is_safe_link_url(url),
            c["link"].as_bool().unwrap(),
            "link {url:?}"
        );
    }
}

#[test]
fn story_people_apply_by_email_so_mailto_is_stored_and_linked_but_ftp_is_not() {
    assert!(
        is_storable_job_url("mailto:jobs@example.com")
            && is_safe_link_url("mailto:jobs@example.com")
    );
    assert!(!is_storable_job_url("ftp://example.com/job"));
    assert!(
        is_storable_job_url("careervector-local://ws1/abc")
            && !is_safe_link_url("careervector-local://ws1/abc")
    );
}

#[test]
fn story_a_job_without_a_link_keeps_an_empty_url_that_is_never_rendered() {
    assert!(is_storable_job_url("") && !is_safe_link_url(""));
}
