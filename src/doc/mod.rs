//! The Yrs document engine: the `CareerVector` `Y.Doc` layout and op catalogue (v10)
//! behind a small boundary, with no I/O.
//!
//! The layout is the wire contract with the web client, byte for byte: profile
//! roots `cv_profile` / `cl_profile` hold one map per language with a `tree`
//! (recursive node maps) and a `format` map. Rules the engine keeps:
//!
//! * every document counts text in UTF-16 units (`OffsetKind::Utf16`), like Yjs;
//! * prose (`bullet`/`sub`/`subheading` `text`, `list-item` `heading`, text
//!   archetype `text`) is `Y.Text` inside a `Y.Map` `data`; other kinds keep a
//!   plain-object `data` that is upgraded lazily on the first prose write;
//! * `selection.active` is a plain array (last writer wins as a whole);
//! * a `Y.Text` is parented to the document before its first insert.
//!
//! A [`Doc::scoped`] document owns one profile root (a sub-document of the
//! workspace) and reports and edits only that root.
//!
//! Known limit: `yrs` decodes plain nested values recursively (`Any::decode`),
//! so a hostile frame of a few hundred KB of nested arrays can exhaust the stack
//! before the engine sees it. Hosts cap the size of a frame and run `receive` on a
//! thread with a stack sized for that cap (`cvws` does the first and documents the
//! second). Everything the engine itself walks is iterative or depth-capped:
//! trees at 128 levels, JSON conversion at 256.
//!
//! Boundary: [`Doc::apply_op`], [`Doc::type_text`], [`Doc::view`],
//! [`Doc::receive`], [`Doc::take_outgoing`]. Mutations return [`Changed`]: the
//! ids a reader must re-read, never a whole tree.
#![allow(
    clippy::too_many_lines,
    clippy::needless_pass_by_value,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

mod convert;
mod cvl;
mod ops;
mod text;
mod track;
mod tree;

use std::collections::BTreeSet;
use std::fmt;
use std::hash::BuildHasher;
use std::ops::Range;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Map, Value};
use yrs::block::ClientID;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{
    GetString, Map as _, OffsetKind, Options, Out, ReadTxn, StateVector, Text, Transact,
    TransactionMut, Update,
};

use ops::{Ctx, Roots};
use track::Tracker;

/// The op catalogue version this engine implements (`OP_CATALOG_VERSION` in `@cv/mutations`).
pub const OP_CATALOG_VERSION: u32 = 10;

/// An op the engine refused. The message is the one `@cv/mutations` throws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpError(String);

impl OpError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    /// An error with a message of the host's own (a host crate refusing an op before the engine sees it).
    #[must_use]
    pub fn from_message(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    /// The human-readable reason.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for OpError {}

/// Who submits an op. Non-human actors cannot edit `FIXED` nodes or apply raw updates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActorClass {
    Human,
    Agent,
    System,
}

impl ActorClass {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Agent => "agent",
            Self::System => "system",
        }
    }

    /// Parse `"human"`, `"agent"` or `"system"`.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "human" => Some(Self::Human),
            "agent" => Some(Self::Agent),
            "system" => Some(Self::System),
            _ => None,
        }
    }
}

/// Where a node lives: the branch (`<profile>.<lang>`) and the id path from the
/// tree root as segments, wrappers included. [`Doc::view_ref`] takes it. Ids repeat
/// across language trees and between siblings of different parents, so an id alone
/// does not say which node changed; and an id may contain a dot, so the path is kept
/// as segments (the dotted form exists for op envelopes only).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub struct NodeRef {
    pub branch: String,
    pub path: Vec<String>,
}

impl NodeRef {
    /// A location from a branch and id segments.
    pub fn new(branch: impl Into<String>, path: Vec<String>) -> Self {
        Self {
            branch: branch.into(),
            path,
        }
    }

    /// The path as the dotted string op envelopes use. Ambiguous when an id contains a dot.
    #[must_use]
    pub fn dotted(&self) -> String {
        self.path.join(".")
    }
}

/// What a mutation touched, sorted: nodes by location, and the
/// `<profile>.<lang>` branches whose `format` map changed. A document reports
/// only the roots it owns.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Changed {
    pub nodes: Vec<NodeRef>,
    pub formats: Vec<String>,
}

