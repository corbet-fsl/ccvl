//! The `cvl.*` action ops: each expands, by reading the current tree, into a
//! sequence of `node.*` / `selection.update` sub-ops that run as one batch.
use serde_json::{Value, json};
use yrs::{Array, MapRef, Out, ReadTxn};

use super::OpError;
use super::ops::{Ctx, fail, req_str};
use super::tree::{
    BULLET, LIST_ITEM, SUB, VARIANT_POOL, child_maps, find_child_index, get_array, node_id,
    node_kind, parse_node_path, resolve_node_path, snapshot_node_fields,
};

/// Dot path of a node from the tree root, wrappers included (iterative pre-order).
pub(crate) fn path_from_root<T: ReadTxn>(txn: &T, tree: &MapRef, id: &str) -> Option<String> {
    let root_id = node_id(txn, tree)?;
    if root_id == id {
        return Some(root_id);
    }
    let mut stack: Vec<(MapRef, String)> = child_maps(txn, tree)
        .into_iter()
        .rev()
        .filter_map(|c| node_id(txn, &c).map(|cid| (c, format!("{root_id}.{cid}"))))
        .collect();
    while let Some((node, path)) = stack.pop() {
        if node_id(txn, &node).as_deref() == Some(id) {
            return Some(path);
        }
        for child in child_maps(txn, &node).into_iter().rev() {
            if let Some(cid) = node_id(txn, &child) {
                stack.push((child, format!("{path}.{cid}")));
            }
        }
    }
    None
}

/// The map directly containing `id` in its `children`, and the index there (iterative pre-order).
pub(crate) fn find_direct_parent_of_id<T: ReadTxn>(
    txn: &T,
    root: &MapRef,
    id: &str,
) -> Option<(MapRef, u32)> {
    let mut stack = vec![root.clone()];
    while let Some(node) = stack.pop() {
        if let Some(children) = get_array(txn, &node, "children") {
            if let Some(idx) = find_child_index(txn, &children, id) {
                return Some((node, idx));
            }
            stack.extend(child_maps(txn, &node).into_iter().rev());
        }
    }
    None
}

fn parent_path_of(node_path: &str) -> Result<String, OpError> {
    let parts = parse_node_path(node_path);
    if parts.len() < 2 {
        return fail(format!(
            "cvl: node_path '{node_path}' must include parent + node id"
        ));
    }
    Ok(parts[..parts.len() - 1].join("."))
}

fn child_index_at<T: ReadTxn>(txn: &T, parent: &MapRef, id: &str) -> Option<u32> {
    let children = get_array(txn, parent, "children")?;
    find_child_index(txn, &children, id)
}

fn kind_is<T: ReadTxn>(txn: &T, node: &MapRef, kind: &str) -> bool {
    node_kind(txn, node).as_deref() == Some(kind)
}

/// Expand a `cvl.*` action; the caller applies the result as one batch.
pub(crate) fn expand<T: ReadTxn>(
    ctx: &mut Ctx<'_>,
    txn: &T,
    tree: &MapRef,
    op: &Value,
) -> Result<Vec<Value>, OpError> {
    match req_str(op, "cvl", "kind")? {
        "cvl.duplicate_as_variant" => duplicate_as_variant(ctx, txn, tree, op),
        "cvl.break_out" => break_out(txn, tree, op),
        "cvl.promote_sub" => promote_sub(txn, tree, op),
        _ => demote_bullet(txn, tree, op),
    }
}

