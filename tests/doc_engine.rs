//! The engine boundary beyond the vectors: typing, sync between two engines,
//! outgoing bytes, refusals that must change nothing, deep trees, and the
//! checks the vectors cannot carry.
#![cfg(feature = "doc")]
#![allow(clippy::needless_pass_by_value)]

use ccvl_core::doc::{ActorClass, Doc, NodeRef, inspect_update};
use serde_json::{Value, json};
use yrs::{Array, ArrayPrelim, Doc as YDoc, Map, MapPrelim, ReadTxn, Transact};

const BRANCH: &str = "cv_profile.en";

fn node(id: &str, kind: &str, data: Value, children: Vec<Value>) -> Value {
    node_with_tier(id, kind, "HYBRID", data, children)
}

fn node_with_tier(id: &str, kind: &str, tier: &str, data: Value, children: Vec<Value>) -> Value {
    let active: Vec<Value> = children.iter().map(|c| c["id"].clone()).collect();
    json!({ "id": id, "kind": kind, "tier": tier, "data": data,
            "selection": { "active": active }, "children": children })
}

fn tree() -> Value {
    node(
        "root",
        "cv-root",
        json!({}),
        vec![
            node_with_tier(
                "info",
                "personal-info",
                "FIXED",
                json!({ "name": "Ada" }),
                vec![],
            ),
            node(
                "item",
                "list-item",
                json!({ "heading": "Engineer" }),
                vec![node(
                    "b1",
                    "bullet",
                    json!({ "text": "Built \u{1F600} things" }),
                    vec![],
                )],
            ),
        ],
    )
}

fn seed() -> Doc {
    let mut doc = Doc::with_client_id(1);
    doc.apply_op(&json!({ "kind": "cv_profile.replace", "lang": "en", "tree": tree(), "format": { "font": "Archivo" } }))
        .expect("seed");
    doc
}

fn sibling(from: &Doc, client: u64) -> Doc {
    let mut doc = Doc::with_client_id(client);
    doc.receive(&from.encode_state()).expect("copy");
    doc
}

fn at(path: &str) -> NodeRef {
    node_ref(BRANCH, path)
}

fn node_ref(branch: &str, path: &str) -> NodeRef {
    NodeRef::new(branch, path.split('.').map(str::to_string).collect())
}