impl Changed {
    /// A change report (sorted by the caller or not: it is stored as given).
    #[must_use]
    pub fn new(nodes: Vec<NodeRef>, formats: Vec<String>) -> Self {
        Self { nodes, formats }
    }

    /// True when nothing changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.formats.is_empty()
    }
}

/// What an update frame is made of, read without applying it to a document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct UpdateInfo {
    /// Top-level types named by structs of the frame (`cv_profile`, `jobs`, ...).
    pub roots: BTreeSet<String>,
    /// Clients whose structs the frame inserts or deletes.
    pub clients: BTreeSet<u64>,
    /// Clients of structs the frame refers to without carrying them. `yrs` records
    /// one dependency per stack of blocks it cannot place (the first miss gives up
    /// on the whole stack), so a frame that merges several commits lists one per
    /// commit at best; blocks whose dependencies are present integrate behind a
    /// skip and name nothing. A struct is attributed to the document that already
    /// holds one of these: never treat the list as complete.
    pub depends_on: BTreeSet<u64>,
    /// Clients that created shared types (maps, arrays, texts) which the frame
    /// places under a root that is not a profile root: authors seen to belong to
    /// a foreign sub-document even when the frame also names a profile root.
    pub foreign_authors: BTreeSet<u64>,
    /// True for a frame that carries nothing.
    pub empty: bool,
}

/// Walk the shared types under the roots that are not profile roots and collect
/// the clients that created them (bounded in nodes and depth). Roots a frame
/// creates implicitly are untyped until a document asks for them; every root of
/// the web is a map, so they are read as maps.
fn foreign_branch_clients(probe: &yrs::Doc, roots: &BTreeSet<String>) -> BTreeSet<u64> {
    use yrs::types::SharedRef;
    fn note(id: &yrs::branch::BranchID, out: &mut BTreeSet<u64>) {
        if let yrs::branch::BranchID::Nested(item) = id {
            out.insert(item.client.get());
        }
    }
    let mut out = BTreeSet::new();
    let mut budget = 10_000_usize;
    for name in roots {
        if matches!(name.as_str(), "cv_profile" | "cl_profile") {
            continue;
        }
        let root = probe.get_or_insert_map(name.as_str());
        let txn = probe.transact();
        let mut stack = vec![(Out::YMap(root), 0_usize)];
        while let Some((value, depth)) = stack.pop() {
            if budget == 0 || depth > 64 {
                break;
            }
            budget -= 1;
            match value {
                Out::YMap(m) => {
                    note(m.hook().id(), &mut out);
                    stack.extend(m.iter(&txn).map(|(_, v)| (v, depth + 1)));
                }
                Out::YArray(a) => {
                    note(a.hook().id(), &mut out);
                    stack.extend(yrs::Array::iter(&a, &txn).map(|v| (v, depth + 1)));
                }
                Out::YText(t) => note(t.hook().id(), &mut out),
                _ => {}
            }
        }
    }
    out
}

/// Inspect update bytes: which root names, clients and dependencies they involve.
/// The frame is decoded once and applied to a throw-away empty document; structs
/// whose parents are not in the frame wait there, and what they wait for is
/// `depends_on`.
pub fn inspect_update(bytes: &[u8]) -> Result<UpdateInfo, OpError> {
    let update =
        Update::decode_v1(bytes).map_err(|e| OpError::new(format!("undecodable update: {e}")))?;
    let mut clients = BTreeSet::new();
    for (client, _) in update.insertions(true).iter() {
        clients.insert(client.get());
    }
    for (client, _) in update.delete_set().iter() {
        clients.insert(client.get());
    }
    let empty = update.is_empty();
    let probe = yrs::Doc::new();
    let (roots, depends_on) = {
        let mut txn = probe.transact_mut();
        txn.apply_update(update)
            .map_err(|e| OpError::new(format!("update refused: {e}")))?;
        let roots: BTreeSet<String> = txn.root_refs().map(|(name, _)| name.to_string()).collect();
        let mut depends_on = BTreeSet::new();
        if let Some(pending) = txn.store().pending_update() {
            for (client, _) in pending.missing.iter() {
                depends_on.insert(client.get());
            }
        }
        if let Some(pending) = txn.store().pending_ds() {
            for (client, _) in pending.iter() {
                depends_on.insert(client.get());
            }
        }
        (roots, depends_on)
    };
    let foreign_authors = foreign_branch_clients(&probe, &roots);
    Ok(UpdateInfo {
        roots,
        clients,
        depends_on,
        foreign_authors,
        empty,
    })
}