fn duplicate_as_variant<T: ReadTxn>(
    ctx: &mut Ctx<'_>,
    txn: &T,
    tree: &MapRef,
    op: &Value,
) -> Result<Vec<Value>, OpError> {
    let kind = "cvl.duplicate_as_variant";
    let branch = req_str(op, kind, "branch_path")?;
    let node_path = req_str(op, kind, "node_path")?;
    let Some(source) = resolve_node_path(txn, tree, node_path) else {
        return fail(format!("{kind}: source '{node_path}' not found"));
    };
    let parent_path = parent_path_of(node_path)?;
    let Some(parent) = resolve_node_path(txn, tree, &parent_path) else {
        return fail(format!("{kind}: parent '{parent_path}' not found"));
    };
    let fields = snapshot_node_fields(txn, &source);
    let source_id = node_id(txn, &source).unwrap_or_default();
    let duplicate_id = match op.get("new_id").and_then(Value::as_str) {
        Some(id) => id.to_string(),
        None => (ctx.ids)(),
    };
    let mut data = fields.data.clone().unwrap_or_default();
    if let Some(overrides) = op.get("data_overrides").and_then(Value::as_object) {
        for (k, v) in overrides {
            data.insert(k.clone(), v.clone());
        }
    }
    let duplicate = |parent_path: &str| {
        json!({
            "kind": "node.create",
            "branch_path": branch,
            "parent_path": parent_path,
            "node": {
                "id": duplicate_id,
                "kind": fields.kind,
                "tier": fields.tier,
                "data": Value::Object(data.clone()),
                "selection": { "active": [] },
            },
        })
    };

    if kind_is(txn, &parent, VARIANT_POOL) {
        return Ok(vec![duplicate(&parent_path)]);
    }

    let wrapper_id = match op.get("new_wrapper_id").and_then(Value::as_str) {
        Some(id) => id.to_string(),
        None => format!("vp-{}", (ctx.ids)().chars().take(8).collect::<String>()),
    };
    let wrapper_path = format!("{parent_path}.{wrapper_id}");
    let source_index = child_index_at(txn, &parent, &source_id);
    let mut ops = vec![json!({
        "kind": "node.create",
        "branch_path": branch,
        "parent_path": parent_path,
        "node": {
            "id": wrapper_id,
            "kind": VARIANT_POOL,
            "tier": "HYBRID",
            "selection": { "active": [], "maxCount": 1 },
        },
    })];
    if let Some(index) = source_index {
        ops.push(json!({
            "kind": "node.move",
            "branch_path": branch,
            "from_path": wrapper_path,
            "to_parent_path": parent_path,
            "to_index": index,
        }));
    }
    ops.push(json!({
        "kind": "node.move",
        "branch_path": branch,
        "from_path": node_path,
        "to_parent_path": wrapper_path,
        "to_index": 0,
    }));
    ops.push(duplicate(&wrapper_path));
    Ok(ops)
}

fn break_out<T: ReadTxn>(txn: &T, tree: &MapRef, op: &Value) -> Result<Vec<Value>, OpError> {
    let kind = "cvl.break_out";
    let branch = req_str(op, kind, "branch_path")?;
    let node_path = req_str(op, kind, "node_path")?;
    let Some(member) = resolve_node_path(txn, tree, node_path) else {
        return fail(format!("{kind}: node '{node_path}' not found"));
    };
    let Some(member_id) = node_id(txn, &member) else {
        return fail(format!("{kind}: node '{node_path}' has no id"));
    };
    let Some((wrapper, _)) = find_direct_parent_of_id(txn, tree, &member_id) else {
        return fail(format!("{kind}: node '{node_path}' parent not found"));
    };
    if !kind_is(txn, &wrapper, VARIANT_POOL) {
        return fail(format!(
            "{kind}: node '{node_path}' is not inside a variant-pool wrapper"
        ));
    }
    let wrapper_id = node_id(txn, &wrapper).unwrap_or_default();
    let Some((grand, wrapper_index)) = find_direct_parent_of_id(txn, tree, &wrapper_id) else {
        return fail(format!("{kind}: wrapper '{wrapper_id}' parent not found"));
    };
    let grand_id = node_id(txn, &grand).unwrap_or_default();
    let members = child_maps(txn, &wrapper);
    let count = members.len();
    let wrapper_path = path_from_root(txn, tree, &wrapper_id);
    let grand_path = path_from_root(txn, tree, &grand_id);
    let (Some(wrapper_path), Some(grand_path)) = (wrapper_path, grand_path) else {
        return fail(format!(
            "{kind}: failed to compute paths for wrapper '{wrapper_id}'"
        ));
    };
    let delete_wrapper = json!({
        "kind": "node.delete",
        "branch_path": branch,
        "node_path": format!("{grand_path}.{wrapper_id}"),
    });
    let mut ops = Vec::new();
    if count <= 1 {
        if count == 1 {
            ops.push(json!({
                "kind": "node.move",
                "branch_path": branch,
                "from_path": format!("{wrapper_path}.{member_id}"),
                "to_parent_path": grand_path,
                "to_index": wrapper_index + 1,
            }));
        }
        ops.push(delete_wrapper);
        return Ok(ops);
    }
    ops.push(json!({
        "kind": "node.move",
        "branch_path": branch,
        "from_path": node_path,
        "to_parent_path": grand_path,
        "to_index": wrapper_index + 1,
    }));
    if count == 2 {
        let remaining = members
            .iter()
            .filter_map(|m| node_id(txn, m))
            .find(|id| *id != member_id);
        if let Some(remaining_id) = remaining {
            ops.push(json!({
                "kind": "node.move",
                "branch_path": branch,
                "from_path": format!("{wrapper_path}.{remaining_id}"),
                "to_parent_path": grand_path,
                "to_index": wrapper_index + 2,
            }));
            ops.push(delete_wrapper);
        }
    }
    Ok(ops)
}

