//! The engine boundary beyond the vectors: typing, sync between two engines,
//! outgoing bytes, dry runs and the refusals the vectors cannot carry.
#![cfg(feature = "doc")]
#![allow(clippy::needless_pass_by_value)]

use ccvl_core::doc::{ActorClass, Doc};
use serde_json::{Value, json};

const BRANCH: &str = "cv_profile.en";

fn node(id: &str, kind: &str, data: Value, children: Vec<Value>) -> Value {
    let active: Vec<Value> = children.iter().map(|c| c["id"].clone()).collect();
    json!({ "id": id, "kind": kind, "tier": "HYBRID", "data": data,
            "selection": { "active": active }, "children": children })
}

fn seed() -> Doc {
    let tree = node(
        "root",
        "cv-root",
        json!({}),
        vec![node(
            "item",
            "list-item",
            json!({ "heading": "Engineer" }),
            vec![node(
                "b1",
                "bullet",
                json!({ "text": "Built \u{1F600} things" }),
                vec![],
            )],
        )],
    );
    let mut doc = Doc::with_client_id(1);
    doc.apply_op(&json!({ "kind": "cv_profile.replace", "lang": "en", "tree": tree, "format": { "font": "Archivo" } }))
        .expect("seed");
    doc
}

fn sibling(from: &Doc, client: u64) -> Doc {
    let mut doc = Doc::with_client_id(client);
    doc.receive(&from.encode_state()).expect("copy");
    doc
}

#[test]
fn type_text_counts_utf16_units() {
    let mut doc = seed();
    // "Built " is 6 units, the emoji 2 more: replace the word after them.
    let changed = doc
        .type_text(BRANCH, "root.item.b1", "text", 9..15, "systems")
        .expect("typed");
    assert_eq!(changed.nodes, vec!["b1"]);
    assert_eq!(
        doc.prose(BRANCH, "root.item.b1", "text").as_deref(),
        Some("Built \u{1F600} systems")
    );
    assert!(
        doc.type_text(BRANCH, "root.item.b1", "text", 0..99, "x")
            .is_err()
    );
    assert!(
        doc.type_text(BRANCH, "root.item.b1", "nope", 0..0, "x")
            .is_err()
    );
}

#[test]
fn type_text_upgrades_a_plain_data_object() {
    let mut doc = seed();
    doc.apply_op(
        &json!({ "kind": "node.create", "branch_path": BRANCH, "parent_path": "root.item",
        "node": node("b2", "bullet", json!({ "note": "n" }), vec![]) }),
    )
    .expect("create");
    doc.type_text(BRANCH, "root.item.b2", "text", 0..0, "hello")
        .expect("typed");
    assert_eq!(
        doc.prose(BRANCH, "root.item.b2", "text").as_deref(),
        Some("hello")
    );
}

#[test]
fn view_returns_the_node_shape() {
    let doc = seed();
    let b1 = doc.view(BRANCH, "root.item.b1").expect("view");
    assert_eq!(b1["id"], "b1");
    assert_eq!(b1["data"]["text"], "Built \u{1F600} things");
    assert_eq!(b1["selection"]["active"], json!([]));
    assert!(doc.view(BRANCH, "root.ghost").is_err());
    assert_eq!(
        doc.format_json(BRANCH).expect("format"),
        Some(json!({ "font": "Archivo" }))
    );
}

#[test]
fn two_engines_converge_through_outgoing_bytes() {
    let mut a = seed();
    let mut b = sibling(&a, 2);
    a.take_outgoing();
    assert!(
        b.take_outgoing().is_empty(),
        "loading a copy is not an edit"
    );

    a.type_text(BRANCH, "root.item.b1", "text", 0..0, ">> ")
        .expect("a types");
    a.apply_op(&json!({ "kind": "node.update", "branch_path": BRANCH, "node_path": "root.item", "patch": { "tier": "OPEN" } }))
        .expect("a updates");
    let bytes = a.take_outgoing();
    assert_ne!(bytes.len(), 0);
    assert_eq!(a.take_outgoing().len(), 0, "outgoing is taken once");

    let changed = b.receive(&bytes).expect("b receives");
    assert_eq!(changed.nodes, vec!["b1", "item"]);
    assert!(
        b.take_outgoing().is_empty(),
        "received updates are not echoed"
    );
    assert_eq!(a.shape(), b.shape());
}

