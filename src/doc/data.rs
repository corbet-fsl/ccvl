//! The data sub-documents: `jobs`, `settings`, `layout` and `order-state`.
//!
//! `job.*`, `settings.update`, `layout.update` and `order_state.update` with the semantics of
//! `applyOp` in `@cv/mutations`: same checks, same order, same messages. Values are plain JSON
//! (`Any`) except the job envelope: `jobs[job_id]` is a `Y.Map` of `url` and an `attributes`
//! `Y.Map`, and the record-shaped attributes merge per key.
use serde_json::{Map, Value, json};
use yrs::{Any, Map as _, MapPrelim, MapRef, Out, ReadTxn, TransactionMut};

use super::OpError;
use super::convert::{out_to_value, value_to_any};
use super::ops::{Ctx, fail};
use crate::jobid::job_id_from_url;
use crate::transfer::jsnum::{number_to_string, order_properties};

/// Job attribute fields whose values merge per key instead of replacing the container.
const JOB_MAP_FIELDS: [&str; 4] = ["evaluations", "eval_reasoning", "origins", "commute_config"];

/// Keys a job envelope owns: refused in `attributes` and `patch`.
const ENVELOPE_FIELDS: [&str; 11] = [
    "url",
    "id",
    "job_id",
    "jobId",
    "metadata",
    "ad_id",
    "adId",
    "role_id",
    "roleId",
    "source_ad_id",
    "sourceAdId",
];

/// Keys stripped from attribute values at any depth.
const HIDDEN_ATTRIBUTE_FIELDS: [&str; 9] = [
    "job_id",
    "jobId",
    "metadata",
    "ad_id",
    "adId",
    "role_id",
    "roleId",
    "source_ad_id",
    "sourceAdId",
];

const NO_ENTRIES: &str = "Cannot convert undefined or null to object";

/// The own enumerable properties of a JSON object in JavaScript order.
fn entries(map: &Map<String, Value>) -> Vec<(String, Value)> {
    order_properties(map.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
}

fn require_data_root(ctx: &Ctx<'_>, kind: &str, root: &str) -> Result<MapRef, OpError> {
    ctx.roots.data(root).cloned().map_or_else(
        || fail(format!("{kind}: '{root}' is not part of this document")),
        Ok,
    )
}

/// `JSON.stringify` of a value parsed from JSON: JavaScript number text, property order.
fn js_stringify(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            let f = n.as_f64().unwrap_or(0.0);
            out.push_str(&if f.is_finite() {
                number_to_string(f)
            } else {
                "null".into()
            });
        }
        Value::String(s) => out.push_str(&serde_json::to_string(s).unwrap_or_default()),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                js_stringify(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (i, (k, v)) in entries(map).iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).unwrap_or_default());
                out.push(':');
                js_stringify(v, out);
            }
            out.push('}');
        }
    }
}

