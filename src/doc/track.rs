//! Changed-id tracking from Yrs deep-observer events.
//!
//! A node id is reported when its map, `selection`, `children` array, `data` or
//! prose text changed in the transaction, or when the node was created (or
//! re-created by a move) in it. Ids of removed nodes are not reported: the
//! parent's `children` changed, so the parent is. Format maps are reported as
//! `<profile>.<lang>`.
use std::collections::BTreeSet;

use yrs::types::{Change, EntryChange, Event, Events, PathSegment};
use yrs::{Array, Map as _, MapRef, Out, ReadTxn, TransactionMut};

use super::tree::{child_maps, node_id};

#[derive(Default)]
pub(crate) struct Tracker {
    pub nodes: BTreeSet<String>,
    pub formats: BTreeSet<String>,
}

fn collect_ids<T: ReadTxn>(txn: &T, node: &MapRef, out: &mut BTreeSet<String>) {
    if let Some(id) = node_id(txn, node) {
        out.insert(id);
    }
    for child in child_maps(txn, node) {
        collect_ids(txn, &child, out);
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
    if lang_map.get(txn, "format").is_some() {
        t.formats.insert(format!("{root_name}.{lang}"));
    }
    if let Some(Out::YMap(tree)) = lang_map.get(txn, "tree") {
        collect_ids(txn, &tree, &mut t.nodes);
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
                for (key, change) in me.keys(txn) {
                    match &**key {
                        "tree" => {
                            if let Some(Out::YMap(tree)) = new_value(change) {
                                collect_ids(txn, tree, &mut t.nodes);
                            }
                        }
                        "format" => {
                            t.formats.insert(format!("{root_name}.{lang}"));
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
                record_in_tree(root, txn, lang, rest, event, t);
            }
        }
        _ => {}
    }
}

fn record_in_tree(
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
    let mut owner = node_id(txn, &tree);
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
                    owner = node_id(txn, m);
                }
                previous = None;
            }
        }
    }
    if let Some(id) = owner {
        t.nodes.insert(id);
    }
    if let Event::Array(ae) = event {
        for change in ae.delta(txn) {
            if let Change::Added(added) = change {
                for out in added {
                    if let Out::YMap(m) = out {
                        collect_ids(txn, m, &mut t.nodes);
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