#[test]
fn concurrent_prose_edits_merge() {
    let mut a = seed();
    let mut b = sibling(&a, 2);
    a.take_outgoing();
    a.type_text(BRANCH, "root.item.b1", "text", 0..0, "A:")
        .expect("a");
    let end = u32::try_from("Built \u{1F600} things".encode_utf16().count()).expect("small");
    b.type_text(BRANCH, "root.item.b1", "text", end..end, ":B")
        .expect("b");
    let (from_a, from_b) = (a.take_outgoing(), b.take_outgoing());
    a.receive(&from_b).expect("a gets b");
    b.receive(&from_a).expect("b gets a");
    assert_eq!(a.shape(), b.shape());
    assert_eq!(
        a.prose(BRANCH, "root.item.b1", "text").as_deref(),
        Some("A:Built \u{1F600} things:B")
    );
}

#[test]
fn dry_run_changes_nothing() {
    let mut doc = seed();
    doc.take_outgoing();
    let before = doc.shape();
    let op = json!({ "kind": "node.delete", "branch_path": BRANCH, "node_path": "root.item.b1" });
    let changed = doc.dry_run_as(&op, ActorClass::Human).expect("would apply");
    assert_eq!(changed.nodes, vec!["item"]);
    assert_eq!(doc.shape(), before);
    assert_eq!(doc.take_outgoing().len(), 0);
    let bad = json!({ "kind": "node.delete", "branch_path": BRANCH, "node_path": "root.ghost.x" });
    assert!(doc.dry_run_as(&bad, ActorClass::Human).is_err());
}

#[test]
fn a_move_into_its_own_subtree_is_refused() {
    let mut doc = seed();
    doc.apply_op(
        &json!({ "kind": "node.create", "branch_path": BRANCH, "parent_path": "root.item.b1",
        "node": node("s1", "sub", json!({ "text": "s" }), vec![]) }),
    )
    .expect("create");
    let before = doc.shape();
    let err = doc
        .apply_op(&json!({ "kind": "node.move", "branch_path": BRANCH, "from_path": "root.item.b1", "to_parent_path": "root.item.b1.s1" }))
        .expect_err("refused");
    assert!(err.message().contains("is inside the moved node"), "{err}");
    assert_eq!(doc.shape(), before);
}

#[test]
fn apply_op_json_and_ids_from_the_source() {
    let mut doc = seed().with_id_source(|| "fixed-id-01".to_string());
    let changed = doc
        .apply_op_json(r#"{"kind":"node.create","branch_path":"cv_profile.en","parent_path":"root.item","node":{"kind":"bullet","tier":"HYBRID","data":{"text":"x"}}}"#)
        .expect("created");
    assert_eq!(changed.nodes, vec!["fixed-id-01", "item"]);
    assert!(doc.apply_op_json("{not json").is_err());
}

#[test]
fn a_surrogate_pair_edit_stays_valid_text() {
    let mut doc = seed();
    doc.apply_op(
        &json!({ "kind": "node.update", "branch_path": BRANCH, "node_path": "root.item.b1",
        "patch": { "data": { "text": "\u{1F600}" } } }),
    )
    .expect("one emoji");
    doc.apply_op(
        &json!({ "kind": "node.update", "branch_path": BRANCH, "node_path": "root.item.b1",
        "patch": { "data": { "text": "\u{1F601}" } } }),
    )
    .expect("another emoji");
    assert_eq!(
        doc.prose(BRANCH, "root.item.b1", "text").as_deref(),
        Some("\u{1F601}")
    );
}

#[test]
fn profile_json_and_pending_state() {
    let mut doc = seed();
    assert_eq!(
        doc.profile_json("cv_profile").expect("json")["en"]["tree"]["id"],
        "root"
    );
    assert!(doc.profile_json("jobs").is_err());
    assert!(!doc.has_pending());
    // An update whose history this document lacks stays pending.
    let mut a = seed();
    let first = a.take_outgoing();
    let _ = first;
    a.type_text(BRANCH, "root.item.b1", "text", 0..0, "x")
        .expect("a");
    let later = a.take_outgoing();
    let mut empty = Doc::new();
    empty.receive(&later).expect("accepted as pending");
    assert!(empty.has_pending());
    let _ = doc.generate_id();
}
