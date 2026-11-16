//! The op catalogue (version 10) applied to a Yrs document with the semantics of
//! `applyOp` in `@cv/mutations`: same checks, same order, same error messages.
//!
//! Every op validates before it writes, so a refused op leaves the document
//! untouched and produces no update bytes (`@cv/mutations` can leave a partial
//! write behind for a few refusals; the vectors note those cases).
use base64::Engine as _;
use serde_json::{Map, Value, json};
use yrs::updates::decoder::Decode;
use yrs::{
    Any, Array, Doc, Map as _, MapRef, Out, ReadTxn, StateVector, Transact, TransactionMut, Update,
};

use super::convert::{out_to_value, value_to_any};
use super::cvl;
use super::tree::{
    apply_data_patch, child_id_set, ensure_children_array, ensure_fits, ensure_lang_map,
    ensure_map, ensure_selection_map, fill_node, find_child_index, get_array, get_map, node_id,
    node_json, parse_node_path, prose_keys, resolve_node_path, resolve_segments,
    scrub_from_selection, subtree_depth, value_depth,
};
use super::{ActorClass, OP_CATALOG_VERSION, OpError, new_doc};

const OP_KINDS: [&str; 21] = [
    "job.create",
    "job.update",
    "job.delete",
    "node.create",
    "node.update",
    "node.delete",
    "node.move",
    "selection.update",
    "cvl.duplicate_as_variant",
    "cvl.break_out",
    "cvl.promote_sub",
    "cvl.demote_bullet",
    "tailored.snapshot",
    "tailored.promote",
    "settings.update",
    "layout.update",
    "order_state.update",
    "cv_profile.replace",
    "cl_profile.replace",
    "yjs.update",
    "translate",
];

/// The root maps of one document, created once so ops never create a root
/// while a transaction is open. A scoped document owns one profile root only
/// (a sub-document of the workspace); the other root is not part of it.
#[derive(Clone)]
pub(crate) struct Roots {
    pub cv: MapRef,
    pub cl: MapRef,
    pub scope: Option<&'static str>,
}

impl Roots {
    pub(crate) fn new(doc: &Doc, scope: Option<&'static str>) -> Self {
        Self {
            cv: doc.get_or_insert_map("cv_profile"),
            cl: doc.get_or_insert_map("cl_profile"),
            scope,
        }
    }

    /// True when `name` is a profile root this document owns.
    pub(crate) fn owns(&self, name: &str) -> bool {
        matches!(name, "cv_profile" | "cl_profile") && self.scope.is_none_or(|s| s == name)
    }

    pub(crate) fn profile(&self, name: &str) -> Option<&MapRef> {
        if !self.owns(name) {
            return None;
        }
        match name {
            "cv_profile" => Some(&self.cv),
            _ => Some(&self.cl),
        }
    }
}

pub(crate) struct Ctx<'a> {
    pub roots: &'a Roots,
    pub actor: ActorClass,
    pub ids: &'a mut dyn FnMut() -> String,
}

pub(crate) fn fail<T>(message: impl Into<String>) -> Result<T, OpError> {
    Err(OpError::new(message))
}

pub(crate) fn req_str<'a>(op: &'a Value, kind: &str, key: &str) -> Result<&'a str, OpError> {
    op.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| OpError::new(format!("{kind}: '{key}' must be a string")))
}

fn op_kind(op: &Value) -> String {
    match op.get("kind") {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => "undefined".to_string(),
    }
}

/// JS `String(number)` for the numbers that appear in error messages.
fn js_number(n: &Value) -> String {
    match n.as_f64() {
        Some(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{f:.0}"),
        Some(f) => f.to_string(),
        None => n.to_string(),
    }
}

fn js_typeof(v: &Value) -> &'static str {
    match v {
        Value::String(_) => "string",
        Value::Number(_) => "number",
        Value::Bool(_) => "boolean",
        _ => "object",
    }
}

