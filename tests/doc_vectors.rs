//! Compatibility vectors: the engine against what `@cv/mutations` does.
//!
//! The vectors in `tests/vectors/` are generated from the real `applyOp` by
//! `lib/mutations/scripts/ccvl-vectors.ts` in `CareerVector` (commit recorded in
//! each file's `source`). Every case starts from a Yjs update, applies one op or
//! one peer update, and compares the resulting structure (`Y.Map` / `Y.Array` /
//! `Y.Text` / plain value), the changed ids, or the exact refusal.
#![cfg(feature = "doc")]

use std::collections::HashMap;

use base64::Engine as _;
use ccvl_core::doc::{ActorClass, Changed, Doc, OP_CATALOG_VERSION};
use serde_json::Value;

const STATES: &str = include_str!("vectors/states.json");
const OPS: &str = include_str!("vectors/v1-ops.json");
const ERRORS: &str = include_str!("vectors/v3-errors.json");
const IDS: &str = include_str!("vectors/v6-ids.json");

fn decode(text: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .expect("vector bytes are base64")
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).expect("vector file is JSON")
}

fn states() -> HashMap<String, Vec<u8>> {
    let file = parse(STATES);
    assert_eq!(file["source"]["op_catalog_version"], OP_CATALOG_VERSION);
    file["states"]
        .as_object()
        .expect("states")
        .iter()
        .map(|(k, v)| (k.clone(), decode(v.as_str().expect("state"))))
        .collect()
}

/// A document holding `pre`, whose generated ids are `$NEW1`, `$NEW2`, ...
fn engine(pre: &[u8]) -> Doc {
    let mut counter = 0;
    let mut doc = Doc::new().with_id_source(move || {
        counter += 1;
        format!("$NEW{counter}")
    });
    doc.receive(pre).expect("pre-state loads");
    doc
}

fn first_diff(ours: &Value, theirs: &Value, path: &str) -> Option<String> {
    match (ours, theirs) {
        (Value::Object(mine), Value::Object(other)) => {
            for (key, value) in mine {
                let here = format!("{path}/{key}");
                match other.get(key) {
                    Some(expected) => {
                        if let Some(found) = first_diff(value, expected, &here) {
                            return Some(found);
                        }
                    }
                    None => return Some(format!("{here}: only in the engine")),
                }
            }
            other
                .keys()
                .find(|k| !mine.contains_key(*k))
                .map(|k| format!("{path}/{k}: only in the expectation"))
        }
        (Value::Array(mine), Value::Array(other)) => {
            for (i, (value, expected)) in mine.iter().zip(other).enumerate() {
                if let Some(found) = first_diff(value, expected, &format!("{path}/{i}")) {
                    return Some(found);
                }
            }
            (mine.len() != other.len())
                .then(|| format!("{path}: length {} vs expected {}", mine.len(), other.len()))
        }
        _ => (ours != theirs).then(|| format!("{path}: engine {ours} vs expected {theirs}")),
    }
}

fn assert_shape(case: &str, what: &str, doc: &Doc, expected: &Value) {
    if let Some(diff) = first_diff(&doc.shape(), expected, "") {
        panic!("{case} ({what}): structure differs at {diff}");
    }
}

fn expected_changed(expect: &Value) -> Changed {
    let list = |key: &str| -> Vec<String> {
        expect["changed"][key]
            .as_array()
            .expect("changed list")
            .iter()
            .map(|v| v.as_str().expect("id").to_string())
            .collect()
    };
    Changed {
        nodes: list("nodes"),
        formats: list("formats"),
    }
}

fn actor(case: &Value) -> ActorClass {
    ActorClass::parse(case["actor"].as_str().expect("actor")).expect("known actor")
}

/// Run every refusal-free op case; returns how many ran.
fn run_ops(file: &str, states: &HashMap<String, Vec<u8>>) -> usize {
    let file = parse(file);
    let out_dir = std::env::var_os("CCVL_VECTOR_OUT").map(std::path::PathBuf::from);
    let cases = file["cases"].as_array().expect("cases");
    for case in cases {
        let name = case["name"].as_str().expect("name");
        let pre = &states[case["pre"].as_str().expect("pre")];
        let expect = &case["expect"];
        let mut doc = engine(pre);
        let changed = doc
            .apply_op_as(&case["op"], actor(case))
            .unwrap_or_else(|e| panic!("{name}: engine refused: {e}"));
        assert_shape(name, "op", &doc, &expect["shape"]);
        assert_eq!(changed, expected_changed(expect), "{name}: changed ids");
        let outgoing = doc.take_outgoing();
        assert!(
            changed.is_empty() || !outgoing.is_empty(),
            "{name}: a change must leave bytes to send"
        );
        if let (Some(dir), None, false) = (&out_dir, case.get("random_ids"), outgoing.is_empty()) {
            std::fs::create_dir_all(dir).expect("out dir");
            let text = base64::engine::general_purpose::STANDARD.encode(&outgoing);
            std::fs::write(dir.join(format!("{name}.b64")), text).expect("write update");
        }
        // The bytes Yjs produced must decode here and give the same document.
        if let Some(diff) = expect.get("diff_update").and_then(Value::as_str) {
            let mut peer = engine(pre);
            peer.receive(&decode(diff))
                .unwrap_or_else(|e| panic!("{name}: Yjs update refused: {e}"));
            assert_shape(name, "Yjs update", &peer, &expect["shape"]);
        }
    }
    cases.len()
}

#[test]
fn v1_ops_and_v4_shape_match_yjs() {
    let states = states();
    let count = run_ops(OPS, &states);
    assert!(count >= 80, "expected the full op suite, ran {count}");
}

#[test]
fn v3_refusals_match_yjs() {
    let states = states();
    let file = parse(ERRORS);
    let cases = file["cases"].as_array().expect("cases");
    for case in cases {
        let name = case["name"].as_str().expect("name");
        let pre = &states[case["pre"].as_str().expect("pre")];
        let expect = &case["expect"];
        let mut doc = engine(pre);
        let error = doc
            .apply_op_as(&case["op"], actor(case))
            .expect_err(&format!("{name}: the engine must refuse"));
        assert_eq!(
            error.message(),
            expect["error"].as_str().expect("error"),
            "{name}: reason"
        );
        assert_shape(name, "after refusal", &doc, &expect["shape"]);
    }
    assert!(cases.len() >= 50, "ran {}", cases.len());
}

#[test]
fn v6_changed_ids_of_peer_updates_match_yjs() {
    let states = states();
    let file = parse(IDS);
    let cases = file["cases"].as_array().expect("cases");
    for case in cases {
        let name = case["name"].as_str().expect("name");
        let pre = &states[case["pre"].as_str().expect("pre")];
        let expect = &case["expect"];
        let mut doc = engine(pre);
        let changed = doc
            .receive(&decode(case["update"].as_str().expect("update")))
            .unwrap_or_else(|e| panic!("{name}: receive failed: {e}"));
        assert_shape(name, "receive", &doc, &expect["shape"]);
        assert_eq!(changed, expected_changed(expect), "{name}: changed ids");
        assert!(
            doc.take_outgoing().is_empty(),
            "{name}: remote updates are not echoed"
        );
    }
    assert!(cases.len() >= 12, "ran {}", cases.len());
}
