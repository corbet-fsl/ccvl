//! Reads and writes of the `CareerVector` node layout on a Yrs document.
//!
//! Layout of one node (a `Y.Map`): `id`, `kind`, `tier`, optional `data`,
//! `prompt`, `applicable_langs`, `selection` (a `Y.Map` whose `active` is a plain
//! array), and `children` (a `Y.Array` of node maps). `data` is a plain object for
//! kinds without prose and a `Y.Map` holding `Y.Text` for the prose keys.
use std::collections::HashSet;

use serde_json::{Map, Value};
use yrs::{
    Any, Array, ArrayPrelim, ArrayRef, Map as _, MapPrelim, MapRef, Out, ReadTxn, Text, TextPrelim,
    TransactionMut,
};

use super::OpError;
use super::convert::{any_to_value_checked, out_to_value, value_to_any};
use super::text::compute_middle_edit;

pub(crate) const VARIANT_POOL: &str = "variant-pool";
pub(crate) const BULLET: &str = "bullet";
pub(crate) const SUB: &str = "sub";
pub(crate) const SUBHEADING: &str = "subheading";
pub(crate) const LIST_ITEM: &str = "list-item";

/// The `data` keys stored as `Y.Text` for a node (`proseDataKeysForNode`).
pub(crate) fn prose_keys(kind: &str, data: Option<&Value>) -> Vec<&'static str> {
    if kind == VARIANT_POOL {
        return Vec::new();
    }
    match kind {
        BULLET | SUB | SUBHEADING => return vec!["text"],
        LIST_ITEM => return vec!["heading"],
        _ => {}
    }
    let archetype_text = data
        .and_then(|d| d.get("archetype"))
        .and_then(Value::as_str)
        == Some("text");
    if archetype_text {
        vec!["text"]
    } else {
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------

pub(crate) fn get_map<T: ReadTxn>(txn: &T, m: &MapRef, key: &str) -> Option<MapRef> {
    match m.get(txn, key) {
        Some(Out::YMap(inner)) => Some(inner),
        _ => None,
    }
}

pub(crate) fn get_array<T: ReadTxn>(txn: &T, m: &MapRef, key: &str) -> Option<ArrayRef> {
    match m.get(txn, key) {
        Some(Out::YArray(inner)) => Some(inner),
        _ => None,
    }
}

pub(crate) fn get_string<T: ReadTxn>(txn: &T, m: &MapRef, key: &str) -> Option<String> {
    match m.get(txn, key) {
        Some(Out::Any(Any::String(s))) => Some(s.to_string()),
        _ => None,
    }
}

pub(crate) fn node_id<T: ReadTxn>(txn: &T, node: &MapRef) -> Option<String> {
    get_string(txn, node, "id")
}

pub(crate) fn node_kind<T: ReadTxn>(txn: &T, node: &MapRef) -> Option<String> {
    get_string(txn, node, "kind")
}

pub(crate) fn children_of<T: ReadTxn>(txn: &T, array: &ArrayRef) -> Vec<MapRef> {
    array
        .iter(txn)
        .filter_map(|out| match out {
            Out::YMap(m) => Some(m),
            _ => None,
        })
        .collect()
}

pub(crate) fn child_maps<T: ReadTxn>(txn: &T, node: &MapRef) -> Vec<MapRef> {
    get_array(txn, node, "children").map_or_else(Vec::new, |a| children_of(txn, &a))
}

/// Index of the child map with `id` in the raw array (non-map entries count as
/// positions, so the index is valid for `get`, `insert` and `remove`).
pub(crate) fn find_child_index<T: ReadTxn>(txn: &T, children: &ArrayRef, id: &str) -> Option<u32> {
    children
        .iter(txn)
        .position(|out| match out {
            Out::YMap(m) => node_id(txn, &m).as_deref() == Some(id),
            _ => false,
        })
        .and_then(|i| u32::try_from(i).ok())
}

pub(crate) fn child_id_set<T: ReadTxn>(txn: &T, children: &ArrayRef) -> HashSet<String> {
    children_of(txn, children)
        .iter()
        .filter_map(|c| node_id(txn, c))
        .filter(|id| !id.is_empty())
        .collect()
}

pub(crate) fn parse_node_path(path: &str) -> Vec<&str> {
    path.split('.').filter(|s| !s.is_empty()).collect()
}

fn find_child_in_variant_pool<T: ReadTxn>(txn: &T, node: &MapRef, id: &str) -> Option<MapRef> {
    for child in child_maps(txn, node) {
        if node_kind(txn, &child).as_deref() != Some(VARIANT_POOL) {
            continue;
        }
        if let Some(inner) = get_array(txn, &child, "children")
            && let Some(idx) = find_child_index(txn, &inner, id)
            && let Some(Out::YMap(found)) = inner.get(txn, idx)
        {
            return Some(found);
        }
    }
    None
}

/// Walk a dot-joined id path from the tree root. A segment that is not a direct
/// child is looked up inside the variant-pool wrappers at that level.
pub(crate) fn resolve_node_path<T: ReadTxn>(txn: &T, tree: &MapRef, path: &str) -> Option<MapRef> {
    resolve_segments(txn, tree, &parse_node_path(path)).map(|(node, _)| node)
}

/// The same walk over id segments (an id may contain a dot), also returning the
/// depth of the node in the tree (the root is 1; a wrapper hop counts as a level).
pub(crate) fn resolve_segments<T: ReadTxn, S: AsRef<str>>(
    txn: &T,
    tree: &MapRef,
    parts: &[S],
) -> Option<(MapRef, usize)> {
    if parts.is_empty() {
        return Some((tree.clone(), 1));
    }
    if node_id(txn, tree).as_deref() != Some(parts[0].as_ref()) {
        return None;
    }
    let mut current = tree.clone();
    let mut depth = 1;
    for part in &parts[1..] {
        let part = part.as_ref();
        let children = get_array(txn, &current, "children")?;
        if let Some(idx) = find_child_index(txn, &children, part)
            && let Some(Out::YMap(next)) = children.get(txn, idx)
        {
            current = next;
            depth += 1;
            continue;
        }
        current = find_child_in_variant_pool(txn, &current, part)?;
        depth += 2;
    }
    Some((current, depth))
}

/// `{ id, kind, tier, data }` of a node, with prose flattened to strings.
pub(crate) struct NodeFields {
    pub kind: Value,
    pub tier: Value,
    pub data: Option<Map<String, Value>>,
}

pub(crate) fn snapshot_node_fields<T: ReadTxn>(
    txn: &T,
    node: &MapRef,
) -> Result<NodeFields, OpError> {
    let field = |key: &str| -> Result<Value, OpError> {
        node.get(txn, key)
            .map_or(Ok(Value::Null), |o| out_to_value(txn, &o))
    };
    let data = match node.get(txn, "data") {
        Some(Out::YMap(m)) => out_to_value(txn, &Out::YMap(m))?.as_object().cloned(),
        Some(Out::Any(Any::Map(m))) => any_to_value_checked(&Any::Map(m))?.as_object().cloned(),
        _ => None,
    };
    Ok(NodeFields {
        kind: field("kind")?,
        tier: field("tier")?,
        data,
    })
}

/// Deepest nesting of node maps the engine reads or writes. Trees come from
/// peers, so every walk that cannot be iterative checks this first.
pub(crate) const MAX_DEPTH: usize = 128;

fn too_deep() -> OpError {
    OpError::new(format!("node tree is deeper than {MAX_DEPTH} levels"))
}

/// Depth of a node subtree (a leaf is 1), refusing one deeper than [`MAX_DEPTH`] (iterative walk).
pub(crate) fn subtree_depth<T: ReadTxn>(txn: &T, node: &MapRef) -> Result<usize, OpError> {
    let mut stack = vec![(node.clone(), 1_usize)];
    let mut deepest = 1;
    while let Some((current, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            return Err(too_deep());
        }
        deepest = deepest.max(depth);
        for child in child_maps(txn, &current) {
            stack.push((child, depth + 1));
        }
    }
    Ok(deepest)
}

/// Refuse a node subtree deeper than [`MAX_DEPTH`].
pub(crate) fn ensure_depth<T: ReadTxn>(txn: &T, node: &MapRef) -> Result<(), OpError> {
    subtree_depth(txn, node).map(|_| ())
}

/// Depth of a JSON node (a leaf is 1), refusing one deeper than [`MAX_DEPTH`] (iterative walk).
pub(crate) fn value_depth(node: &Value) -> Result<usize, OpError> {
    let mut stack = vec![(node, 1_usize)];
    let mut deepest = 1;
    while let Some((current, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            return Err(too_deep());
        }
        deepest = deepest.max(depth);
        if let Some(children) = current.get("children").and_then(Value::as_array) {
            stack.extend(children.iter().map(|c| (c, depth + 1)));
        }
    }
    Ok(deepest)
}

/// Refuse a write that would put a subtree of `subtree` levels below a parent
/// at `parent_depth`: the engine never builds a tree its own reads refuse.
pub(crate) fn ensure_fits(parent_depth: usize, subtree: usize) -> Result<(), OpError> {
    if parent_depth + subtree > MAX_DEPTH {
        return Err(too_deep());
    }
    Ok(())
}

/// JSON of a node subtree (`Y.Map.toJSON()`); refuses a tree that is too deep.
pub(crate) fn node_json<T: ReadTxn>(txn: &T, node: &MapRef) -> Result<Value, OpError> {
    ensure_depth(txn, node)?;
    out_to_value(txn, &Out::YMap(node.clone()))
}

// ---------------------------------------------------------------------------
// Writes
// ---------------------------------------------------------------------------

/// Write a node onto an already integrated, empty `Y.Map`. Every `Y.Text` is
/// created empty and seeded afterwards, so it is parented to the document before
/// its first insert (Yjs drops inserts on detached text).
pub(crate) fn fill_node(txn: &mut TransactionMut, m: &MapRef, node: &Value) {
    let kind = node.get("kind").and_then(Value::as_str).unwrap_or("");
    m.insert(
        txn,
        "id",
        node.get("id").map_or(Any::Undefined, value_to_any),
    );
    m.insert(
        txn,
        "kind",
        node.get("kind").map_or(Any::Undefined, value_to_any),
    );
    m.insert(
        txn,
        "tier",
        node.get("tier").map_or(Any::Undefined, value_to_any),
    );
    if let Some(data) = node.get("data") {
        let keys = prose_keys(kind, Some(data));
        if keys.is_empty() {
            m.insert(txn, "data", value_to_any(data));
        } else {
            let yd = m.insert(txn, "data", MapPrelim::default());
            let object = data.as_object();
            let mut placed: HashSet<&str> = HashSet::new();
            if let Some(object) = object {
                for (k, v) in object {
                    if keys.contains(&k.as_str()) {
                        yd.insert(txn, k.as_str(), TextPrelim::new(""));
                    } else {
                        yd.insert(txn, k.as_str(), value_to_any(v));
                    }
                    placed.insert(k.as_str());
                }
            }
            for k in &keys {
                if !placed.contains(k) {
                    yd.insert(txn, *k, TextPrelim::new(""));
                }
            }
            for k in &keys {
                let seed = object.and_then(|o| o.get(*k)).and_then(Value::as_str);
                if let Some(seed) = seed
                    && !seed.is_empty()
                    && let Some(Out::YText(t)) = yd.get(&*txn, k)
                    && t.len(&*txn) == 0
                {
                    t.insert(txn, 0, seed);
                }
            }
        }
    }
    if let Some(p) = node.get("prompt") {
        m.insert(txn, "prompt", value_to_any(p));
    }
    if let Some(p) = node.get("applicable_langs") {
        m.insert(txn, "applicable_langs", value_to_any(p));
    }
    let selection = node.get("selection");
    let sel = m.insert(txn, "selection", MapPrelim::default());
    let active: Vec<Any> = selection
        .and_then(|s| s.get("active"))
        .and_then(Value::as_array)
        .map(|a| a.iter().map(value_to_any).collect())
        .unwrap_or_default();
    sel.insert(txn, "active", Any::Array(active.into()));
    for key in ["minCount", "maxCount"] {
        if let Some(v) = selection.and_then(|s| s.get(key)) {
            sel.insert(txn, key, value_to_any(v));
        }
    }
    let arr = m.insert(txn, "children", ArrayPrelim::default());
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        for child in children {
            let cm = arr.push_back(txn, MapPrelim::default());
            fill_node(txn, &cm, child);
        }
    }
}

