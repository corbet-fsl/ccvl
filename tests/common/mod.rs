#![allow(dead_code, clippy::needless_pass_by_value)]
use ccvl::session::SubDoc;
use ccvl::session::storage::{Stored, Writer};
use cvws::Store;
use serde_json::{Value, json};

pub const CV: &str = "cv_profile.en";
pub const CL: &str = "cl_profile.en";

fn node(id: &str, kind: &str, data: Value, children: Vec<Value>) -> Value {
    let active: Vec<Value> = children.iter().map(|c| c["id"].clone()).collect();
    json!({ "id": id, "kind": kind, "tier": "HYBRID", "data": data,
            "selection": { "active": active }, "children": children })
}

pub fn cv_tree() -> Value {
    node(
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
                json!({ "text": "Built things" }),
                vec![],
            )],
        )],
    )
}

pub fn cl_tree() -> Value {
    node(
        "cl-root",
        "cl-root",
        json!({}),
        vec![node(
            "body",
            "paragraph",
            json!({ "archetype": "text", "text": "Dear team" }),
            vec![],
        )],
    )
}

pub fn writer(who: &str) -> Writer {
    Writer {
        actor_id: who.into(),
        device: format!("{who}-device"),
    }
}

pub fn seed<S: Store>(s: &mut Stored<S>) {
    for (kind, tree) in [
        ("cv_profile.replace", cv_tree()),
        ("cl_profile.replace", cl_tree()),
    ] {
        s.edit(&json!({ "kind": kind, "lang": "en", "tree": tree }))
            .expect("seed");
    }
}

pub fn set_bullet(text: &str) -> Value {
    json!({ "kind": "node.update", "branch_path": CV, "node_path": "root.item.b1",
            "patch": { "data": { "text": text } } })
}

pub fn bullet<S: Store>(s: &Stored<S>) -> String {
    s.workspace()
        .view(SubDoc::CvProfile, CV, "root.item.b1")
        .expect("view")["data"]["text"]
        .as_str()
        .expect("text")
        .to_owned()
}

#[cfg(feature = "remote")]
pub fn seed_remote(s: &mut ccvl::session::remote::RemoteSession) {
    for (kind, tree) in [
        ("cv_profile.replace", cv_tree()),
        ("cl_profile.replace", cl_tree()),
    ] {
        s.edit(&json!({ "kind": kind, "lang": "en", "tree": tree }))
            .expect("seed");
    }
}