pub(crate) fn resolve_branch_root<T: ReadTxn>(
    roots: &Roots,
    txn: &T,
    branch_path: &str,
) -> Result<MapRef, OpError> {
    let parts: Vec<&str> = branch_path.split('.').collect();
    if parts.len() < 2 {
        return fail(format!(
            "resolveBranchRoot: branch_path must be '<profile>.<lang>', got '{branch_path}'"
        ));
    }
    let (profile, lang) = (parts[0], parts[1]);
    let Some(root) = roots.profile(profile) else {
        if matches!(profile, "cv_profile" | "cl_profile") {
            return fail(format!(
                "resolveBranchRoot: '{profile}' is not part of this document"
            ));
        }
        return fail(format!(
            "resolveBranchRoot: branch_path must start with cv_profile or cl_profile, got '{profile}'"
        ));
    };
    get_map(txn, root, lang).ok_or_else(|| {
        OpError::new(format!(
            "resolveBranchRoot: lang '{lang}' not found in {profile}; seed via {profile}.replace first"
        ))
    })
}

/// The error for an actor that may not edit a `FIXED` node.
pub(crate) fn tier_protected<T: ReadTxn>(
    txn: &T,
    node: &MapRef,
    actor: ActorClass,
    action: &str,
) -> Result<(), OpError> {
    if actor != ActorClass::Human
        && super::tree::get_string(txn, node, "tier").as_deref() == Some("FIXED")
    {
        return fail(format!(
            "tier_protected: {} {action} cannot mutate FIXED node '{}'",
            actor.as_str(),
            node_id(txn, node).unwrap_or_default()
        ));
    }
    Ok(())
}

/// Agents and system actors may not delete or move a FIXED node, nor a node whose
/// subtree holds one (taking the parent removes it just the same). Humans may.
fn ensure_unprotected<T: ReadTxn>(
    txn: &T,
    node: &MapRef,
    actor: ActorClass,
    action: &str,
) -> Result<(), OpError> {
    if actor == ActorClass::Human {
        return Ok(());
    }
    let mut stack = vec![node.clone()];
    while let Some(current) = stack.pop() {
        tier_protected(txn, &current, actor, action)?;
        stack.extend(super::tree::child_maps(txn, &current));
    }
    Ok(())
}

/// Apply one op inside an open transaction.
pub(crate) fn apply_op(
    ctx: &mut Ctx<'_>,
    txn: &mut TransactionMut,
    op: &Value,
) -> Result<(), OpError> {
    let kind = op_kind(op);
    if !OP_KINDS.contains(&kind.as_str()) && kind != "batch" {
        return fail(format!(
            "unknown_op_kind: '{kind}' is not in the current op catalog (OP_CATALOG_VERSION={OP_CATALOG_VERSION}). \
             The client may be stale \u{2014} please refresh the page."
        ));
    }
    match kind.as_str() {
        "cv_profile.replace" | "cl_profile.replace" => profile_replace(ctx, txn, op, &kind),
        "yjs.update" => yjs_update(ctx, txn, op),
        "node.create" => node_create(ctx, txn, op),
        "node.update" => node_update(ctx, txn, op),
        "node.delete" => node_delete(ctx, txn, op),
        "node.move" => node_move(ctx, txn, op),
        "batch" => batch(ctx, txn, op),
        "selection.update" => selection_update(ctx, txn, op),
        "cvl.duplicate_as_variant" | "cvl.break_out" | "cvl.promote_sub" | "cvl.demote_bullet" => {
            cvl_action(ctx, txn, op, &kind)
        }
        "translate" => translate(ctx, txn, op),
        "tailored.snapshot" | "tailored.promote" => fail(format!(
            "unsupported_op_kind: '{kind}' belongs to the jobs sub-document; the profile engine does not edit it"
        )),
        other => fail(format!(
            "unsupported_op_kind: '{other}' targets a sub-document the profile engine does not edit"
        )),
    }
}

fn profile_replace(
    ctx: &mut Ctx<'_>,
    txn: &mut TransactionMut,
    op: &Value,
    kind: &str,
) -> Result<(), OpError> {
    let lang = req_str(op, kind, "lang")?;
    let Some(tree) = op.get("tree").filter(|t| t.is_object()) else {
        return fail(format!("{kind}: 'tree' must be an object"));
    };
    ensure_fits(0, value_depth(tree)?)?;
    let format = match op.get("format") {
        None => None,
        Some(Value::Object(f)) => Some(f),
        Some(_) => return fail(format!("{kind}: 'format' must be an object")),
    };
    let profile = if kind == "cv_profile.replace" {
        "cv_profile"
    } else {
        "cl_profile"
    };
    let Some(root) = ctx.roots.profile(profile).cloned() else {
        return fail(format!("{kind}: '{profile}' is not part of this document"));
    };
    let existing = ensure_lang_map(&*txn, &root, lang)?;
    if let Some(lang_map) = &existing
        && let Some(other) = lang_map.get(&*txn, "format")
        && !matches!(other, Out::YMap(_))
    {
        return fail(format!("'format' of {lang} is not a map"));
    }
    let lang_map = match existing {
        Some(m) => m,
        None => root.insert(txn, lang, yrs::MapPrelim::default()),
    };
    let tree_map = lang_map.insert(txn, "tree", yrs::MapPrelim::default());
    fill_node(txn, &tree_map, tree);
    if let Some(format) = format {
        let fm = ensure_map(txn, &lang_map, "format")?;
        for (k, v) in format {
            fm.insert(txn, k.as_str(), value_to_any(v));
        }
    }
    Ok(())
}