/// Climb from a node's direct parent to the nearest bullet and list-item ancestors.
fn climb_to_bullet_and_item<T: ReadTxn>(
    txn: &T,
    tree: &MapRef,
    start: &MapRef,
) -> (Option<MapRef>, Option<MapRef>) {
    let mut bullet: Option<MapRef> = None;
    let mut current = Some(start.clone());
    while let Some(cur) = current {
        if kind_is(txn, &cur, BULLET) && bullet.is_none() {
            bullet = Some(cur.clone());
        }
        if kind_is(txn, &cur, LIST_ITEM) {
            return (bullet, Some(cur));
        }
        let Some(id) = node_id(txn, &cur) else { break };
        if cur == *tree {
            break;
        }
        current = find_direct_parent_of_id(txn, tree, &id).map(|(p, _)| p);
    }
    (bullet, None)
}

fn find_enclosing_list_item<T: ReadTxn>(txn: &T, tree: &MapRef, start: &MapRef) -> Option<MapRef> {
    let mut current = Some(start.clone());
    while let Some(cur) = current {
        if kind_is(txn, &cur, LIST_ITEM) {
            return Some(cur);
        }
        let id = node_id(txn, &cur)?;
        if cur == *tree {
            return None;
        }
        current = find_direct_parent_of_id(txn, tree, &id).map(|(p, _)| p);
    }
    None
}

/// Index of `child_id` in `parent.children` (raw array index); a child inside a wrapper reports the wrapper's index.
fn position_considering_wrappers<T: ReadTxn>(
    txn: &T,
    parent: &MapRef,
    child_id: &str,
) -> Option<u32> {
    let children = get_array(txn, parent, "children")?;
    for (i, out) in children.iter(txn).enumerate() {
        let Out::YMap(child) = out else { continue };
        let index = u32::try_from(i).ok()?;
        if node_id(txn, &child).as_deref() == Some(child_id) {
            return Some(index);
        }
        if kind_is(txn, &child, VARIANT_POOL)
            && let Some(inner) = get_array(txn, &child, "children")
            && find_child_index(txn, &inner, child_id).is_some()
        {
            return Some(index);
        }
    }
    None
}

/// Bullet ids of a list item in display order, wrappers expanded; the one before `bullet_id`.
fn previous_bullet_id<T: ReadTxn>(txn: &T, list_item: &MapRef, bullet_id: &str) -> Option<String> {
    let mut flat: Vec<String> = Vec::new();
    for child in child_maps(txn, list_item) {
        if kind_is(txn, &child, BULLET) {
            flat.extend(node_id(txn, &child));
        } else if kind_is(txn, &child, VARIANT_POOL) {
            for inner in child_maps(txn, &child) {
                if kind_is(txn, &inner, BULLET) {
                    flat.extend(node_id(txn, &inner));
                }
            }
        }
    }
    let idx = flat.iter().position(|id| id == bullet_id)?;
    if idx == 0 {
        None
    } else {
        Some(flat[idx - 1].clone())
    }
}

