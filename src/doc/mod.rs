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

/// What a mutation touched. `nodes` are node ids (sorted); `formats` are
/// `<profile>.<lang>` branches whose `format` map changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changed {
    pub nodes: Vec<String>,
    pub formats: Vec<String>,
}

impl Changed {
    /// True when nothing changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.formats.is_empty()
    }
}

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
        Self::build(None)
    }

    /// An empty document with a fixed client id (tests and reproducible vectors).
    #[must_use]
    pub fn with_client_id(client_id: u64) -> Self {
        Self::build(Some(client_id))
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

    fn build(client_id: Option<u64>) -> Self {
        let doc = new_doc(client_id);
        let roots = Roots::new(&doc);
        let tracker = Arc::new(Mutex::new(Tracker::default()));
        for (name, root) in [("cv_profile", &roots.cv), ("cl_profile", &roots.cl)] {
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
    /// prose key of its `data` (`text` or `heading`).
    pub fn type_text(
        &mut self,
        branch_path: &str,
        node_path: &str,
        key: &str,
        range: Range<u32>,
        replacement: &str,
    ) -> Result<Changed, OpError> {
        self.run(ActorClass::Human, false, |ctx, txn| {
            let branch = ops::resolve_branch_root(ctx, txn, branch_path)?;
            let Some(tree) = tree::get_map(&*txn, &branch, "tree") else {
                return ops::fail(format!("type_text: tree not found at '{branch_path}'"));
            };
            let Some(node) = tree::resolve_node_path(&*txn, &tree, node_path) else {
                return ops::fail(format!(
                    "type_text: node '{node_path}' not found in '{branch_path}'"
                ));
            };
            let kind = tree::node_kind(&*txn, &node).unwrap_or_default();
            let data_json = node
                .get(&*txn, "data")
                .map(|d| convert::out_to_value(&*txn, &d));
            if !tree::prose_keys(&kind, data_json.as_ref()).contains(&key) {
                return ops::fail(format!(
                    "type_text: '{key}' is not a prose field of node '{node_path}'"
                ));
            }
            let data = tree::ensure_data_ymap(txn, &node);
            let text = match data.get(&*txn, key) {
                Some(Out::YText(t)) => t,
                _ => data.insert(txn, key, yrs::TextPrelim::new("")),
            };
            let len = text.len(&*txn);
            if range.start > range.end || range.end > len {
                return ops::fail(format!(
                    "type_text: range {}..{} is outside the text (length {len})",
                    range.start, range.end
                ));
            }
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
        let txn = self.inner.transact();
        let tree = self.tree(&txn, branch_path)?;
        let node = tree::resolve_node_path(&txn, &tree, node_path).ok_or_else(|| {
            OpError::new(format!(
                "view: node '{node_path}' not found in '{branch_path}'"
            ))
        })?;
        Ok(tree::node_json(&txn, &node))
    }

    /// The whole tree of a branch (`<profile>.<lang>`), for cold loads and tests.
    pub fn tree_json(&self, branch_path: &str) -> Result<Value, OpError> {
        let txn = self.inner.transact();
        let tree = self.tree(&txn, branch_path)?;
        Ok(tree::node_json(&txn, &tree))
    }

    /// The `format` map of a branch, if any.
    pub fn format_json(&self, branch_path: &str) -> Result<Option<Value>, OpError> {
        let txn = self.inner.transact();
        let lang = self.lang_map(&txn, branch_path)?;
        Ok(
            tree::get_map(&txn, &lang, "format")
                .map(|f| convert::out_to_value(&txn, &Out::YMap(f))),
        )
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
        let mut copy = Self::new();
        copy.receive(&self.encode_state())?;
        Ok(copy)
    }

    /// Check an op against a copy of the document: the changed ids it would
    /// produce, or the reason it would be refused. The document is not changed
    /// and nothing becomes outgoing.
    pub fn dry_run_as(&self, op: &Value, actor: ActorClass) -> Result<Changed, OpError> {
        self.fork()?.apply_op_as(op, actor)
    }

    /// `toJSON` of one profile root (`cv_profile` or `cl_profile`).
    pub fn profile_json(&self, profile: &str) -> Result<Value, OpError> {
        let root = self
            .roots
            .profile(profile)
            .ok_or_else(|| OpError::new(format!("'{profile}' is not cv_profile or cl_profile")))?;
        let txn = self.inner.transact();
        Ok(convert::out_to_value(&txn, &Out::YMap(root.clone())))
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
    /// Empty when nothing is pending.
    pub fn take_outgoing(&mut self) -> Vec<u8> {
        let pending = std::mem::take(&mut *lock(&self.outgoing));
        match pending.len() {
            0 => Vec::new(),
            1 => pending.into_iter().next().unwrap_or_default(),
            _ => yrs::merge_updates_v1(&pending).unwrap_or_default(),
        }
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

    /// Structural dump of the profile and jobs roots (see `convert::dump_shape`):
    /// it tells `Y.Map`, `Y.Array`, `Y.Text` and plain values apart, which
    /// `toJSON` cannot. Empty roots are left out.
    #[must_use]
    pub fn shape(&self) -> Value {
        let txn = self.inner.transact();
        let mut out = Map::new();
        for (name, root) in [
            ("cv_profile", &self.roots.cv),
            ("cl_profile", &self.roots.cl),
            ("jobs", &self.roots.jobs),
        ] {
            if root.len(&txn) > 0 {
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