fn not_a(what: &str, key: &str, owner: &str) -> OpError {
    OpError::new(format!("'{key}' of {owner} is not {what}"))
}

pub(crate) fn ensure_lang_map<T: ReadTxn>(
    txn: &T,
    root: &MapRef,
    lang: &str,
) -> Result<Option<MapRef>, OpError> {
    match root.get(txn, lang) {
        Some(Out::YMap(m)) => Ok(Some(m)),
        Some(_) => Err(not_a("a map", lang, "the profile root")),
        None => Ok(None),
    }
}

/// The map under `key`, created when absent; a value of another type is refused, never replaced.
pub(crate) fn ensure_map(
    txn: &mut TransactionMut,
    parent: &MapRef,
    key: &str,
) -> Result<MapRef, OpError> {
    match parent.get(&*txn, key) {
        Some(Out::YMap(m)) => Ok(m),
        Some(_) => Err(not_a("a map", key, "its parent")),
        None => Ok(parent.insert(txn, key, MapPrelim::default())),
    }
}

pub(crate) fn ensure_children_array(
    txn: &mut TransactionMut,
    node: &MapRef,
) -> Result<ArrayRef, OpError> {
    match node.get(&*txn, "children") {
        Some(Out::YArray(a)) => Ok(a),
        Some(_) => Err(not_a(
            "an array",
            "children",
            &format!("node '{}'", node_id(&*txn, node).unwrap_or_default()),
        )),
        None => Ok(node.insert(txn, "children", ArrayPrelim::default())),
    }
}

