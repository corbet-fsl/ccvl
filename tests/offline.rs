#![cfg(feature = "core")]
//! The offline policy against the shared JSON vectors (the TypeScript twin in the CareerVector
//! workspace-client reads a copy of `tests/vectors/offline.json`).

use ccvl::offline::{FieldKind, Resolution, STALE_AFTER_MS, resolve};
use serde_json::Value;

#[test]
fn vectors_match_the_policy() {
    let v: Value = serde_json::from_str(include_str!("vectors/offline.json")).unwrap();
    assert_eq!(v["threshold_ms"].as_u64(), Some(STALE_AFTER_MS));
    let cases = v["cases"].as_array().unwrap();
    assert!(cases.len() >= 30);
    for c in cases {
        let s = |k: &str| c[k].as_str().unwrap();
        let kind = if s("kind") == "text" {
            FieldKind::Text
        } else {
            FieldKind::Single
        };
        let got = resolve(
            kind,
            s("base"),
            s("mine"),
            s("theirs"),
            c["age_ms"].as_u64().unwrap(),
        );
        let want = match s("expect") {
            "apply" => Resolution::Apply,
            "keep" => Resolution::Keep,
            "merged" => Resolution::Merged(s("text").to_string()),
            other => panic!("unknown expectation {other}"),
        };
        assert_eq!(got, want, "{}", s("name"));
    }
}

/// Story S1-S5 in one pass: the same edit, aging, with and without a change elsewhere.
#[test]
fn an_edit_ages_from_applied_to_kept_only_when_something_changed_elsewhere() {
    for age in [
        0,
        1_000,
        STALE_AFTER_MS,
        STALE_AFTER_MS + 1,
        3 * 24 * 3_600_000,
    ] {
        assert_eq!(
            resolve(FieldKind::Single, "\"a\"", "\"b\"", "\"a\"", age),
            Resolution::Apply
        );
        let changed = resolve(FieldKind::Single, "\"a\"", "\"b\"", "\"c\"", age);
        assert_eq!(changed == Resolution::Keep, age > STALE_AFTER_MS);
    }
}