fn yjs_update(ctx: &mut Ctx<'_>, txn: &mut TransactionMut, op: &Value) -> Result<(), OpError> {
    if ctx.actor != ActorClass::Human {
        return fail(format!(
            "yjs_update_forbidden: {} cannot apply raw profile updates",
            ctx.actor.as_str()
        ));
    }
    let sub_doc = op
        .get("sub_doc")
        .and_then(Value::as_str)
        .unwrap_or("undefined");
    if sub_doc != "cv-profile" && sub_doc != "cl-profile" {
        return fail(format!("yjs.update: unsupported sub_doc '{sub_doc}'"));
    }
    let encoded = op.get("update").and_then(Value::as_str).unwrap_or("");
    if encoded.is_empty() {
        return fail("yjs.update: update must be a non-empty base64 string");
    }
    let engine = base64::engine::general_purpose::STANDARD;
    let bytes = engine
        .decode(encoded)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(encoded))
        .map_err(|_| OpError::new("yjs.update: update is not valid base64"))?;
    if bytes.is_empty() {
        return fail("yjs.update: decoded update is empty");
    }
    let update = Update::decode_v1(&bytes)
        .map_err(|e| OpError::new(format!("yjs.update: undecodable update: {e}")))?;
    txn.apply_update(update)
        .map_err(|e| OpError::new(format!("yjs.update: update refused: {e}")))
}

fn tree_of<T: ReadTxn>(
    roots: &Roots,
    txn: &T,
    kind: &str,
    branch_path: &str,
    suffix: &str,
) -> Result<MapRef, OpError> {
    let branch = resolve_branch_root(roots, txn, branch_path)?;
    get_map(txn, &branch, "tree")
        .ok_or_else(|| OpError::new(format!("{kind}: tree not found at '{branch_path}'{suffix}")))
}

fn node_create(ctx: &mut Ctx<'_>, txn: &mut TransactionMut, op: &Value) -> Result<(), OpError> {
    let kind = "node.create";
    let branch_path = req_str(op, kind, "branch_path")?;
    let parent_path = req_str(op, kind, "parent_path")?;
    let Some(node) = op.get("node").and_then(Value::as_object) else {
        return fail("node.create: 'node' must be an object");
    };
    let tree = tree_of(
        ctx.roots,
        &*txn,
        kind,
        branch_path,
        "; seed via profile.replace first",
    )?;
    let Some((parent, parent_depth)) =
        resolve_segments(&*txn, &tree, &parse_node_path(parent_path))
    else {
        return fail(format!(
            "node.create: parent '{parent_path}' not found in '{branch_path}'"
        ));
    };
    let node_id_value = match node.get("id") {
        None | Some(Value::Null) => (ctx.ids)(),
        Some(Value::String(id)) => id.clone(),
        Some(_) => return fail("node.create: node id must be a string"),
    };
    ensure_fits(parent_depth, value_depth(&Value::Object(node.clone()))?)?;
    let existing_children = match parent.get(&*txn, "children") {
        None => None,
        Some(Out::YArray(a)) => Some(a),
        Some(_) => {
            return fail(format!(
                "'children' of node '{}' is not an array",
                node_id(&*txn, &parent).unwrap_or_default()
            ));
        }
    };
    if let Some(children) = &existing_children
        && child_id_set(&*txn, children).contains(&node_id_value)
    {
        return fail(format!(
            "node.create: id '{node_id_value}' already exists in parent '{parent_path}'"
        ));
    }
    let mut full = Map::new();
    full.insert("id".into(), Value::String(node_id_value));
    full.insert(
        "kind".into(),
        node.get("kind").cloned().unwrap_or(Value::Null),
    );
    full.insert(
        "tier".into(),
        node.get("tier").cloned().unwrap_or(Value::Null),
    );
    full.insert(
        "children".into(),
        node.get("children").cloned().unwrap_or_else(|| json!([])),
    );
    full.insert(
        "selection".into(),
        node.get("selection")
            .cloned()
            .unwrap_or_else(|| json!({ "active": [] })),
    );
    for key in ["data", "prompt", "applicable_langs"] {
        if let Some(v) = node.get(key) {
            full.insert(key.into(), v.clone());
        }
    }
    let children = ensure_children_array(txn, &parent)?;
    let map = children.push_back(txn, yrs::MapPrelim::default());
    fill_node(txn, &map, &Value::Object(full));
    Ok(())
}