fn promote_sub<T: ReadTxn>(txn: &T, tree: &MapRef, op: &Value) -> Result<Vec<Value>, OpError> {
    let kind = "cvl.promote_sub";
    let branch = req_str(op, kind, "branch_path")?;
    let node_path = req_str(op, kind, "node_path")?;
    let Some(sub) = resolve_node_path(txn, tree, node_path) else {
        return fail(format!("{kind}: node '{node_path}' not found"));
    };
    if !kind_is(txn, &sub, SUB) {
        return fail(format!("{kind}: node '{node_path}' is not a sub-Node"));
    }
    let sub_id = node_id(txn, &sub).unwrap_or_default();
    let Some((direct, _)) = find_direct_parent_of_id(txn, tree, &sub_id) else {
        return fail(format!("{kind}: sub '{sub_id}' has no parent"));
    };
    let (bullet, item) = climb_to_bullet_and_item(txn, tree, &direct);
    let (Some(bullet), Some(item)) = (bullet, item) else {
        return fail(format!(
            "{kind}: sub '{sub_id}' is not inside bullet \u{2192} list-item"
        ));
    };
    let bullet_id = node_id(txn, &bullet).unwrap_or_default();
    let item_id = node_id(txn, &item).unwrap_or_default();
    let Some(item_path) = path_from_root(txn, tree, &item_id) else {
        return fail(format!("{kind}: failed to compute path for list-item"));
    };
    let Some(position) = position_considering_wrappers(txn, &item, &bullet_id) else {
        return fail(format!(
            "{kind}: failed to locate bullet '{bullet_id}' inside list-item"
        ));
    };
    Ok(vec![
        json!({
            "kind": "node.move",
            "branch_path": branch,
            "from_path": node_path,
            "to_parent_path": item_path,
            "to_index": position + 1,
        }),
        json!({
            "kind": "node.update",
            "branch_path": branch,
            "node_path": format!("{item_path}.{sub_id}"),
            "patch": { "kind": BULLET },
        }),
    ])
}

fn demote_bullet<T: ReadTxn>(txn: &T, tree: &MapRef, op: &Value) -> Result<Vec<Value>, OpError> {
    let kind = "cvl.demote_bullet";
    let branch = req_str(op, kind, "branch_path")?;
    let node_path = req_str(op, kind, "node_path")?;
    let Some(bullet) = resolve_node_path(txn, tree, node_path) else {
        return fail(format!("{kind}: node '{node_path}' not found"));
    };
    if !kind_is(txn, &bullet, BULLET) {
        return fail(format!("{kind}: node '{node_path}' is not a bullet-Node"));
    }
    let bullet_id = node_id(txn, &bullet).unwrap_or_default();
    let Some((direct, _)) = find_direct_parent_of_id(txn, tree, &bullet_id) else {
        return fail(format!("{kind}: bullet '{bullet_id}' has no parent"));
    };
    let Some(item) = find_enclosing_list_item(txn, tree, &direct) else {
        return fail(format!(
            "{kind}: bullet '{bullet_id}' is not inside a list-item"
        ));
    };
    let item_id = node_id(txn, &item).unwrap_or_default();
    let Some(item_path) = path_from_root(txn, tree, &item_id) else {
        return fail(format!("{kind}: failed to compute list-item path"));
    };
    let Some(previous) = previous_bullet_id(txn, &item, &bullet_id) else {
        return fail(format!(
            "{kind}: bullet '{bullet_id}' has no previous bullet to demote into"
        ));
    };
    let previous_path = format!("{item_path}.{previous}");
    Ok(vec![
        json!({
            "kind": "node.move",
            "branch_path": branch,
            "from_path": node_path,
            "to_parent_path": previous_path,
            "to_index": 9_007_199_254_740_991_u64,
        }),
        json!({
            "kind": "node.update",
            "branch_path": branch,
            "node_path": format!("{previous_path}.{bullet_id}"),
            "patch": { "kind": SUB },
        }),
    ])
}