/// A plain Yrs document that a host keeps only to attribute frames: the state of a
/// sub-document that is not a profile (settings, layout, order state, jobs, notes).
/// It is never an engine document: it reports nothing, produces no bytes, and has
/// no way to be written out. Frames attributed to it are applied to it, so it keeps
/// learning clients and can resolve the parents and left items of later frames.
pub struct Shadow {
    inner: yrs::Doc,
}

impl Default for Shadow {
    fn default() -> Self {
        Self::new()
    }
}

impl Shadow {
    /// An empty shadow.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: yrs::Doc::new(),
        }
    }

    /// Apply a frame or a whole state (update format v1).
    pub fn receive(&mut self, update: &[u8]) -> Result<(), OpError> {
        let update = Update::decode_v1(update)
            .map_err(|e| OpError::new(format!("undecodable update: {e}")))?;
        self.inner
            .transact_mut()
            .apply_update(update)
            .map_err(|e| OpError::new(format!("update refused: {e}")))
    }

    /// The clients this shadow has seen structs of.
    #[must_use]
    pub fn known_clients(&self) -> BTreeSet<u64> {
        self.inner
            .transact()
            .state_vector()
            .iter()
            .map(|(client, _)| client.get())
            .collect()
    }

    /// The top-level types this shadow holds.
    #[must_use]
    pub fn roots(&self) -> BTreeSet<String> {
        self.inner
            .transact()
            .root_refs()
            .map(|(name, _)| name.to_string())
            .collect()
    }
}

/// Outgoing bytes could not be handed over; they stay pending in the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutgoingError(String);