fn node_update(ctx: &mut Ctx<'_>, txn: &mut TransactionMut, op: &Value) -> Result<(), OpError> {
    let kind = "node.update";
    let branch_path = req_str(op, kind, "branch_path")?;
    let node_path = req_str(op, kind, "node_path")?;
    let tree = tree_of(ctx.roots, &*txn, kind, branch_path, "")?;
    let Some(node) = resolve_node_path(&*txn, &tree, node_path) else {
        return fail(format!(
            "node.update: node '{node_path}' not found in '{branch_path}'"
        ));
    };
    tier_protected(&*txn, &node, ctx.actor, "node.update")?;
    let Some(patch) = op.get("patch").and_then(Value::as_object) else {
        return fail("node.update: 'patch' must be an object");
    };
    for key in patch.keys() {
        if matches!(key.as_str(), "id" | "children" | "selection") {
            return fail(format!(
                "node.update: patching '{key}' is forbidden; use node.move or selection.update"
            ));
        }
    }
    if let Some(data) = patch.get("data")
        && !data.is_object()
    {
        return fail(format!(
            "node.update: data patch must be an object, got {}",
            js_typeof(data)
        ));
    }
    let node_kind = super::tree::get_string(&*txn, &node, "kind").unwrap_or_default();
    let existing: Option<Map<String, Value>> = node
        .get(&*txn, "data")
        .map(|d| out_to_value(&*txn, &d))
        .transpose()?
        .and_then(|d| d.as_object().cloned());
    let next_kind = patch
        .get("kind")
        .and_then(Value::as_str)
        .map_or(node_kind, str::to_string);
    for (key, value) in patch {
        if key == "data" {
            let data_patch = value.as_object().cloned().unwrap_or_default();
            let mut merged = existing.clone().unwrap_or_default();
            for (k, v) in &data_patch {
                merged.insert(k.clone(), v.clone());
            }
            let keys = prose_keys(&next_kind, Some(&Value::Object(merged)));
            apply_data_patch(txn, &node, &keys, &data_patch)?;
        } else {
            node.insert(txn, key.as_str(), value_to_any(value));
        }
    }
    Ok(())
}

fn node_delete(ctx: &mut Ctx<'_>, txn: &mut TransactionMut, op: &Value) -> Result<(), OpError> {
    let kind = "node.delete";
    let branch_path = req_str(op, kind, "branch_path")?;
    let node_path = req_str(op, kind, "node_path")?;
    let tree = tree_of(ctx.roots, &*txn, kind, branch_path, "")?;
    let parts = parse_node_path(node_path);
    if parts.len() < 2 {
        return fail(format!(
            "node.delete: node_path must have >= 2 segments; got '{node_path}'"
        ));
    }
    let parent_path = parts[..parts.len() - 1].join(".");
    let target = parts[parts.len() - 1];
    let Some(parent) = resolve_node_path(&*txn, &tree, &parent_path) else {
        return fail(format!(
            "node.delete: parent '{parent_path}' not found in '{branch_path}'"
        ));
    };
    let Some(children) = get_array(&*txn, &parent, "children") else {
        return fail(format!(
            "node.delete: parent '{parent_path}' has no children array"
        ));
    };
    let Some(index) = find_child_index(&*txn, &children, target) else {
        return fail(format!(
            "node.delete: node '{target}' not found in parent '{parent_path}'"
        ));
    };
    if let Some(Out::YMap(doomed)) = children.get(&*txn, index) {
        ensure_unprotected(&*txn, &doomed, ctx.actor, "node.delete")?;
    }
    children.remove(txn, index);
    scrub_from_selection(txn, &parent, target);
    Ok(())
}