pub(crate) fn ensure_selection_map(
    txn: &mut TransactionMut,
    node: &MapRef,
) -> Result<MapRef, OpError> {
    match node.get(&*txn, "selection") {
        Some(Out::YMap(m)) => Ok(m),
        Some(_) => Err(not_a(
            "a map",
            "selection",
            &format!("node '{}'", node_id(&*txn, node).unwrap_or_default()),
        )),
        None => {
            let sel = node.insert(txn, "selection", MapPrelim::default());
            sel.insert(txn, "active", Any::Array(Vec::<Any>::new().into()));
            Ok(sel)
        }
    }
}

/// Make `data` a `Y.Map` (needed before `Y.Text` can live in it), keeping every
/// key of a plain-object `data`.
pub(crate) fn ensure_data_ymap(txn: &mut TransactionMut, node: &MapRef) -> MapRef {
    let existing = node.get(&*txn, "data");
    if let Some(Out::YMap(m)) = existing {
        return m;
    }
    let fresh = node.insert(txn, "data", MapPrelim::default());
    if let Some(Out::Any(Any::Map(entries))) = existing {
        for (k, v) in entries.iter() {
            fresh.insert(txn, k.as_str(), v.clone());
        }
    }
    fresh
}

/// Set a `Y.Text` field to `next` with one minimal middle edit.
pub(crate) fn apply_string_to_ytext(
    txn: &mut TransactionMut,
    data: &MapRef,
    key: &str,
    next: &str,
) {
    let Some(Out::YText(target)) = data.get(&*txn, key) else {
        data.insert(txn, key, TextPrelim::new(next));
        return;
    };
    let current = yrs::GetString::get_string(&target, &*txn);
    if current == next {
        return;
    }
    // A text with embedded content has more positions than characters, so a
    // diff over the string would drift: replace it whole.
    let length = target.len(&*txn);
    if u32::try_from(current.encode_utf16().count()).ok() != Some(length) {
        target.remove_range(txn, 0, length);
        target.insert(txn, 0, next);
        return;
    }
    let edit = compute_middle_edit(&current, next);
    if edit.remove_len > 0 {
        target.remove_range(txn, edit.position, edit.remove_len);
    }
    if !edit.insert.is_empty() {
        target.insert(txn, edit.position, &edit.insert);
    }
}