impl OutgoingError {
    /// The human-readable reason.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OutgoingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for OutgoingError {}

const ID_ALPHABET: &[u8; 64] = b"useandom-26T198340PX75pxJACKVERYMINDBUSHWOLF_GQZbfghjklqvwyzrict";

/// Ten characters from the URL-safe alphabet; ids are labels, not secrets.
pub(crate) fn default_id_source() -> impl FnMut() -> String {
    let state = std::collections::hash_map::RandomState::new();
    let mut counter: u64 = 0;
    move || {
        counter += 1;
        let mut bits = state.hash_one(counter);
        (0..10)
            .map(|_| {
                let c = ID_ALPHABET[(bits & 63) as usize] as char;
                bits >>= 6;
                c
            })
            .collect()
    }
}

/// A document with Yjs-compatible text offsets.
pub(crate) fn new_doc(client_id: Option<u64>) -> yrs::Doc {
    let mut options = Options {
        offset_kind: OffsetKind::Utf16,
        ..Options::default()
    };
    if let Some(id) = client_id {
        options.client_id = ClientID::new(id);
    }
    yrs::Doc::with_options(options)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

const REMOTE_ORIGIN: &str = "remote";

/// One Yrs document and its change tracking.
pub struct Doc {
    inner: yrs::Doc,
    roots: Roots,
    scope: Option<&'static str>,
    tracker: Arc<Mutex<Tracker>>,
    outgoing: Arc<Mutex<Vec<Vec<u8>>>>,
    ids: Box<dyn FnMut() -> String + Send>,
}

impl Default for Doc {
    fn default() -> Self {
        Self::new()
    }
}

impl Doc {
    /// An empty document with a random client id.
    #[must_use]
    pub fn new() -> Self {
        Self::build(None, None)
    }

    /// An empty document with a fixed client id (tests and reproducible vectors).
    #[must_use]
    pub fn with_client_id(client_id: u64) -> Self {
        Self::build(Some(client_id), None)
    }

    /// An empty document that owns one profile root, `cv_profile` or `cl_profile`
    /// (a sub-document of the workspace): it observes and reports only that
    /// root, and refuses ops on the other one. `None` for any other name.
    #[must_use]
    pub fn scoped(profile: &str) -> Option<Self> {
        let scope = match profile {
            "cv_profile" => "cv_profile",
            "cl_profile" => "cl_profile",
            _ => return None,
        };
        Some(Self::build(None, Some(scope)))
    }

    /// Replace the generator used when an op omits an id (`node.create`, promotions).
    #[must_use]
    pub fn with_id_source(mut self, source: impl FnMut() -> String + Send + 'static) -> Self {
        self.ids = Box::new(source);
        self
    }

    /// Load a document from a full-state update; the load itself is not reported as a change.
    pub fn load(update: &[u8]) -> Result<Self, OpError> {
        let mut doc = Self::new();
        doc.receive(update)?;
        Ok(doc)
    }

    fn build(client_id: Option<u64>, scope: Option<&'static str>) -> Self {
        let doc = new_doc(client_id);
        let roots = Roots::new(&doc, scope);
        let tracker = Arc::new(Mutex::new(Tracker::default()));
        for (name, root) in [("cv_profile", &roots.cv), ("cl_profile", &roots.cl)] {
            if !roots.owns(name) {
                continue;
            }
            let tracker = Arc::clone(&tracker);
            let observed = root.clone();
            yrs::DeepObservable::observe_deep(root, name, move |txn, events| {
                track::record(name, &observed, txn, events, &mut lock(&tracker));
            });
        }
        let outgoing = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&outgoing);
        // Local transactions only: the bytes a client would send.
        let _subscription = doc.observe_update_v1("engine-out", move |txn, event| {
            if txn
                .origin()
                .is_none_or(|o| o.as_ref() != REMOTE_ORIGIN.as_bytes())
            {
                lock(&sink).push(event.update.clone());
            }
        });
        Self {
            inner: doc,
            roots,
            scope,
            tracker,
            outgoing,
            ids: Box::new(default_id_source()),
        }
    }

    fn run<F>(&mut self, actor: ActorClass, remote: bool, f: F) -> Result<Changed, OpError>
    where
        F: FnOnce(&mut Ctx<'_>, &mut TransactionMut) -> Result<(), OpError>,
    {
        *lock(&self.tracker) = Tracker::default();
        let result = {
            let mut txn = if remote {
                self.inner.transact_mut_with(REMOTE_ORIGIN)
            } else {
                self.inner.transact_mut()
            };
            let mut ctx = Ctx {
                roots: &self.roots,
                actor,
                ids: &mut *self.ids,
            };
            f(&mut ctx, &mut txn)
            // The transaction commits here: observers run and the tracker fills.
        };
        let tracker = std::mem::take(&mut *lock(&self.tracker));
        result.map(|()| Changed {
            nodes: tracker.nodes.into_iter().collect(),
            formats: tracker.formats.into_iter().collect(),
        })
    }

    /// Apply one op as a human actor.
    pub fn apply_op(&mut self, op: &Value) -> Result<Changed, OpError> {
        self.apply_op_as(op, ActorClass::Human)
    }

    /// Apply one op JSON string as a human actor.
    pub fn apply_op_json(&mut self, op: &str) -> Result<Changed, OpError> {
        let value: Value = serde_json::from_str(op)
            .map_err(|e| OpError::new(format!("op is not valid JSON: {e}")))?;
        self.apply_op(&value)
    }

    /// Apply one op; the op shapes are those of the `@cv/mutations` catalogue.
    pub fn apply_op_as(&mut self, op: &Value, actor: ActorClass) -> Result<Changed, OpError> {
        self.run(actor, false, |ctx, txn| ops::apply_op(ctx, txn, op))
    }

    /// Replace `range` (UTF-16 units) of a prose field with `replacement`.
    ///
    /// `node_path` is the dot-joined id path from the tree root, `key` the
    /// prose key of its `data` (`text` or `heading`). A key that still holds a
    /// plain string is upgraded to a `Y.Text` seeded with it. Everything is
    /// checked before the first write: a refused call changes nothing and
    /// produces no bytes. A range may not split a surrogate pair, and a text
    /// with embedded content is refused because its offsets are not text offsets.
    pub fn type_text(
        &mut self,
        branch_path: &str,
        node_path: &str,
        key: &str,
        range: Range<u32>,
        replacement: &str,
        actor: ActorClass,
    ) -> Result<Changed, OpError> {
        let segments: Vec<String> = tree::parse_node_path(node_path)
            .into_iter()
            .map(str::to_string)
            .collect();
        self.type_text_segments(branch_path, &segments, key, range, replacement, actor)
    }

    /// [`Doc::type_text`] at a node named by a [`NodeRef`] (ids may contain dots).
    pub fn type_text_ref(
        &mut self,
        at: &NodeRef,
        key: &str,
        range: Range<u32>,
        replacement: &str,
        actor: ActorClass,
    ) -> Result<Changed, OpError> {
        self.type_text_segments(&at.branch, &at.path, key, range, replacement, actor)
    }

    fn type_text_segments(
        &mut self,
        branch_path: &str,
        segments: &[String],
        key: &str,
        range: Range<u32>,
        replacement: &str,
        actor: ActorClass,
    ) -> Result<Changed, OpError> {
        let node_path = segments.join(".");
        let node_path = node_path.as_str();
        self.run(actor, false, |ctx, txn| {
            let branch = ops::resolve_branch_root(ctx.roots, &*txn, branch_path)?;
            let Some(tree) = tree::get_map(&*txn, &branch, "tree") else {
                return ops::fail(format!("type_text: tree not found at '{branch_path}'"));
            };
            let Some((node, _)) = tree::resolve_segments(&*txn, &tree, segments) else {
                return ops::fail(format!(
                    "type_text: node '{node_path}' not found in '{branch_path}'"
                ));
            };
            ops::tier_protected(&*txn, &node, ctx.actor, "type_text")?;
            let kind = tree::node_kind(&*txn, &node).unwrap_or_default();
            let data_json = node
                .get(&*txn, "data")
                .map(|d| convert::out_to_value(&*txn, &d))
                .transpose()?;
            if !tree::prose_keys(&kind, data_json.as_ref()).contains(&key) {
                return ops::fail(format!(
                    "type_text: '{key}' is not a prose field of node '{node_path}'"
                ));
            }
            let not_text = || {
                OpError::new(format!(
                    "type_text: '{key}' of node '{node_path}' is not text"
                ))
            };
            let existing = match node.get(&*txn, "data") {
                None => None,
                Some(Out::YMap(m)) => m.get(&*txn, key),
                Some(Out::Any(yrs::Any::Map(map))) => map.get(key).cloned().map(Out::Any),
                Some(_) => {
                    return Err(OpError::new(format!(
                        "type_text: data of node '{node_path}' is not an object"
                    )));
                }
            };
            let (current, text_ref) = match existing {
                None => (String::new(), None),
                Some(Out::YText(t)) => {
                    let current = t.get_string(&*txn);
                    let units = u32::try_from(current.encode_utf16().count()).unwrap_or(u32::MAX);
                    if units != t.len(&*txn) {
                        return ops::fail(format!(
                            "type_text: '{key}' of node '{node_path}' holds embedded content"
                        ));
                    }
                    (current, Some(t))
                }
                Some(Out::Any(yrs::Any::String(s))) => (s.to_string(), None),
                Some(_) => return Err(not_text()),
            };
            let units: Vec<u16> = current.encode_utf16().collect();
            let len = u32::try_from(units.len()).unwrap_or(u32::MAX);
            if range.start > range.end || range.end > len {
                return ops::fail(format!(
                    "type_text: range {}..{} is outside the text (length {len})",
                    range.start, range.end
                ));
            }
            let splits = |at: u32| {
                let at = at as usize;
                at > 0
                    && at < units.len()
                    && matches!(units[at - 1], 0xD800..=0xDBFF)
                    && matches!(units[at], 0xDC00..=0xDFFF)
            };
            if splits(range.start) || splits(range.end) {
                return ops::fail("type_text: the range splits a surrogate pair");
            }
            let text = text_ref.unwrap_or_else(|| {
                let data = tree::ensure_data_ymap(txn, &node);
                data.insert(txn, key, yrs::TextPrelim::new(current))
            });
            if range.end > range.start {
                text.remove_range(txn, range.start, range.end - range.start);
            }
            if !replacement.is_empty() {
                text.insert(txn, range.start, replacement);
            }
            Ok(())
        })
    }

    /// The JSON of one node and its subtree, exactly the `Node` shape the web reads.
    pub fn view(&self, branch_path: &str, node_path: &str) -> Result<Value, OpError> {
        let segments: Vec<String> = tree::parse_node_path(node_path)
            .into_iter()
            .map(str::to_string)
            .collect();
        self.view_segments(branch_path, &segments)
    }

    /// [`Doc::view`] at a node named by a [`NodeRef`] (ids may contain dots).
    pub fn view_ref(&self, at: &NodeRef) -> Result<Value, OpError> {
        self.view_segments(&at.branch, &at.path)
    }

    fn view_segments(&self, branch_path: &str, segments: &[String]) -> Result<Value, OpError> {
        let txn = self.inner.transact();
        let tree = self.tree(&txn, branch_path)?;
        let (node, _) = tree::resolve_segments(&txn, &tree, segments).ok_or_else(|| {
            OpError::new(format!(
                "view: node '{}' not found in '{branch_path}'",
                segments.join(".")
            ))
        })?;
        tree::node_json(&txn, &node)
    }

    /// The whole tree of a branch (`<profile>.<lang>`), for cold loads and tests.
    pub fn tree_json(&self, branch_path: &str) -> Result<Value, OpError> {
        let txn = self.inner.transact();
        let tree = self.tree(&txn, branch_path)?;
        tree::node_json(&txn, &tree)
    }

    /// The `format` map of a branch, if any.
    pub fn format_json(&self, branch_path: &str) -> Result<Option<Value>, OpError> {
        let txn = self.inner.transact();
        let lang = self.lang_map(&txn, branch_path)?;
        tree::get_map(&txn, &lang, "format")
            .map(|f| convert::out_to_value(&txn, &Out::YMap(f)))
            .transpose()
    }

    fn lang_map<T: ReadTxn>(&self, txn: &T, branch_path: &str) -> Result<yrs::MapRef, OpError> {
        let mut parts = branch_path.split('.');
        let (Some(profile), Some(lang), None) = (parts.next(), parts.next(), parts.next()) else {
            return Err(OpError::new(format!(
                "branch_path must be '<profile>.<lang>', got '{branch_path}'"
            )));
        };
        let root = self.roots.profile(profile).ok_or_else(|| {
            OpError::new(format!(
                "branch_path must start with cv_profile or cl_profile, got '{profile}'"
            ))
        })?;
        tree::get_map(txn, root, lang)
            .ok_or_else(|| OpError::new(format!("lang '{lang}' not found in {profile}")))
    }

    fn tree<T: ReadTxn>(&self, txn: &T, branch_path: &str) -> Result<yrs::MapRef, OpError> {
        let lang = self.lang_map(txn, branch_path)?;
        tree::get_map(txn, &lang, "tree")
            .ok_or_else(|| OpError::new(format!("tree not found at '{branch_path}'")))
    }

    /// A copy of this document (same content, new client id, no observers
    /// shared). Used to try an update or an op without touching the original.
    pub fn fork(&self) -> Result<Self, OpError> {
        let mut copy = Self::build(None, self.scope);
        copy.receive(&self.encode_state())?;
        Ok(copy)
    }

    /// Check an op against a copy of the document: the changed ids it would
    /// produce, or the reason it would be refused. The document is not changed
    /// and nothing becomes outgoing.
    pub fn dry_run_as(&self, op: &Value, actor: ActorClass) -> Result<Changed, OpError> {
        self.fork()?.apply_op_as(op, actor)
    }

    /// `toJSON` of one profile root (`cv_profile` or `cl_profile`); refuses a tree that is too deep.
    pub fn profile_json(&self, profile: &str) -> Result<Value, OpError> {
        let root = self
            .roots
            .profile(profile)
            .ok_or_else(|| OpError::new(format!("'{profile}' is not cv_profile or cl_profile")))?;
        let txn = self.inner.transact();
        for (_, lang) in root.iter(&txn) {
            if let Out::YMap(lang_map) = lang
                && let Some(tree) = tree::get_map(&txn, &lang_map, "tree")
            {
                tree::ensure_depth(&txn, &tree)?;
            }
        }
        convert::out_to_value(&txn, &Out::YMap(root.clone()))
    }

    /// True when received updates are waiting for missing causal history.
    #[must_use]
    pub fn has_pending(&self) -> bool {
        let txn = self.inner.transact();
        let store = txn.store();
        store.pending_update().is_some() || store.pending_ds().is_some()
    }

    /// A fresh id from this document's id source (for op envelopes).
    pub fn generate_id(&mut self) -> String {
        (self.ids)()
    }

    /// Apply update bytes from a peer (Yjs update format v1).
    pub fn receive(&mut self, update: &[u8]) -> Result<Changed, OpError> {
        let update = Update::decode_v1(update)
            .map_err(|e| OpError::new(format!("receive: undecodable update: {e}")))?;
        self.run(ActorClass::Human, true, |_, txn| {
            txn.apply_update(update)
                .map_err(|e| OpError::new(format!("receive: update refused: {e}")))
        })
    }

    /// The updates produced by local ops since the last call, merged into one.
    /// Empty when nothing is pending. If merging fails the updates stay pending
    /// and the error is returned: outgoing bytes are never lost.
    pub fn take_outgoing(&mut self) -> Result<Vec<u8>, OutgoingError> {
        let pending = std::mem::take(&mut *lock(&self.outgoing));
        match pending.len() {
            0 => Ok(Vec::new()),
            1 => Ok(pending.into_iter().next().unwrap_or_default()),
            _ => match yrs::merge_updates_v1(&pending) {
                Ok(merged) => Ok(merged),
                Err(e) => {
                    *lock(&self.outgoing) = pending;
                    Err(OutgoingError(format!(
                        "outgoing updates could not be merged: {e}"
                    )))
                }
            },
        }
    }

    /// Put bytes taken with [`Doc::take_outgoing`] back, ahead of anything newer,
    /// when the caller could not hand them on.
    pub fn restore_outgoing(&mut self, update: Vec<u8>) {
        if !update.is_empty() {
            lock(&self.outgoing).insert(0, update);
        }
    }

    /// The clients this document has seen structs of (one state-vector walk: take it
    /// once per call, not once per client).
    #[must_use]
    pub fn known_clients(&self) -> BTreeSet<u64> {
        self.inner
            .transact()
            .state_vector()
            .iter()
            .map(|(client, _)| client.get())
            .collect()
    }

    /// True when this document has seen structs of `client`.
    #[must_use]
    pub fn knows_client(&self, client: u64) -> bool {
        self.inner
            .transact()
            .state_vector()
            .contains_client(&ClientID::new(client))
    }

    /// The state vector, for a peer to ask for a diff.
    #[must_use]
    pub fn state_vector(&self) -> Vec<u8> {
        self.inner.transact().state_vector().encode_v1()
    }

    /// The whole document as one update.
    #[must_use]
    pub fn encode_state(&self) -> Vec<u8> {
        self.inner
            .transact()
            .encode_state_as_update_v1(&StateVector::default())
    }

    /// What a peer with state vector `remote` is missing.
    pub fn encode_diff(&self, remote: &[u8]) -> Result<Vec<u8>, OpError> {
        let sv = if remote.is_empty() {
            StateVector::default()
        } else {
            StateVector::decode_v1(remote)
                .map_err(|e| OpError::new(format!("undecodable state vector: {e}")))?
        };
        Ok(self.inner.transact().encode_state_as_update_v1(&sv))
    }

    /// Structural dump of the profile roots (see `convert::dump_shape`):
    /// it tells `Y.Map`, `Y.Array`, `Y.Text` and plain values apart, which
    /// `toJSON` cannot. Empty roots are left out.
    #[must_use]
    pub fn shape(&self) -> Value {
        let txn = self.inner.transact();
        let mut out = Map::new();
        for (name, root) in [
            ("cv_profile", &self.roots.cv),
            ("cl_profile", &self.roots.cl),
        ] {
            if self.roots.owns(name) && root.len(&txn) > 0 {
                out.insert(
                    name.into(),
                    convert::dump_shape(&txn, &Out::YMap(root.clone())),
                );
            }
        }
        Value::Object(out)
    }

    /// Plain text of a prose field (tests and tooling).
    #[must_use]
    pub fn prose(&self, branch_path: &str, node_path: &str, key: &str) -> Option<String> {
        let txn = self.inner.transact();
        let tree = self.tree(&txn, branch_path).ok()?;
        let node = tree::resolve_node_path(&txn, &tree, node_path)?;
        let data = tree::get_map(&txn, &node, "data")?;
        match data.get(&txn, key) {
            Some(Out::YText(t)) => Some(t.get_string(&txn)),
            _ => None,
        }
    }
}