fn node_move(ctx: &mut Ctx<'_>, txn: &mut TransactionMut, op: &Value) -> Result<(), OpError> {
    let kind = "node.move";
    let branch_path = req_str(op, kind, "branch_path")?;
    let from_path = req_str(op, kind, "from_path")?;
    let to_parent_path = req_str(op, kind, "to_parent_path")?;
    let tree = tree_of(ctx.roots, &*txn, kind, branch_path, "")?;
    let parts = parse_node_path(from_path);
    if parts.len() < 2 {
        return fail("node.move: from_path must include parent + node id");
    }
    let from_parent_path = parts[..parts.len() - 1].join(".");
    let id = parts[parts.len() - 1];
    let Some(from_parent) = resolve_node_path(&*txn, &tree, &from_parent_path) else {
        return fail(format!(
            "node.move: from_parent '{from_parent_path}' not found"
        ));
    };
    let Some(from_children) = get_array(&*txn, &from_parent, "children") else {
        return fail(format!(
            "node.move: from_parent '{from_parent_path}' has no children"
        ));
    };
    let Some(index) = find_child_index(&*txn, &from_children, id) else {
        return fail(format!(
            "node.move: node '{id}' not found in '{from_parent_path}'"
        ));
    };
    let Some(Out::YMap(moved)) = from_children.get(&*txn, index) else {
        return fail(format!(
            "node.move: node '{id}' not found in '{from_parent_path}'"
        ));
    };
    ensure_unprotected(&*txn, &moved, ctx.actor, "node.move")?;
    let node_value = node_json(&*txn, &moved)?;
    let Some((to_parent, to_depth)) =
        resolve_segments(&*txn, &tree, &parse_node_path(to_parent_path))
    else {
        return fail(format!("node.move: to_parent '{to_parent_path}' not found"));
    };
    ensure_fits(to_depth, subtree_depth(&*txn, &moved)?)?;
    if is_inside(&*txn, &moved, &to_parent) {
        return fail(format!(
            "node.move: destination '{to_parent_path}' is inside the moved node '{id}'"
        ));
    }
    let same_parent = from_parent == to_parent;
    let to_existing = match to_parent.get(&*txn, "children") {
        None => None,
        Some(Out::YArray(a)) => Some(a),
        Some(_) => {
            return fail(format!(
                "'children' of node '{}' is not an array",
                node_id(&*txn, &to_parent).unwrap_or_default()
            ));
        }
    };
    if !same_parent
        && let Some(children) = &to_existing
        && child_id_set(&*txn, children).contains(id)
    {
        return fail(format!(
            "node.move: id '{id}' already exists in destination '{to_parent_path}'"
        ));
    }
    from_children.remove(txn, index);
    scrub_from_selection(txn, &from_parent, id);
    let to_children = ensure_children_array(txn, &to_parent)?;
    let target = match op.get("to_index") {
        None => None,
        Some(Value::Number(n)) => {
            let len = f64::from(to_children.len(&*txn));
            let clamped = n.as_f64().unwrap_or(0.0).clamp(0.0, len);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            Some(clamped as u32)
        }
        Some(_) => Some(0),
    };
    let new_map = match target {
        None => to_children.push_back(txn, yrs::MapPrelim::default()),
        Some(at) => to_children.insert(txn, at, yrs::MapPrelim::default()),
    };
    fill_node(txn, &new_map, &node_value);
    Ok(())
}

/// True when `candidate` is `ancestor` itself or lies inside its subtree (iterative).
fn is_inside<T: ReadTxn>(txn: &T, ancestor: &MapRef, candidate: &MapRef) -> bool {
    let mut stack = vec![ancestor.clone()];
    while let Some(current) = stack.pop() {
        if current == *candidate {
            return true;
        }
        stack.extend(super::tree::child_maps(txn, &current));
    }
    false
}