/// `stripFields`: drop hidden keys at any depth, also inside strings that hold a JSON container.
fn strip_hidden(value: &Value) -> Value {
    match value {
        Value::String(s) => {
            let trimmed = js_trim(s);
            if trimmed.starts_with(['{', '['])
                && let Ok(parsed @ (Value::Object(_) | Value::Array(_))) =
                    serde_json::from_str::<Value>(trimmed)
            {
                let mut text = String::new();
                js_stringify(&strip_hidden(&parsed), &mut text);
                return Value::String(text);
            }
            value.clone()
        }
        Value::Array(items) => Value::Array(items.iter().map(strip_hidden).collect()),
        Value::Object(map) => Value::Object(
            entries(map)
                .into_iter()
                .filter(|(k, _)| !HIDDEN_ATTRIBUTE_FIELDS.contains(&k.as_str()))
                .map(|(k, v)| (k, strip_hidden(&v)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn js_trim(s: &str) -> &str {
    crate::transfer::jsnum::js_trim(s)
}

/// `assertJobAttributesAllowed` plus the value sanitising of `sanitizedJobAttributes` / `sanitizedJobPatch`.
fn sanitized(kind: &str, fields: &Map<String, Value>) -> Result<Vec<(String, Value)>, OpError> {
    let ordered = entries(fields);
    for (key, _) in &ordered {
        if ENVELOPE_FIELDS.contains(&key.as_str()) {
            return fail(format!(
                "{kind}: '{key}' is reserved for the workspace job envelope"
            ));
        }
    }
    Ok(ordered
        .into_iter()
        .map(|(k, v)| (k, strip_hidden(&v)))
        .collect())
}

fn job_entry<T: ReadTxn>(txn: &T, jobs: &MapRef, id: &str) -> Option<MapRef> {
    match jobs.get(txn, id) {
        Some(Out::YMap(m)) => Some(m),
        _ => None,
    }
}

pub(crate) fn job_create(
    ctx: &mut Ctx<'_>,
    txn: &mut TransactionMut,
    op: &Value,
) -> Result<(), OpError> {
    let kind = "job.create";
    let jobs = require_data_root(ctx, kind, "jobs")?;
    let url = match op.get("url").and_then(Value::as_str) {
        Some(u) if !u.is_empty() => u,
        _ => return fail("job.create: url is required and must be a non-empty string"),
    };
    let attributes = match op.get("attributes") {
        None | Some(Value::Null | Value::Bool(false)) => Vec::new(),
        Some(Value::Object(map)) => sanitized(kind, map)?,
        Some(_) => return fail("job.create: 'attributes' must be an object"),
    };
    let job_id = job_id_from_url(url).map_err(OpError::new)?;
    if jobs.contains_key(&*txn, &job_id) {
        // Idempotent: the same URL twice collapses on the existing row.
        return Ok(());
    }
    let entry = jobs.insert(txn, job_id.as_str(), MapPrelim::default());
    entry.insert(txn, "url", url);
    let attrs = entry.insert(txn, "attributes", MapPrelim::default());
    for (k, v) in &attributes {
        attrs.insert(txn, k.as_str(), value_to_any(v));
    }
    Ok(())
}

/// Merge `patch` into the value of `key`: record-shaped values merge per key.
fn merge_map_field(
    txn: &mut TransactionMut,
    attributes: &MapRef,
    key: &str,
    patch: &Map<String, Value>,
) {
    match attributes.get(&*txn, key) {
        Some(Out::YMap(existing)) => {
            for (k, v) in entries(patch) {
                existing.insert(txn, k.as_str(), value_to_any(&v));
            }
        }
        Some(Out::Any(Any::Map(existing))) => {
            let mut merged: Vec<(String, Any)> = existing
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            for (k, v) in entries(patch) {
                let any = value_to_any(&v);
                if let Some(slot) = merged.iter_mut().find(|(name, _)| *name == k) {
                    slot.1 = any;
                } else {
                    merged.push((k, any));
                }
            }
            attributes.insert(
                txn,
                key,
                Any::Map(
                    merged
                        .into_iter()
                        .collect::<std::collections::HashMap<_, _>>()
                        .into(),
                ),
            );
        }
        _ => {
            attributes.insert(txn, key, value_to_any(&Value::Object(patch.clone())));
        }
    }
}

/// `ensureNestedMap`: the `Y.Map` at `key`, upgrading a plain record in place.
fn ensure_nested_map(txn: &mut TransactionMut, container: &MapRef, key: &str) -> MapRef {
    match container.get(&*txn, key) {
        Some(Out::YMap(m)) => m,
        Some(Out::Any(Any::Map(existing))) => {
            let pairs: Vec<(String, Any)> = existing
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let next = container.insert(txn, key, MapPrelim::default());
            for (k, v) in pairs {
                next.insert(txn, k.as_str(), v);
            }
            next
        }
        _ => container.insert(txn, key, MapPrelim::default()),
    }
}

fn record_votes(
    ctx: &Ctx<'_>,
    txn: &mut TransactionMut,
    attributes: &MapRef,
    evaluations: &Map<String, Value>,
) {
    if ctx.actor != super::ActorClass::Human {
        return;
    }
    let Some(actor_id) = ctx.actor_id.filter(|id| !id.is_empty()) else {
        return;
    };
    let votes = ensure_nested_map(txn, attributes, "evaluation_votes");
    for (criterion, value) in entries(evaluations) {
        let vote_key = serde_json::to_string(&json!([criterion, actor_id])).unwrap_or_default();
        let vote = json!({ "criterion_id": criterion, "actor_id": actor_id, "value": value });
        votes.insert(txn, vote_key.as_str(), value_to_any(&vote));
    }
}

pub(crate) fn job_update(
    ctx: &mut Ctx<'_>,
    txn: &mut TransactionMut,
    op: &Value,
) -> Result<(), OpError> {
    let kind = "job.update";
    let jobs = require_data_root(ctx, kind, "jobs")?;
    let job_id = match op.get("job_id").and_then(Value::as_str) {
        Some(id) if !id.is_empty() => id,
        _ => return fail("job.update: job_id is required"),
    };
    let Some(entry) = job_entry(&*txn, &jobs, job_id) else {
        return fail(format!("job.update: job '{job_id}' not found"));
    };
    let patch = match op.get("patch") {
        Some(Value::Object(map)) => sanitized(kind, map)?,
        None | Some(Value::Null) => return fail(NO_ENTRIES),
        Some(_) => return fail("job.update: 'patch' must be an object"),
    };
    let attributes = match entry.get(&*txn, "attributes") {
        Some(Out::YMap(m)) => m,
        _ => entry.insert(txn, "attributes", MapPrelim::default()),
    };
    for (k, v) in patch {
        match &v {
            Value::Object(record) if JOB_MAP_FIELDS.contains(&k.as_str()) => {
                merge_map_field(txn, &attributes, &k, record);
                if k == "evaluations" {
                    record_votes(ctx, txn, &attributes, record);
                }
            }
            _ => {
                attributes.insert(txn, k.as_str(), value_to_any(&v));
            }
        }
    }
    Ok(())
}

pub(crate) fn job_delete(
    ctx: &mut Ctx<'_>,
    txn: &mut TransactionMut,
    op: &Value,
) -> Result<(), OpError> {
    let kind = "job.delete";
    let jobs = require_data_root(ctx, kind, "jobs")?;
    let job_id = match op.get("job_id").and_then(Value::as_str) {
        Some(id) if !id.is_empty() => id,
        _ => return fail("job.delete: job_id is required"),
    };
    if !jobs.contains_key(&*txn, job_id) {
        return fail(format!("job.delete: job '{job_id}' not found"));
    }
    jobs.remove(txn, job_id);
    Ok(())
}

/// `settings.update`, `layout.update` and `order_state.update`: set each key of `patch`.
pub(crate) fn flat_update(
    ctx: &mut Ctx<'_>,
    txn: &mut TransactionMut,
    op: &Value,
    kind: &str,
) -> Result<(), OpError> {
    let root_name = match kind {
        "settings.update" => "settings",
        "layout.update" => "layout",
        _ => "order_state",
    };
    let root = require_data_root(ctx, kind, root_name)?;
    let patch = match op.get("patch") {
        Some(Value::Object(map)) => map,
        None | Some(Value::Null) => return fail(NO_ENTRIES),
        Some(_) => return fail(format!("{kind}: 'patch' must be an object")),
    };
    for (k, v) in entries(patch) {
        root.insert(txn, k.as_str(), value_to_any(&v));
    }
    Ok(())
}

/// Plain JSON of a stored value: maps, arrays and text as their JSON, no structure markers.
fn plain<T: ReadTxn>(txn: &T, out: &Out) -> Result<Value, OpError> {
    use yrs::{Array as _, GetString};
    Ok(match out {
        Out::YMap(m) => {
            let mut o = Map::new();
            for (k, v) in m.iter(txn) {
                o.insert(k.to_string(), plain(txn, &v)?);
            }
            Value::Object(o)
        }
        Out::YArray(a) => Value::Array(
            a.iter(txn)
                .map(|v| plain(txn, &v))
                .collect::<Result<_, _>>()?,
        ),
        Out::YText(t) => Value::String(t.get_string(txn)),
        other => out_to_value(txn, other)?,
    })
}

/// The row keys of a flat root, sorted.
pub(crate) fn keys<T: ReadTxn>(txn: &T, root: &MapRef) -> Vec<String> {
    let mut ids: Vec<String> = root.keys(txn).map(str::to_string).collect();
    ids.sort();
    ids
}

/// Rows of a flat root as `{ "<key>": <plain JSON> }`; `only` limits it to those keys (absent
/// ones are left out). Jobs are `{ "url", "attributes" }`.
pub(crate) fn rows<T: ReadTxn>(
    txn: &T,
    root: &MapRef,
    only: Option<&[String]>,
) -> Result<Value, OpError> {
    let mut o = Map::new();
    let wanted: Vec<String> = match only {
        Some(ids) => ids.to_vec(),
        None => keys(txn, root),
    };
    for id in wanted {
        if let Some(out) = root.get(txn, &id) {
            o.insert(id, plain(txn, &out)?);
        }
    }
    Ok(Value::Object(o))
}
