//! Changed-node tracking from Yrs deep-observer events.
//!
//! A node is reported, by location, when its map, `selection`, `children`
//! array, `data` or prose text changed in the transaction, or when it was
//! created (or re-created by a move) in it. A removed node is not reported: the
//! parent's `children` changed, so the parent is. Format maps are reported as
//! `<profile>.<lang>`. Every walk is iterative: trees come from peers.
use std::collections::BTreeSet;

use yrs::types::{Change, EntryChange, Event, Events, PathSegment};
use yrs::{Array, Map as _, MapRef, Out, ReadTxn, TransactionMut};

use super::NodeRef;
use super::tree::{MAX_DEPTH, child_maps, node_id};

#[derive(Default)]
pub(crate) struct Tracker {
    pub nodes: BTreeSet<NodeRef>,
    pub formats: BTreeSet<String>,
}

/// Report `node` and everything below it down to `MAX_DEPTH` levels; `path` is
/// the path of `node`. Deeper nodes are not reachable by any read, and the
/// walk keeps one path per node on the stack, so its memory is bounded by the cap.
fn collect_subtree<T: ReadTxn>(
    txn: &T,
    branch: &str,
    node: &MapRef,
    path: &[String],
    out: &mut BTreeSet<NodeRef>,
) {
    let mut stack = vec![(node.clone(), path.to_vec())];
    while let Some((current, current_path)) = stack.pop() {
        out.insert(NodeRef::new(branch, current_path.clone()));
        if current_path.len() >= MAX_DEPTH {
            continue;
        }
        for child in child_maps(txn, &current) {
            if let Some(id) = node_id(txn, &child) {
                let mut child_path = current_path.clone();
                child_path.push(id);
                stack.push((child, child_path));
            }
        }
    }
}

fn collect_tree<T: ReadTxn>(txn: &T, branch: &str, tree: &MapRef, out: &mut BTreeSet<NodeRef>) {
    if let Some(id) = node_id(txn, tree) {
        collect_subtree(txn, branch, tree, &[id], out);
    }
}

fn new_value(change: &EntryChange) -> Option<&Out> {
    match change {
        EntryChange::Inserted(new) | EntryChange::Updated(_, new) => Some(new),
        EntryChange::Removed(_) => None,
    }
}

/// A language map was set on a profile root: everything inside it is new.
fn record_lang<T: ReadTxn>(
    root_name: &str,
    lang: &str,
    txn: &T,
    lang_map: &MapRef,
    t: &mut Tracker,
) {
    let branch = format!("{root_name}.{lang}");
    if lang_map.get(txn, "format").is_some() {
        t.formats.insert(branch.clone());
    }
    if let Some(Out::YMap(tree)) = lang_map.get(txn, "tree") {
        collect_tree(txn, &branch, &tree, &mut t.nodes);
    }
}

fn record_event(
    root_name: &str,
    root: &MapRef,
    txn: &TransactionMut,
    event: &Event,
    t: &mut Tracker,
) {
    let path: Vec<PathSegment> = event.path().into_iter().collect();
    match path.as_slice() {
        [] => {
            if let Event::Map(me) = event {
                for (lang, change) in me.keys(txn) {
                    if let Some(Out::YMap(lm)) = new_value(change) {
                        record_lang(root_name, lang, txn, lm, t);
                    }
                }
            }
        }
        [PathSegment::Key(lang)] => {
            if let Event::Map(me) = event {
                let branch = format!("{root_name}.{lang}");
                for (key, change) in me.keys(txn) {
                    match &**key {
                        "tree" => {
                            if let Some(Out::YMap(tree)) = new_value(change) {
                                collect_tree(txn, &branch, tree, &mut t.nodes);
                            }
                        }
                        "format" => {
                            t.formats.insert(branch.clone());
                        }
                        _ => {}
                    }
                }
            }
        }
        [PathSegment::Key(lang), PathSegment::Key(section), rest @ ..] => {
            if &**section == "format" {
                t.formats.insert(format!("{root_name}.{lang}"));
            } else if &**section == "tree" {
                record_in_tree(root_name, root, txn, lang, rest, event, t);
            }
        }
        _ => {}
    }
}

fn record_in_tree(
    root_name: &str,
    root: &MapRef,
    txn: &TransactionMut,
    lang: &str,
    rest: &[PathSegment],
    event: &Event,
    t: &mut Tracker,
) -> Option<()> {
    let Some(Out::YMap(lang_map)) = root.get(txn, lang) else {
        return None;
    };
    let Some(Out::YMap(tree)) = lang_map.get(txn, "tree") else {
        return None;
    };
    let branch = format!("{root_name}.{lang}");
    let mut owner_path = vec![node_id(txn, &tree)?];
    let mut current = Out::YMap(tree);
    let mut previous: Option<String> = None;
    for segment in rest {
        match segment {
            PathSegment::Key(key) => {
                let Out::YMap(m) = &current else { return None };
                let next = m.get(txn, key)?;
                current = next;
                previous = Some(key.to_string());
            }
            PathSegment::Index(i) => {
                let Out::YArray(a) = &current else {
                    return None;
                };
                let next = a.get(txn, *i)?;
                current = next;
                if previous.as_deref() == Some("children")
                    && let Out::YMap(m) = &current
                {
                    if owner_path.len() >= MAX_DEPTH {
                        return None;
                    }
                    owner_path.push(node_id(txn, m)?);
                }
                previous = None;
            }
        }
    }
    t.nodes.insert(NodeRef::new(&branch, owner_path.clone()));
    if let Event::Array(ae) = event {
        for change in ae.delta(txn) {
            if let Change::Added(added) = change {
                for out in added {
                    if let Out::YMap(m) = out
                        && let Some(id) = node_id(txn, m)
                        && owner_path.len() < MAX_DEPTH
                    {
                        let mut path = owner_path.clone();
                        path.push(id);
                        collect_subtree(txn, &branch, m, &path, &mut t.nodes);
                    }
                }
            }
        }
    }
    Some(())
}

pub(crate) fn record(
    root_name: &str,
    root: &MapRef,
    txn: &TransactionMut,
    events: &Events,
    t: &mut Tracker,
) {
    for event in events.iter() {
        record_event(root_name, root, txn, event, t);
    }
}