fn batch(ctx: &mut Ctx<'_>, txn: &mut TransactionMut, op: &Value) -> Result<(), OpError> {
    let Some(ops) = op.get("ops").and_then(Value::as_array) else {
        return fail("batch: 'ops' must be an array");
    };
    if ops.is_empty() {
        return Ok(());
    }
    let mut declared: Option<&str> = None;
    for sub in ops {
        let Some(sub_doc) = sub.get("subDoc").and_then(Value::as_str) else {
            continue;
        };
        match declared {
            None => declared = Some(sub_doc),
            Some(first) if first != sub_doc => {
                return fail(format!(
                    "batch: cross-sub-doc batch cannot be applied through applyOp(doc, op); \
                     sub-ops span sub-docs '{first}' and '{sub_doc}'. \
                     Use planBatch + per-sub-doc dispatch (server: commitBatchOp; client: applyBatch)."
                ));
            }
            Some(_) => {}
        }
    }
    let mut sub_ops = Vec::with_capacity(ops.len());
    for sub in ops {
        let Some(inner) = sub.get("op") else {
            return fail("batch: every sub-op needs an 'op'");
        };
        sub_ops.push(inner);
    }
    // A failing sub-op must not leave a half-applied batch: dry-run on a copy
    // of the document first, then replay on the real one.
    let probe = new_doc(None);
    let probe_roots = Roots::new(&probe, ctx.roots.scope);
    {
        let snapshot = txn.encode_state_as_update_v1(&StateVector::default());
        let update = Update::decode_v1(&snapshot)
            .map_err(|e| OpError::new(format!("batch: probe failed: {e}")))?;
        let mut probe_txn = probe.transact_mut();
        probe_txn
            .apply_update(update)
            .map_err(|e| OpError::new(format!("batch: probe failed: {e}")))?;
        let mut probe_ids = super::default_id_source();
        let mut probe_ctx = Ctx {
            roots: &probe_roots,
            actor: ctx.actor,
            ids: &mut probe_ids,
        };
        for inner in &sub_ops {
            apply_op(&mut probe_ctx, &mut probe_txn, inner)?;
        }
    }
    for inner in sub_ops {
        apply_op(ctx, txn, inner)?;
    }
    Ok(())
}