/// Merge a `data` patch into a node: prose keys become `Y.Text` edits, the rest
/// are last-writer-wins scalars. Kinds without prose keep a plain object.
pub(crate) fn apply_data_patch(
    txn: &mut TransactionMut,
    node: &MapRef,
    prose: &[&str],
    patch: &Map<String, Value>,
) -> Result<(), OpError> {
    if prose.is_empty() {
        let mut base: Map<String, Value> = match node.get(&*txn, "data") {
            Some(Out::Any(Any::Map(m))) => any_to_value_checked(&Any::Map(m))?
                .as_object()
                .cloned()
                .unwrap_or_default(),
            _ => Map::new(),
        };
        for (k, v) in patch {
            base.insert(k.clone(), v.clone());
        }
        node.insert(txn, "data", value_to_any(&Value::Object(base)));
        return Ok(());
    }
    let yd = ensure_data_ymap(txn, node);
    for (k, v) in patch {
        match v.as_str() {
            Some(s) if prose.contains(&k.as_str()) => apply_string_to_ytext(txn, &yd, k, s),
            _ => {
                yd.insert(txn, k.as_str(), value_to_any(v));
            }
        }
    }
    Ok(())
}

/// Remove `id` from the parent's `selection.active` after the child is gone.
pub(crate) fn scrub_from_selection(txn: &mut TransactionMut, node: &MapRef, id: &str) {
    let Some(sel) = get_map(&*txn, node, "selection") else {
        return;
    };
    let Some(Out::Any(Any::Array(active))) = sel.get(&*txn, "active") else {
        return;
    };
    let next: Vec<Any> = active
        .iter()
        .filter(|a| !matches!(a, Any::String(s) if &**s == id))
        .cloned()
        .collect();
    if next.len() != active.len() {
        sel.insert(txn, "active", Any::Array(next.into()));
    }
}
