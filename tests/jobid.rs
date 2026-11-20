//! The job id language under the `jobid` feature alone: golden values of `@cv/jobcache` `ids.ts`.
#![cfg(feature = "jobid")]

use ccvl::{URL_RULE_VERSION, is_email_shaped, job_id_from_url, normalize_url};

const URL: &str = "https://EXAMPLE.com/jobs/42/?utm_source=news#top";

#[test]
fn normalizes_like_typescript() {
    assert_eq!(normalize_url(URL).unwrap(), "https://example.com/jobs/42");
}

#[test]
fn job_id_matches_typescript_golden_vector() {
    assert_eq!(job_id_from_url(URL).unwrap(), "job_ttiaykbb4hwpf7ff");
}

/// The shared fixture (also read by `@cv/jobcache/ids`): every row byte for byte, ids included.
#[test]
fn shared_fixture_rows_match_the_typescript_twin() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/normalize-url.json")).unwrap();
    assert_eq!(fixture["rule_version"], URL_RULE_VERSION);
    assert_eq!(URL_RULE_VERSION, 2);
    let cases = fixture["cases"].as_array().unwrap();
    assert!(cases.len() >= 40);
    for c in cases {
        let (name, input) = (c["name"].as_str().unwrap(), c["input"].as_str().unwrap());
        assert_eq!(
            normalize_url(input).unwrap(),
            c["normalized"].as_str().unwrap(),
            "{name}"
        );
        assert_eq!(
            job_id_from_url(input).unwrap(),
            c["job_id"].as_str().unwrap(),
            "{name}"
        );
    }
}

#[test]
fn personal_parameters_are_dropped_and_job_id_parameters_are_kept() {
    let b = "https://jobs.example.com/p/42";
    assert_eq!(
        normalize_url(&format!("{b}?e=alice@example.com&id=7")).unwrap(),
        format!("{b}?id=7")
    );
    assert_eq!(
        normalize_url(&format!("{b}?mail=alice%40example.com&id=7")).unwrap(),
        format!("{b}?id=7")
    );
    assert_eq!(normalize_url(&format!("{b}?contact=a@b.co")).unwrap(), b);
    assert_eq!(
        normalize_url(&format!("{b}?Token=a&API_KEY=b&SessionId=c&Sig=d&id=1")).unwrap(),
        format!("{b}?id=1")
    );
    for name in [
        "token",
        "access_token",
        "auth",
        "authorization",
        "password",
        "pass",
        "pwd",
        "secret",
        "signature",
        "sig",
        "session",
        "sessionid",
        "apikey",
        "api_key",
    ] {
        assert_eq!(
            normalize_url(&format!("{b}?{name}=x&id=1")).unwrap(),
            format!("{b}?id=1"),
            "{name}"
        );
    }
}

/// NEGATIVE CONTROLS: these stay.
#[test]
fn key_id_uid_sid_jk_and_non_email_values_are_kept() {
    let b = "https://jobs.example.com/p/42";
    for name in ["key", "id", "uid", "sid", "jk"] {
        assert_eq!(
            normalize_url(&format!("{b}?{name}=abc123")).unwrap(),
            format!("{b}?{name}=abc123"),
            "{name}"
        );
    }
    assert_eq!(
        normalize_url(&format!("{b}?q=a@b&id=1")).unwrap(),
        format!("{b}?id=1&q=a%40b")
    );
    assert_eq!(
        normalize_url(&format!("{b}?q=a@b@c.com&id=1")).unwrap(),
        format!("{b}?id=1&q=a%40b%40c.com")
    );
    assert_eq!(
        normalize_url(&format!("{b}?tokenizer=a&passport=b&id=1")).unwrap(),
        format!("{b}?id=1&passport=b&tokenizer=a")
    );
}

#[test]
fn email_shape() {
    for ok in [
        "a@b.co",
        "alice.b+jobs@Example.CO.UK",
        "j\u{fc}rgen@b\u{fc}cher.de",
    ] {
        assert!(is_email_shaped(ok), "{ok}");
    }
    for no in [
        "",
        "a@b",
        "@b.co",
        "a@.co",
        "a@b.",
        "a@b@c.com",
        "a b@c.de",
        "plain",
        "a@",
    ] {
        assert!(!is_email_shaped(no), "{no:?}");
    }
}