/// A delta list that must hold strings only.
fn delta_strings(delta: &Map<String, Value>, key: &str) -> Result<Option<Vec<String>>, OpError> {
    let Some(value) = delta.get(key) else {
        return Ok(None);
    };
    let bad = || {
        OpError::new(format!(
            "selection.update: delta.{key} must be an array of strings"
        ))
    };
    let items = value.as_array().ok_or_else(bad)?;
    items
        .iter()
        .map(|v| v.as_str().map(str::to_string).ok_or_else(bad))
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn selection_update(
    ctx: &mut Ctx<'_>,
    txn: &mut TransactionMut,
    op: &Value,
) -> Result<(), OpError> {
    let kind = "selection.update";
    let branch_path = req_str(op, kind, "branch_path")?;
    let node_path = req_str(op, kind, "node_path")?;
    let tree = tree_of(ctx.roots, &*txn, kind, branch_path, "")?;
    let Some(node) = resolve_node_path(&*txn, &tree, node_path) else {
        return fail(format!(
            "selection.update: node '{node_path}' not found in '{branch_path}'"
        ));
    };
    let Some(delta) = op.get("delta").and_then(Value::as_object) else {
        return fail("selection.update: 'delta' must be an object");
    };
    let existing_sel = match node.get(&*txn, "selection") {
        None => None,
        Some(Out::YMap(m)) => Some(m),
        Some(_) => {
            return fail(format!(
                "'selection' of node '{}' is not a map",
                node_id(&*txn, &node).unwrap_or_default()
            ));
        }
    };
    let mut current: Vec<String> = match existing_sel
        .as_ref()
        .and_then(|sel| sel.get(&*txn, "active"))
    {
        Some(Out::Any(Any::Array(items))) => items
            .iter()
            .map(|a| match a {
                Any::String(s) => Ok(s.to_string()),
                _ => fail("selection.update: active[] contains non-string entries"),
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => Vec::new(),
    };
    let set_active = delta_strings(delta, "setActive")?;
    let remove = delta_strings(delta, "remove")?;
    let add = delta_strings(delta, "add")?;
    if let Some(set) = set_active {
        current = set;
    } else {
        if let Some(remove) = remove {
            current.retain(|id| !remove.contains(id));
        }
        if let Some(add) = add {
            for id in add {
                if !current.contains(&id) {
                    current.push(id);
                }
            }
        }
    }
    let allowed = get_array(&*txn, &node, "children")
        .map(|c| child_id_set(&*txn, &c))
        .unwrap_or_default();
    let invalid: Vec<&str> = current
        .iter()
        .filter(|id| !allowed.contains(*id))
        .map(String::as_str)
        .collect();
    if !invalid.is_empty() {
        return fail(format!(
            "selection.update: active[] contains ids not in children: {}",
            invalid.join(", ")
        ));
    }
    let existing = |key: &str| -> Option<f64> {
        match existing_sel.as_ref()?.get(&*txn, key) {
            Some(Out::Any(Any::Number(n))) => Some(match n {
                yrs::any::Number::Int(i) => {
                    #[allow(clippy::cast_precision_loss)]
                    let f = i as f64;
                    f
                }
                yrs::any::Number::Float(f) => f,
            }),
            _ => None,
        }
    };
    let next = |key: &str| -> Option<f64> {
        match delta.get(key) {
            None => existing(key),
            Some(Value::Null) => None,
            Some(v) => v.as_f64(),
        }
    };
    if let (Some(min), Some(max)) = (next("minCount"), next("maxCount"))
        && min > max
    {
        return fail(format!(
            "selection.update: minCount ({}) must be <= maxCount ({})",
            js_number(&json!(min)),
            js_number(&json!(max))
        ));
    }
    let sel = ensure_selection_map(txn, &node)?;
    sel.insert(
        txn,
        "active",
        Any::Array(
            current
                .iter()
                .map(|s| Any::String(s.as_str().into()))
                .collect::<Vec<_>>()
                .into(),
        ),
    );
    for key in ["minCount", "maxCount"] {
        match delta.get(key) {
            None => {}
            Some(Value::Null) => {
                sel.remove(txn, key);
            }
            Some(v) => {
                sel.insert(txn, key, value_to_any(v));
            }
        }
    }
    Ok(())
}

fn cvl_action(
    ctx: &mut Ctx<'_>,
    txn: &mut TransactionMut,
    op: &Value,
    kind: &str,
) -> Result<(), OpError> {
    let branch_path = req_str(op, kind, "branch_path")?;
    let tree = tree_of(ctx.roots, &*txn, kind, branch_path, "")?;
    let expansion = cvl::expand(ctx, &*txn, &tree, op)?;
    if expansion.is_empty() {
        return Ok(());
    }
    let ops: Vec<Value> = expansion.into_iter().map(|o| json!({ "op": o })).collect();
    batch(ctx, txn, &json!({ "kind": "batch", "ops": ops }))
}

/// The checks the server runs on a translate op (`validateTranslateOp`), beyond what `applyOp` checks.
fn translate_server_checks(op: &Value, src: &[&str], tgt: &[&str]) -> Result<(), OpError> {
    if src[0] != tgt[0] {
        return fail(format!(
            "translate: source and target must be in the same profile (got '{}' and '{}')",
            src[0], tgt[0]
        ));
    }
    let lang = |key: &str| op.get(key).and_then(Value::as_str).unwrap_or("");
    if lang("source_lang").len() < 2 || lang("target_lang").len() < 2 {
        return fail("translate: source_lang and target_lang must be at least 2 characters");
    }
    if lang("source_lang") == lang("target_lang") {
        return fail("translate: source_lang and target_lang must differ");
    }
    if src[1] == tgt[1] {
        return fail(
            "translate: source_path and target_path must reference different language trees",
        );
    }
    Ok(())
}

fn translated_node_is_valid(node: &Value) -> bool {
    let text = |k: &str| {
        node.get(k)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
    };
    let tier = node.get("tier").and_then(Value::as_str);
    let active_ok = node
        .get("selection")
        .and_then(|s| s.get("active"))
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().all(Value::is_string));
    text("id")
        && text("kind")
        && matches!(tier, Some("FIXED" | "HYBRID" | "OPEN"))
        && node.get("children").is_some_and(Value::is_array)
        && active_ok
}

fn translate(ctx: &mut Ctx<'_>, txn: &mut TransactionMut, op: &Value) -> Result<(), OpError> {
    let kind = "translate";
    let source_path = req_str(op, kind, "source_path")?;
    let target_path = req_str(op, kind, "target_path")?;
    let src: Vec<&str> = source_path.split('.').collect();
    if src.len() < 4 {
        return fail(format!(
            "translate: source_path must be '<profile>.<lang>.tree.<nodeId>[...]', got '{source_path}'"
        ));
    }
    let (src_profile, src_lang, src_tree_key) = (src[0], src[1], src[2]);
    if src_tree_key != "tree" {
        return fail(format!(
            "translate: source_path segment 2 must be 'tree', got '{src_tree_key}'"
        ));
    }
    let src_lang_map = ctx
        .roots
        .profile(src_profile)
        .and_then(|root| get_map(&*txn, root, src_lang));
    let Some(src_lang_map) = src_lang_map else {
        return fail(format!(
            "translate: source lang '{src_lang}' not found in {src_profile}"
        ));
    };
    let Some(src_tree) = get_map(&*txn, &src_lang_map, "tree") else {
        return fail(format!(
            "translate: tree not found at '{}'",
            src[..3].join(".")
        ));
    };
    let src_node_path = src[3..].join(".");
    if resolve_node_path(&*txn, &src_tree, &src_node_path).is_none() {
        return fail(format!(
            "translate: source node '{src_node_path}' not found in {src_profile}.{src_lang}.tree"
        ));
    }

    let tgt: Vec<&str> = target_path.split('.').collect();
    if tgt.len() < 4 {
        return fail(format!(
            "translate: target_path must be '<profile>.<lang>.tree.<nodeId>[...]', got '{target_path}'"
        ));
    }
    let (tgt_profile, tgt_lang, tgt_tree_key) = (tgt[0], tgt[1], tgt[2]);
    if tgt_tree_key != "tree" {
        return fail(format!(
            "translate: target_path segment 2 must be 'tree', got '{tgt_tree_key}'"
        ));
    }
    let Some(tgt_root) = ctx.roots.profile(tgt_profile).cloned() else {
        return fail(format!(
            "translate: target profile '{tgt_profile}' must be cv_profile or cl_profile"
        ));
    };
    translate_server_checks(op, &src, &tgt)?;
    // A target language that has no tree yet has nothing to translate into:
    // refuse without creating an empty language map.
    let tgt_tree = ensure_lang_map(&*txn, &tgt_root, tgt_lang)?
        .and_then(|lang_map| get_map(&*txn, &lang_map, "tree"));
    let Some(tgt_tree) = tgt_tree else {
        return fail(format!(
            "translate: target tree not found at '{}'; seed via {tgt_profile}.replace first",
            tgt[..3].join(".")
        ));
    };
    let tgt_node_path = tgt[3..].join(".");
    let tgt_parts = parse_node_path(&tgt_node_path);
    if tgt_parts.len() < 2 {
        return fail(format!(
            "translate: target_path must include parent + node id segments, got '{tgt_node_path}'"
        ));
    }
    let tgt_parent_path = tgt_parts[..tgt_parts.len() - 1].join(".");
    let tgt_id = tgt_parts[tgt_parts.len() - 1];
    let Some((tgt_parent, tgt_depth)) =
        resolve_segments(&*txn, &tgt_tree, &parse_node_path(&tgt_parent_path))
    else {
        return fail(format!(
            "translate: target parent '{tgt_parent_path}' not found in {tgt_profile}.{tgt_lang}.tree"
        ));
    };
    let Some(translated) = op.get("translated_node").filter(|n| n.is_object()) else {
        return fail("translate: 'translated_node' must be an object");
    };
    if !translated_node_is_valid(translated) {
        return fail(
            "translate: translated_node needs a non-empty id and kind, tier FIXED, HYBRID or OPEN, \
             a children array and selection.active as strings",
        );
    }
    ensure_fits(tgt_depth, value_depth(translated)?)?;
    if !matches!(
        tgt_parent.get(&*txn, "children"),
        None | Some(Out::YArray(_))
    ) {
        return fail(format!(
            "'children' of node '{}' is not an array",
            node_id(&*txn, &tgt_parent).unwrap_or_default()
        ));
    }
    let children = ensure_children_array(txn, &tgt_parent)?;
    let map = if let Some(index) = find_child_index(&*txn, &children, tgt_id) {
        children.remove(txn, index);
        children.insert(txn, index, yrs::MapPrelim::default())
    } else {
        children.push_back(txn, yrs::MapPrelim::default())
    };
    fill_node(txn, &map, translated);
    Ok(())
}