fn human(
    doc: &mut Doc,
    path: &str,
    key: &str,
    range: std::ops::Range<u32>,
    text: &str,
) -> Result<(), String> {
    doc.type_text(BRANCH, path, key, range, text, ActorClass::Human)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn outgoing(doc: &mut Doc) -> Vec<u8> {
    doc.take_outgoing().expect("outgoing")
}

#[test]
fn type_text_counts_utf16_units() {
    let mut doc = seed();
    let changed = doc
        .type_text(
            BRANCH,
            "root.item.b1",
            "text",
            9..15,
            "systems",
            ActorClass::Human,
        )
        .expect("typed");
    assert_eq!(changed.nodes, vec![at("root.item.b1")]);
    assert_eq!(
        doc.prose(BRANCH, "root.item.b1", "text").as_deref(),
        Some("Built \u{1F600} systems")
    );
}

#[test]
fn a_refused_type_text_changes_nothing_and_emits_no_bytes() {
    let mut doc = seed();
    outgoing(&mut doc);
    let before = doc.shape();
    // 6..7 starts at the emoji's high surrogate, 7..8 would split it.
    for (path, key, range, text, why) in [
        ("root.item.b1", "text", 0..99, "x", "outside the text"),
        (
            "root.item.b1",
            "text",
            std::ops::Range { start: 9, end: 3 },
            "x",
            "outside the text",
        ),
        ("root.item.b1", "text", 7..8, "x", "splits a surrogate pair"),
        ("root.item.b1", "text", 0..7, "x", "splits a surrogate pair"),
        ("root.item.b1", "heading", 0..0, "x", "not a prose field"),
        ("root.ghost", "text", 0..0, "x", "not found"),
    ] {
        let err = human(&mut doc, path, key, range, text).expect_err(why);
        assert!(err.contains(why), "{why}: {err}");
        assert_eq!(doc.shape(), before, "{why}: the document changed");
        assert_eq!(outgoing(&mut doc).len(), 0, "{why}: bytes were emitted");
    }
}

#[test]
fn type_text_protects_fixed_nodes_from_non_humans() {
    let mut doc = seed();
    outgoing(&mut doc);
    doc.apply_op(
        &json!({ "kind": "node.update", "branch_path": BRANCH, "node_path": "root.item.b1",
        "patch": { "tier": "FIXED" } }),
    )
    .expect("fix it");
    outgoing(&mut doc);
    let before = doc.shape();
    let err = doc
        .type_text(BRANCH, "root.item.b1", "text", 0..0, "x", ActorClass::Agent)
        .expect_err("protected");
    assert!(err.message().starts_with("tier_protected: agent"), "{err}");
    assert_eq!(doc.shape(), before);
    assert_eq!(outgoing(&mut doc).len(), 0);
    doc.type_text(BRANCH, "root.item.b1", "text", 0..0, "x", ActorClass::Human)
        .expect("humans may");
}

#[test]
fn type_text_seeds_a_y_text_from_a_plain_string() {
    let mut doc = seed();
    // A legacy bullet whose data is a plain object holding a string.
    doc.apply_op(
        &json!({ "kind": "node.update", "branch_path": BRANCH, "node_path": "root.item.b1",
        "patch": { "kind": "note", "data": { "text": "Plain words" } } }),
    )
    .expect("flatten");
    doc.apply_op(
        &json!({ "kind": "node.update", "branch_path": BRANCH, "node_path": "root.item.b1",
        "patch": { "kind": "bullet" } }),
    )
    .expect("back to prose, data stays plain");
    assert!(
        doc.prose(BRANCH, "root.item.b1", "text").is_none(),
        "data is still a plain object"
    );
    outgoing(&mut doc);

    // A refused range must not upgrade anything.
    let before = doc.shape();
    assert!(human(&mut doc, "root.item.b1", "text", 0..99, "x").is_err());
    assert_eq!(doc.shape(), before);
    assert_eq!(outgoing(&mut doc).len(), 0);

    human(&mut doc, "root.item.b1", "text", 6..11, "text").expect("typed");
    assert_eq!(
        doc.prose(BRANCH, "root.item.b1", "text").as_deref(),
        Some("Plain text")
    );
}

#[test]
fn type_text_creates_the_text_of_a_prose_node_that_has_none() {
    let mut doc = seed();
    doc.apply_op(
        &json!({ "kind": "node.create", "branch_path": BRANCH, "parent_path": "root.item",
        "node": node("b2", "bullet", json!({ "note": "n" }), vec![]) }),
    )
    .expect("create");
    human(&mut doc, "root.item.b2", "text", 0..0, "hello").expect("typed");
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
fn changed_nodes_carry_their_location() {
    // The same ids live in two language trees and under different parents.
    let mut doc = seed();
    doc.apply_op(&json!({ "kind": "cv_profile.replace", "lang": "de", "tree": tree() }))
        .expect("de");
    doc.apply_op(
        &json!({ "kind": "node.create", "branch_path": BRANCH, "parent_path": "root.info",
        "node": node("b1", "bullet", json!({ "text": "same id, other parent" }), vec![]) }),
    )
    .expect("twin");
    let changed = doc
        .apply_op(&json!({ "kind": "node.update", "branch_path": "cv_profile.de", "node_path": "root.item.b1",
            "patch": { "tier": "OPEN" } }))
        .expect("update");
    assert_eq!(
        changed.nodes,
        vec![node_ref("cv_profile.de", "root.item.b1")]
    );
    let twin = doc
        .apply_op(
            &json!({ "kind": "node.update", "branch_path": BRANCH, "node_path": "root.info.b1",
            "patch": { "tier": "OPEN" } }),
        )
        .expect("twin update");
    assert_eq!(twin.nodes, vec![at("root.info.b1")]);
}

#[test]
fn two_engines_converge_through_outgoing_bytes() {
    let mut a = seed();
    let mut b = sibling(&a, 2);
    outgoing(&mut a);
    assert_eq!(outgoing(&mut b).len(), 0, "loading a copy is not an edit");

    a.type_text(
        BRANCH,
        "root.item.b1",
        "text",
        0..0,
        ">> ",
        ActorClass::Human,
    )
    .expect("a types");
    a.apply_op(&json!({ "kind": "node.update", "branch_path": BRANCH, "node_path": "root.item", "patch": { "tier": "OPEN" } }))
        .expect("a updates");
    let bytes = outgoing(&mut a);
    assert_ne!(bytes.len(), 0);
    assert_eq!(outgoing(&mut a).len(), 0, "outgoing is taken once");

    let changed = b.receive(&bytes).expect("b receives");
    assert_eq!(changed.nodes, vec![at("root.item"), at("root.item.b1")]);
    assert_eq!(outgoing(&mut b).len(), 0, "received updates are not echoed");
    assert_eq!(a.shape(), b.shape());
}

#[test]
fn restored_outgoing_bytes_come_first() {
    let mut a = seed();
    let first = outgoing(&mut a);
    a.type_text(BRANCH, "root.item.b1", "text", 0..0, "x", ActorClass::Human)
        .expect("typed");
    a.restore_outgoing(first.clone());
    let merged = outgoing(&mut a);
    let mut b = Doc::new();
    b.receive(&merged)
        .expect("both updates arrive in one piece");
    assert!(!b.has_pending());
    assert_eq!(b.shape(), a.shape());
    a.restore_outgoing(Vec::new());
    assert_eq!(outgoing(&mut a).len(), 0);
}

#[test]
fn concurrent_prose_edits_merge() {
    let mut a = seed();
    let mut b = sibling(&a, 2);
    outgoing(&mut a);
    a.type_text(
        BRANCH,
        "root.item.b1",
        "text",
        0..0,
        "A:",
        ActorClass::Human,
    )
    .expect("a");
    let end = u32::try_from("Built \u{1F600} things".encode_utf16().count()).expect("small");
    b.type_text(
        BRANCH,
        "root.item.b1",
        "text",
        end..end,
        ":B",
        ActorClass::Human,
    )
    .expect("b");
    let (from_a, from_b) = (outgoing(&mut a), outgoing(&mut b));
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
    outgoing(&mut doc);
    let before = doc.shape();
    let op = json!({ "kind": "node.delete", "branch_path": BRANCH, "node_path": "root.item.b1" });
    let changed = doc.dry_run_as(&op, ActorClass::Human).expect("would apply");
    assert_eq!(changed.nodes, vec![at("root.item")]);
    assert_eq!(doc.shape(), before);
    assert_eq!(outgoing(&mut doc).len(), 0);
    let bad = json!({ "kind": "node.delete", "branch_path": BRANCH, "node_path": "root.ghost.x" });
    assert!(doc.dry_run_as(&bad, ActorClass::Human).is_err());
}

#[test]
fn refused_ops_change_nothing_and_emit_no_bytes() {
    let mut doc = seed();
    outgoing(&mut doc);
    let before = doc.shape();
    let ops = [
        // selection.update validates before it writes
        json!({ "kind": "selection.update", "branch_path": BRANCH, "node_path": "root.item",
                "delta": { "add": ["ghost"] } }),
        json!({ "kind": "selection.update", "branch_path": BRANCH, "node_path": "root.item",
                "delta": { "add": [5] } }),
        json!({ "kind": "selection.update", "branch_path": BRANCH, "node_path": "root.item",
                "delta": { "setActive": "b1" } }),
        // node.create: ids are strings, and a duplicate is refused before any write
        json!({ "kind": "node.create", "branch_path": BRANCH, "parent_path": "root.item",
                "node": { "id": 5, "kind": "bullet", "tier": "HYBRID" } }),
        json!({ "kind": "node.create", "branch_path": BRANCH, "parent_path": "root.item",
                "node": node("b1", "bullet", json!({}), vec![]) }),
        // translate refuses before it creates a language map or a children array
        json!({ "kind": "translate", "source_path": "cv_profile.en.tree.root.item", "target_path": "cv_profile.fr.tree.root.item",
                "source_lang": "en", "target_lang": "fr", "translated_node": node("item", "list-item", json!({}), vec![]) }),
        // the server's rules: same profile, different languages, a valid node
        json!({ "kind": "translate", "source_path": "cv_profile.en.tree.root.item", "target_path": "cl_profile.de.tree.root.item",
                "source_lang": "en", "target_lang": "de", "translated_node": node("item", "list-item", json!({}), vec![]) }),
        // tailored.* belongs to the jobs sub-document
        json!({ "kind": "tailored.snapshot", "job_id": "j", "branch_path": BRANCH }),
        json!({ "kind": "tailored.promote", "job_id": "j", "node_path": "root.item", "branch_path": BRANCH }),
    ];
    for op in &ops {
        let err = doc.apply_op(op).expect_err("refused");
        assert_ne!(err.message().len(), 0);
        assert_eq!(doc.shape(), before, "{op}: the document changed");
        assert_eq!(outgoing(&mut doc).len(), 0, "{op}: bytes were emitted");
    }
    let same_language = json!({ "kind": "translate", "source_path": "cv_profile.en.tree.root.item",
        "target_path": "cv_profile.en.tree.root.item", "source_lang": "en", "target_lang": "en",
        "translated_node": node("item", "list-item", json!({}), vec![]) });
    assert!(doc.apply_op(&same_language).is_err());
    let tailored = doc
        .apply_op(&json!({ "kind": "tailored.snapshot", "job_id": "j", "branch_path": BRANCH }))
        .expect_err("refused");
    assert!(
        tailored.message().contains("jobs sub-document"),
        "{tailored}"
    );
}

#[test]
fn translate_follows_the_server_rules() {
    let mut doc = seed();
    doc.apply_op(&json!({ "kind": "cv_profile.replace", "lang": "de", "tree": tree() }))
        .expect("de");
    let translated = node(
        "item",
        "list-item",
        json!({ "heading": "Ingenieurin" }),
        vec![],
    );
    let op = |source_lang: &str, target_lang: &str, target: &str| {
        json!({ "kind": "translate", "source_path": "cv_profile.en.tree.root.item", "target_path": target,
                "source_lang": source_lang, "target_lang": target_lang, "translated_node": translated })
    };
    for (op, why) in [
        (
            op("en", "en", "cv_profile.de.tree.root.item"),
            "must differ",
        ),
        (
            op("en", "de", "cv_profile.en.tree.root.item"),
            "different language trees",
        ),
        (
            op("en", "de", "cl_profile.de.tree.root.item"),
            "same profile",
        ),
        (
            op("e", "de", "cv_profile.de.tree.root.item"),
            "at least 2 characters",
        ),
    ] {
        let err = doc.apply_op(&op).expect_err(why);
        assert!(err.message().contains(why), "{why}: {err}");
    }
    let mut bad_node = op("en", "de", "cv_profile.de.tree.root.item");
    bad_node["translated_node"]["tier"] = json!("SECRET");
    assert!(
        doc.apply_op(&bad_node)
            .expect_err("tier")
            .message()
            .contains("tier FIXED, HYBRID or OPEN")
    );
    doc.apply_op(&op("en", "de", "cv_profile.de.tree.root.item"))
        .expect("valid");
}

#[test]
fn a_move_into_its_own_subtree_is_refused() {
    let mut doc = seed();
    doc.apply_op(
        &json!({ "kind": "node.create", "branch_path": BRANCH, "parent_path": "root.item.b1",
        "node": node("s1", "sub", json!({ "text": "s" }), vec![]) }),
    )
    .expect("create");
    outgoing(&mut doc);
    let before = doc.shape();
    let err = doc
        .apply_op(&json!({ "kind": "node.move", "branch_path": BRANCH, "from_path": "root.item.b1", "to_parent_path": "root.item.b1.s1" }))
        .expect_err("refused");
    assert!(err.message().contains("is inside the moved node"), "{err}");
    assert_eq!(doc.shape(), before);
    assert_eq!(outgoing(&mut doc).len(), 0);
}

#[test]
fn apply_op_json_and_ids_from_the_source() {
    let mut doc = seed().with_id_source(|| "fixed-id-01".to_string());
    let changed = doc
        .apply_op_json(r#"{"kind":"node.create","branch_path":"cv_profile.en","parent_path":"root.item","node":{"kind":"bullet","tier":"HYBRID","data":{"text":"x"}}}"#)
        .expect("created");
    assert_eq!(
        changed.nodes,
        vec![at("root.item"), at("root.item.fixed-id-01")]
    );
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
fn profile_json_pending_state_and_update_inspection() {
    let mut doc = seed();
    assert_eq!(
        doc.profile_json("cv_profile").expect("json")["en"]["tree"]["id"],
        "root"
    );
    assert!(doc.profile_json("jobs").is_err());
    assert!(!doc.has_pending());
    // An update whose history this document lacks stays pending.
    let mut a = seed();
    outgoing(&mut a);
    a.type_text(BRANCH, "root.item.b1", "text", 0..0, "x", ActorClass::Human)
        .expect("a");
    let later = outgoing(&mut a);
    let mut empty = Doc::new();
    empty.receive(&later).expect("accepted as pending");
    assert!(empty.has_pending());

    let info = inspect_update(&later).expect("inspect");
    assert!(
        info.roots.is_empty(),
        "a nested edit names no root: {:?}",
        info.roots
    );
    assert_eq!(info.clients.len(), 1);
    assert!(a.knows_client(*info.clients.iter().next().expect("client")));
    assert!(!Doc::new().knows_client(*info.clients.iter().next().expect("client")));
    let seed_bytes = seed().encode_state();
    let seed_info = inspect_update(&seed_bytes).expect("inspect");
    assert!(seed_info.roots.contains("cv_profile"));
    assert!(!seed_info.empty);
    assert!(inspect_update(&[0, 0]).expect("empty update").empty);
    let _ = doc.generate_id();
}

/// A document whose tree nests `depth` levels deep, built the way a hostile peer could.
fn deep_peer_update(depth: usize) -> Vec<u8> {
    let peer = YDoc::new();
    let root = peer.get_or_insert_map("cv_profile");
    let mut txn = peer.transact_mut();
    let lang = root.insert(&mut txn, "en", MapPrelim::default());
    let tree = lang.insert(&mut txn, "tree", MapPrelim::default());
    let mut current = tree;
    for level in 0..depth {
        current.insert(&mut txn, "id", format!("n{level}"));
        current.insert(&mut txn, "kind", "section");
        current.insert(&mut txn, "tier", "HYBRID");
        let kids = current.insert(&mut txn, "children", ArrayPrelim::default());
        current = kids.push_back(&mut txn, MapPrelim::default());
    }
    drop(txn);
    peer.transact()
        .encode_state_as_update_v1(&yrs::StateVector::default())
}

#[test]
fn a_tree_that_is_too_deep_is_refused_not_a_stack_overflow() {
    let mut doc = Doc::new();
    let changed = doc
        .receive(&deep_peer_update(2_000))
        .expect("frames are accepted; reading is guarded");
    assert_ne!(changed.nodes.len(), 0);
    let leaf_path = (0..2_000).map(|i| format!("n{i}")).collect::<Vec<_>>();
    for err in [
        doc.view(BRANCH, "n0").expect_err("view").to_string(),
        doc.tree_json(BRANCH).expect_err("tree").to_string(),
        doc.profile_json("cv_profile")
            .expect_err("profile")
            .to_string(),
        doc.apply_op(&json!({ "kind": "node.move", "branch_path": BRANCH,
            "from_path": "n0.n1", "to_parent_path": "n0" }))
            .expect_err("move")
            .to_string(),
        doc.apply_op(&json!({ "kind": "cvl.break_out", "branch_path": BRANCH,
            "node_path": leaf_path[..40].join(".") }))
            .expect_err("cvl")
            .to_string(),
    ] {
        assert_ne!(err.len(), 0);
    }
    // A deep node in an op is refused before any write.
    let mut nested = node("leaf", "bullet", json!({}), vec![]);
    for level in 0..200 {
        nested = node(&format!("w{level}"), "section", json!({}), vec![nested]);
    }
    let mut ok = seed();
    outgoing(&mut ok);
    let before = ok.shape();
    let err = ok
        .apply_op(&json!({ "kind": "cv_profile.replace", "lang": "de", "tree": nested }))
        .expect_err("too deep");
    assert!(err.message().contains("deeper than"), "{err}");
    assert_eq!(ok.shape(), before);
    assert_eq!(outgoing(&mut ok).len(), 0);
}

#[test]
fn values_of_the_wrong_type_are_refused_never_replaced() {
    // A peer put a plain string where a node's children array belongs.
    let peer = YDoc::new();
    let root = peer.get_or_insert_map("cv_profile");
    {
        let mut txn = peer.transact_mut();
        let lang = root.insert(&mut txn, "en", MapPrelim::default());
        let tree = lang.insert(&mut txn, "tree", MapPrelim::default());
        tree.insert(&mut txn, "id", "root");
        tree.insert(&mut txn, "kind", "cv-root");
        tree.insert(&mut txn, "tier", "FIXED");
        tree.insert(&mut txn, "children", "not an array");
        tree.insert(&mut txn, "selection", "not a map");
    }
    let mut doc = Doc::new();
    doc.receive(
        &peer
            .transact()
            .encode_state_as_update_v1(&yrs::StateVector::default()),
    )
    .expect("loads");
    outgoing(&mut doc);
    let before = doc.shape();
    for op in [
        json!({ "kind": "node.create", "branch_path": BRANCH, "parent_path": "root",
                "node": node("x", "bullet", json!({}), vec![]) }),
        json!({ "kind": "selection.update", "branch_path": BRANCH, "node_path": "root", "delta": { "add": [] } }),
    ] {
        let err = doc.apply_op(&op).expect_err("refused");
        assert!(err.message().contains("is not"), "{err}");
        assert_eq!(doc.shape(), before);
        assert_eq!(outgoing(&mut doc).len(), 0);
    }
}

#[test]
fn indexes_address_the_raw_children_array() {
    // A peer left a plain value in front of the real children.
    let peer = YDoc::new();
    let root = peer.get_or_insert_map("cv_profile");
    {
        let mut txn = peer.transact_mut();
        let lang = root.insert(&mut txn, "en", MapPrelim::default());
        let tree = lang.insert(&mut txn, "tree", MapPrelim::default());
        tree.insert(&mut txn, "id", "root");
        tree.insert(&mut txn, "kind", "cv-root");
        tree.insert(&mut txn, "tier", "FIXED");
        let kids = tree.insert(&mut txn, "children", ArrayPrelim::default());
        kids.push_back(&mut txn, "stray value");
        for id in ["a", "b", "c"] {
            let child = kids.push_back(&mut txn, MapPrelim::default());
            child.insert(&mut txn, "id", id);
            child.insert(&mut txn, "kind", "section");
            child.insert(&mut txn, "tier", "HYBRID");
            child.insert(&mut txn, "children", ArrayPrelim::default());
        }
    }
    let mut doc = Doc::new();
    doc.receive(
        &peer
            .transact()
            .encode_state_as_update_v1(&yrs::StateVector::default()),
    )
    .expect("loads");
    doc.apply_op(&json!({ "kind": "node.delete", "branch_path": BRANCH, "node_path": "root.b" }))
        .expect("delete b");
    let ids = |doc: &Doc| -> Vec<Value> {
        doc.tree_json(BRANCH).expect("tree")["children"]
            .as_array()
            .expect("children")
            .iter()
            .map(|c| c["id"].clone())
            .collect()
    };
    assert_eq!(
        ids(&doc),
        vec![Value::Null, json!("a"), json!("c")],
        "only b went; the stray value stays"
    );
    doc.apply_op(&json!({ "kind": "node.move", "branch_path": BRANCH, "from_path": "root.c", "to_parent_path": "root", "to_index": 2 }))
        .expect("move c within the raw array");
    assert_eq!(ids(&doc).len(), 3);
    assert_eq!(ids(&doc)[1], json!("a"));
}

#[test]
fn text_with_embedded_content_is_refused_by_type_text_and_replaced_whole_by_updates() {
    use yrs::{Any, Text, TextPrelim};
    let peer = YDoc::new();
    let root = peer.get_or_insert_map("cv_profile");
    {
        let mut txn = peer.transact_mut();
        let lang = root.insert(&mut txn, "en", MapPrelim::default());
        let tree = lang.insert(&mut txn, "tree", MapPrelim::default());
        tree.insert(&mut txn, "id", "root");
        tree.insert(&mut txn, "kind", "cv-root");
        tree.insert(&mut txn, "tier", "FIXED");
        let kids = tree.insert(&mut txn, "children", ArrayPrelim::default());
        let bullet = kids.push_back(&mut txn, MapPrelim::default());
        bullet.insert(&mut txn, "id", "b");
        bullet.insert(&mut txn, "kind", "bullet");
        bullet.insert(&mut txn, "tier", "HYBRID");
        bullet.insert(&mut txn, "children", ArrayPrelim::default());
        let data = bullet.insert(&mut txn, "data", MapPrelim::default());
        let text = data.insert(&mut txn, "text", TextPrelim::new("ab"));
        text.insert_embed(&mut txn, 1, Any::Bool(true));
    }
    let mut doc = Doc::new();
    doc.receive(
        &peer
            .transact()
            .encode_state_as_update_v1(&yrs::StateVector::default()),
    )
    .expect("loads");
    outgoing(&mut doc);
    let before = doc.shape();
    let err = human(&mut doc, "root.b", "text", 0..0, "x").expect_err("embed");
    assert!(err.contains("embedded content"), "{err}");
    assert_eq!(doc.shape(), before);
    assert_eq!(outgoing(&mut doc).len(), 0);
    doc.apply_op(
        &json!({ "kind": "node.update", "branch_path": BRANCH, "node_path": "root.b",
        "patch": { "data": { "text": "xyz" } } }),
    )
    .expect("replaced whole");
    assert_eq!(doc.prose(BRANCH, "root.b", "text").as_deref(), Some("xyz"));
    human(&mut doc, "root.b", "text", 0..1, "w").expect("typing works once the embed is gone");
}

#[test]
fn node_refs_keep_ids_with_dots_apart() {
    // Two different nodes whose dotted paths are the same string.
    let mut doc = Doc::new();
    let tree = node(
        "root",
        "cv-root",
        json!({}),
        vec![
            node("a.b", "section", json!({}), vec![]),
            node(
                "a",
                "section",
                json!({}),
                vec![node("b", "section", json!({}), vec![])],
            ),
        ],
    );
    doc.apply_op(&json!({ "kind": "cv_profile.replace", "lang": "en", "tree": tree }))
        .expect("seed");
    let dotted_id = NodeRef::new(BRANCH, vec!["root".into(), "a.b".into()]);
    let nested = NodeRef::new(BRANCH, vec!["root".into(), "a".into(), "b".into()]);
    assert_eq!(
        dotted_id.dotted(),
        nested.dotted(),
        "the dotted forms collide"
    );
    assert_eq!(doc.view_ref(&dotted_id).expect("view")["id"], "a.b");
    assert_eq!(doc.view_ref(&nested).expect("view")["id"], "b");
    let changed = doc
        .type_text_ref(&nested, "text", 0..0, "x", ActorClass::Human)
        .expect_err("a section has no prose");
    assert!(changed.message().contains("not a prose field"), "{changed}");
    let changed = doc
        .apply_op(&json!({ "kind": "node.update", "branch_path": BRANCH, "node_path": "root.a.b", "patch": { "tier": "OPEN" } }))
        .expect("update");
    assert_eq!(changed.nodes, vec![nested]);
}

#[test]
fn a_scoped_document_owns_one_root_and_reports_only_it() {
    let mut cv = Doc::scoped("cv_profile").expect("scope");
    assert!(Doc::scoped("jobs").is_none());
    cv.apply_op(&json!({ "kind": "cv_profile.replace", "lang": "en", "tree": tree() }))
        .expect("own root");
    let err = cv
        .apply_op(&json!({ "kind": "cl_profile.replace", "lang": "en", "tree": tree() }))
        .expect_err("not ours");
    assert!(
        err.message().contains("is not part of this document"),
        "{err}"
    );
    let err = cv
        .apply_op(&json!({ "kind": "node.update", "branch_path": "cl_profile.en", "node_path": "root", "patch": {} }))
        .expect_err("not ours");
    assert!(
        err.message().contains("is not part of this document"),
        "{err}"
    );
    assert_ne!(outgoing(&mut cv).len(), 0);

    // A frame that carries both roots is reported for the owned root only.
    let mut both = Doc::new();
    both.apply_op(&json!({ "kind": "cv_profile.replace", "lang": "en", "tree": tree() }))
        .expect("cv");
    both.apply_op(&json!({ "kind": "cl_profile.replace", "lang": "en", "tree": tree() }))
        .expect("cl");
    let mut scoped = Doc::scoped("cl_profile").expect("scope");
    let changed = scoped.receive(&both.encode_state()).expect("receive");
    assert_ne!(changed.nodes.len(), 0);
    assert!(
        changed
            .nodes
            .iter()
            .all(|n| n.branch.starts_with("cl_profile.")),
        "{changed:?}"
    );
    assert!(
        scoped
            .dry_run_as(
                &json!({ "kind": "node.update", "branch_path": "cl_profile.en",
        "node_path": "root.item.b1", "patch": { "tier": "OPEN" } }),
                ActorClass::Human
            )
            .is_ok()
    );
    assert!(
        scoped
            .fork()
            .expect("fork")
            .profile_json("cv_profile")
            .is_err(),
        "a fork keeps the scope"
    );
}

#[test]
fn deep_trees_are_bounded_in_memory_and_in_reads() {
    // 20 000 levels: the change report stays bounded by the depth cap.
    let mut doc = Doc::new();
    let changed = doc.receive(&deep_peer_update(20_000)).expect("accepted");
    assert!(
        changed.nodes.len() <= 130,
        "{} nodes reported",
        changed.nodes.len()
    );
    // A node whose data is a deep plain value cannot be read as JSON, only refused.
    let peer = YDoc::new();
    let root = peer.get_or_insert_map("cv_profile");
    {
        let mut txn = peer.transact_mut();
        let lang = root.insert(&mut txn, "en", MapPrelim::default());
        let tree = lang.insert(&mut txn, "tree", MapPrelim::default());
        tree.insert(&mut txn, "id", "root");
        tree.insert(&mut txn, "kind", "cv-root");
        tree.insert(&mut txn, "tier", "FIXED");
        tree.insert(&mut txn, "children", ArrayPrelim::default());
        let mut chain = yrs::Any::Null;
        for _ in 0..5_000 {
            chain = yrs::Any::Array(vec![chain].into());
        }
        tree.insert(&mut txn, "data", chain);
    }
    let mut doc = Doc::new();
    doc.receive(
        &peer
            .transact()
            .encode_state_as_update_v1(&yrs::StateVector::default()),
    )
    .expect("loads");
    for err in [
        doc.view(BRANCH, "root").expect_err("view").to_string(),
        doc.apply_op(
            &json!({ "kind": "node.update", "branch_path": BRANCH, "node_path": "root",
            "patch": { "data": { "x": 1 } } }),
        )
        .expect_err("update")
        .to_string(),
        doc.apply_op(
            &json!({ "kind": "cvl.duplicate_as_variant", "branch_path": BRANCH,
            "node_path": "root.root" }),
        )
        .expect_err("dup")
        .to_string(),
    ] {
        assert_ne!(err.len(), 0);
    }
}

#[test]
fn writes_never_build_a_tree_the_reads_refuse() {
    let chain = |levels: usize| {
        let mut tree = node("leaf", "section", json!({}), vec![]);
        for level in (1..levels).rev() {
            tree = node(&format!("n{level}"), "section", json!({}), vec![tree]);
        }
        tree
    };
    let mut doc = Doc::new();
    // 128 levels is the cap; 129 is refused.
    doc.apply_op(&json!({ "kind": "cv_profile.replace", "lang": "en", "tree": chain(128) }))
        .expect("128 levels");
    assert!(
        doc.apply_op(&json!({ "kind": "cv_profile.replace", "lang": "de", "tree": chain(129) }))
            .is_err()
    );
    assert!(
        doc.tree_json(BRANCH).is_ok(),
        "the tree it built can be read"
    );
    let mut path = vec!["n1".to_string()];
    path.extend((2..128).map(|i| format!("n{i}")));
    path.push("leaf".into());
    let deepest = path.join(".");
    let before = doc.shape();
    outgoing(&mut doc);
    let create = |parent: &str, id: &str| {
        json!({ "kind": "node.create", "branch_path": BRANCH,
        "parent_path": parent, "node": node(id, "bullet", json!({}), vec![]) })
    };
    let err = doc
        .apply_op(&create(&deepest, "one-too-many"))
        .expect_err("depth");
    assert!(err.message().contains("deeper than"), "{err}");
    // A move that would push a subtree past the cap is refused too, and changes nothing.
    doc.apply_op(&create("n1", "wide"))
        .expect("shallow create is fine");
    outgoing(&mut doc);
    let moved = json!({ "kind": "node.move", "branch_path": BRANCH, "from_path": "n1.n2", "to_parent_path": "n1.wide" });
    assert!(doc.apply_op(&moved).is_err());
    assert_ne!(doc.shape(), before, "the shallow create did change it");
    assert_eq!(
        outgoing(&mut doc).len(),
        0,
        "the refused move emitted nothing"
    );
}

#[test]
fn inspection_names_the_authors_under_foreign_roots_and_shadows_learn_clients() {
    use ccvl_core::doc::Shadow;
    use yrs::{Map as _, MapPrelim, TextPrelim};
    let profile = YDoc::new();
    {
        let cv = profile.get_or_insert_map("cv_profile");
        let mut txn = profile.transact_mut();
        let lang = cv.insert(&mut txn, "en", MapPrelim::default());
        lang.insert(&mut txn, "tree", MapPrelim::default());
    }
    let settings = YDoc::new();
    {
        let root = settings.get_or_insert_map("settings");
        let mut txn = settings.transact_mut();
        let profile = root.insert(&mut txn, "profile", MapPrelim::default());
        profile.insert(&mut txn, "narrative", TextPrelim::new("hello"));
    }
    let state = |doc: &YDoc| {
        doc.transact()
            .encode_state_as_update_v1(&yrs::StateVector::default())
    };
    let merged = yrs::merge_updates_v1([state(&profile), state(&settings)]).expect("merge");
    let info = inspect_update(&merged).expect("inspect");
    assert!(info.roots.contains("cv_profile") && info.roots.contains("settings"));
    let settings_client = inspect_update(&state(&settings)).expect("inspect").clients;
    assert_eq!(
        info.foreign_authors, settings_client,
        "only the settings author is foreign"
    );
    assert_eq!(info.clients.len(), 2);

    let mut shadow = Shadow::new();
    shadow.receive(&state(&settings)).expect("state");
    assert_eq!(shadow.known_clients(), settings_client);
    assert!(shadow.roots().contains("settings"));
    let mut doc = Doc::new();
    doc.receive(&state(&profile)).expect("profile");
    assert!(doc.known_clients().is_disjoint(&settings_client));
    assert_eq!(doc.known_clients().len(), 1);
    assert!(doc.knows_client(*doc.known_clients().iter().next().expect("one")));
}
