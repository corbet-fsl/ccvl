//! The session API over the Yrs document engine.
//!
//! A `CareerVector` workspace is partitioned into sub-documents; the profile
//! sub-documents (`cv-profile`, `cl-profile`) hold the CV and cover-letter
//! trees the engine edits. `Workspace` keeps one engine document per profile
//! sub-document and gives a host (a TUI, a CLI, a sync backend) one small
//! surface, with no I/O and no knowledge of what the documents say:
//!
//! | call | what it does |
//! |---|---|
//! | [`Workspace::open`] | load sub-documents from stored Yjs state |
//! | [`Workspace::apply`] | route an op (or a batch) to its sub-documents and apply it |
//! | [`Workspace::receive`] | take update bytes from the network or a store |
//! | [`Workspace::take_outgoing`] | the bytes local edits produced, per sub-document |
//! | [`Workspace::view`] | read one node |
//!
//! Two more calls serve hosts that talk to a server which applies ops itself:
//! [`Workspace::prepare`] builds and validates an op envelope without applying
//! it (the server's bytes come back through `receive`, so the edit is never
//! applied twice), and [`Workspace::type_text`] is the keystroke path.
//!
//! Every call returns the nodes that changed, by location (`NodeRef`: branch and
//! id path), never a bare id: ids repeat across language trees. A caller reads
//! only those nodes.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ops::Range;

pub use crate::doc::{ActorClass, Changed, NodeRef, OP_CATALOG_VERSION, OpError, OutgoingError};
use crate::doc::{Doc, Shadow, UpdateInfo, inspect_update};
use serde_json::{Map, Value};

/// A profile sub-document of the workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SubDoc {
    CvProfile,
    ClProfile,
}

/// The two sub-documents the engine edits, in the order the host lists them.
pub const SUB_DOCS: [SubDoc; 2] = [SubDoc::CvProfile, SubDoc::ClProfile];

impl SubDoc {
    /// The sub-document name used in op envelopes and on the wire.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CvProfile => "cv-profile",
            Self::ClProfile => "cl-profile",
        }
    }

    /// The Y.Map root holding the languages of this sub-document.
    #[must_use]
    pub const fn root(self) -> &'static str {
        match self {
            Self::CvProfile => "cv_profile",
            Self::ClProfile => "cl_profile",
        }
    }

    /// Parse `cv-profile` or `cl-profile`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "cv-profile" => Some(Self::CvProfile),
            "cl-profile" => Some(Self::ClProfile),
            _ => None,
        }
    }

    fn from_root(root: &str) -> Option<Self> {
        match root {
            "cv_profile" => Some(Self::CvProfile),
            "cl_profile" => Some(Self::ClProfile),
            _ => None,
        }
    }
}

/// What a call changed, per sub-document in `SUB_DOCS` order; sub-documents that
/// did not change are absent. Counters are for this call (held frames that settled
/// during it included).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Applied {
    pub changed: Vec<(SubDoc, Changed)>,
    /// Frames that belong to no sub-document of ours (a `jobs` or `settings`
    /// update): dropped on purpose.
    pub foreign: usize,
    /// Frames that mix sub-documents (more than one profile root, or a profile
    /// root and a foreign one). A frame cannot be split, so none was applied: the
    /// host must resync.
    pub mixed: usize,
    /// Frames lost: pushed out of the full holding buffer, or refused by their
    /// sub-document. The host must resync.
    pub dropped: usize,
    /// Frames still waiting for a sub-document, after this call.
    pub unrouted: usize,
    /// Per-sub-document failures of this call; what applied elsewhere is in `changed`.
    pub failed: Vec<(SubDoc, OpError)>,
    /// Frames refused before they reached any sub-document (too large, undecodable, a
    /// decoder panic); each one is counted in `dropped` and raised the resync flag.
    pub refused: Vec<OpError>,
    /// True when this call raised the resync flag (see [`Workspace::take_resync`]).
    pub resync: bool,
}

impl Applied {
    /// True when no sub-document changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty()
    }

    /// The change of one sub-document.
    #[must_use]
    pub fn of(&self, sub: SubDoc) -> Option<&Changed> {
        self.changed.iter().find(|(s, _)| *s == sub).map(|(_, c)| c)
    }

    fn merge(&mut self, sub: SubDoc, change: Changed) {
        if change.is_empty() {
            return;
        }
        if let Some((_, existing)) = self.changed.iter_mut().find(|(s, _)| *s == sub) {
            existing.nodes.extend(change.nodes);
            existing.nodes.sort();
            existing.nodes.dedup();
            existing.formats.extend(change.formats);
            existing.formats.sort();
            existing.formats.dedup();
        } else {
            self.changed.push((sub, change));
            self.changed.sort_by_key(|(s, _)| *s);
        }
    }
}

/// An op envelope that was checked against copies of the documents.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Prepared {
    /// The op to send, with an `opId` (kept if the caller set one).
    pub envelope: Value,
    /// The sub-documents it touches.
    pub routes: Vec<SubDoc>,
    /// What applying it would change, per sub-document.
    pub would_change: Applied,
}

/// A prose field of a node: where [`Workspace::type_text`] edits.
#[derive(Clone, Copy, Debug)]
pub struct TextField<'a> {
    pub sub: SubDoc,
    /// `<profile>.<lang>`, for example `cv_profile.en`.
    pub branch_path: &'a str,
    /// Dot-joined id path from the tree root.
    pub node_path: &'a str,
    /// The prose key of the node's `data`: `text` or `heading`.
    pub key: &'a str,
}

/// Update bytes a local edit produced for one sub-document.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Outgoing {
    pub sub_doc: SubDoc,
    pub update: Vec<u8>,
}

impl Outgoing {
    /// Bytes for a sub-document (to hand back with [`Workspace::restore_outgoing`]).
    #[must_use]
    pub fn new(sub_doc: SubDoc, update: Vec<u8>) -> Self {
        Self { sub_doc, update }
    }
}

/// Lifetime counters of a workspace.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Stats {
    pub foreign: usize,
    pub mixed: usize,
    pub dropped: usize,
    pub held: usize,
    /// Held frames attributed again (each costs one classification, no decoding).
    pub reclassified: usize,
    /// Clients currently remembered as foreign.
    pub foreign_clients: usize,
}

/// A non-profile sub-document of the workspace. Its state is only ever kept as a
/// routing shadow (see [`Workspace::load_shadow`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ForeignSub {
    Jobs,
    Settings,
    Layout,
    OrderState,
    Notes,
}

impl ForeignSub {
    /// The sub-document name on the wire.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Jobs => "jobs",
            Self::Settings => "settings",
            Self::Layout => "layout",
            Self::OrderState => "order-state",
            Self::Notes => "notes",
        }
    }

    /// The name of its top-level type.
    #[must_use]
    pub const fn root(self) -> &'static str {
        match self {
            Self::Jobs => "jobs",
            Self::Settings => "settings",
            Self::Layout => "layout",
            Self::OrderState => "order_state",
            Self::Notes => "notes",
        }
    }

    /// Parse a sub-document name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        [
            Self::Jobs,
            Self::Settings,
            Self::Layout,
            Self::OrderState,
            Self::Notes,
        ]
        .into_iter()
        .find(|s| s.as_str() == name)
    }
}

/// The profile sub-documents of one workspace.
///
/// Not kept across a reopen: the frames held for a sub-document, the remembered
/// foreign clients and the outgoing bytes live in memory only. The host persists
/// outgoing bytes (`take_outgoing`, `restore_outgoing`) and reloads the stored
/// state of each sub-document with [`Workspace::open`].
pub struct Workspace {
    docs: BTreeMap<SubDoc, Doc>,
    shadows: BTreeMap<ForeignSub, Shadow>,
    known: Known,
    known_stale: bool,
    /// Set when a document gained a client or the foreign set grew: only then can a
    /// held frame attribute differently.
    known_changed: bool,
    held: VecDeque<Held>,
    held_bytes: usize,
    foreign_now: BTreeSet<u64>,
    foreign_before: BTreeSet<u64>,
    resync: bool,
    stats: Stats,
}

/// The clients each document holds, taken once and kept current.
#[derive(Default, PartialEq, Eq)]
struct Known {
    cv: BTreeSet<u64>,
    cl: BTreeSet<u64>,
    shadows: BTreeMap<ForeignSub, BTreeSet<u64>>,
}

/// Where a client or a root belongs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Place {
    Profile(SubDoc),
    Foreign,
}

/// A frame no sub-document claims yet, with what was read from it once.
struct Held {
    bytes: Vec<u8>,
    info: UpdateInfo,
}

/// Frames larger than this are refused before they are decoded: `yrs` decodes plain
/// nested values recursively, so the size of a frame bounds the stack a hostile one
/// can use. Hosts run `receive` on a thread with a stack sized for it (see README).
/// [`Workspace::open`] and [`Workspace::load_shadow`] take whole stored states and
/// have no such cap.
pub const MAX_FRAME_BYTES: usize = 1 << 20;

/// Frames held for lack of a sub-document; past this the oldest is dropped, counted,
/// and the resync flag is raised.
pub const MAX_HELD: usize = 256;

/// Bytes held for lack of a sub-document, besides the count.
pub const MAX_HELD_BYTES: usize = 8 << 20;

/// Clients remembered as foreign: two generations of half this size, so the memory
/// ages out instead of being cleared.
const MAX_FOREIGN_CLIENTS: usize = 8192;

/// Where a frame goes.
#[derive(Clone, Copy)]
enum Route {
    /// An empty frame.
    Nothing,
    /// A frame for sub-documents that are not profiles.
    Foreign,
    Ours(SubDoc),
    /// A frame of more than one sub-document.
    Mixed,
    /// A frame that cannot be attributed yet.
    Unknown,
}

fn refuse<T>(message: impl Into<String>) -> Result<T, OpError> {
    Err(OpError::from_message(message))
}

/// The sub-document a branch path (`<profile>.<lang>`) or a dotted path starting with a profile belongs to.
fn route_path(path: &str) -> Option<SubDoc> {
    SubDoc::from_root(path.split('.').next().unwrap_or(""))
}

/// The sub-document of one non-batch op.
fn route_op(op: &Value) -> Result<SubDoc, OpError> {
    let kind = op
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("undefined");
    let routed = match kind {
        "cl_profile.replace" => Some(SubDoc::ClProfile),
        "yjs.update" => op
            .get("sub_doc")
            .and_then(Value::as_str)
            .and_then(SubDoc::parse),
        // The web routes every translate to cv-profile (it has no branch_path); the
        // engine of that sub-document refuses what is not its own.
        "cv_profile.replace" | "translate" => Some(SubDoc::CvProfile),
        "node.create"
        | "node.update"
        | "node.delete"
        | "node.move"
        | "selection.update"
        | "cvl.duplicate_as_variant"
        | "cvl.break_out"
        | "cvl.promote_sub"
        | "cvl.demote_bullet" => op
            .get("branch_path")
            .and_then(Value::as_str)
            .and_then(route_path),
        _ => None,
    };
    routed.map_or_else(
        || {
            refuse(format!(
                "ccvl: op '{kind}' does not target a profile sub-document (cv-profile, cl-profile)"
            ))
        },
        Ok,
    )
}

/// An op split by sub-document: a plain op has one route, a batch one per group.
fn plan(op: &Value) -> Result<Vec<(SubDoc, Value)>, OpError> {
    if op.get("kind").and_then(Value::as_str) != Some("batch") {
        return Ok(vec![(route_op(op)?, op.clone())]);
    }
    let Some(sub_ops) = op.get("ops").and_then(Value::as_array) else {
        return refuse("batch: 'ops' must be an array");
    };
    let mut groups: BTreeMap<SubDoc, Vec<Value>> = BTreeMap::new();
    for entry in sub_ops {
        let Some(inner) = entry.get("op") else {
            return refuse("batch: every sub-op needs an 'op'");
        };
        let by_op = route_op(inner)?;
        if let Some(declared) = entry.get("subDoc").and_then(Value::as_str)
            && SubDoc::parse(declared) != Some(by_op)
        {
            return refuse(format!(
                "batch: sub-op '{}' is declared for sub-doc '{declared}' but targets '{}'",
                inner
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("undefined"),
                by_op.as_str()
            ));
        }
        groups.entry(by_op).or_default().push(entry.clone());
    }
    if groups.is_empty() {
        // An empty batch is a no-op; it needs no document.
        return Ok(Vec::new());
    }
    let mut planned = Vec::with_capacity(groups.len());
    for (sub, entries) in groups {
        let mut group = op.as_object().cloned().unwrap_or_default();
        group.insert("ops".into(), Value::Array(entries));
        planned.push((sub, Value::Object(group)));
    }
    Ok(planned)
}

/// `inspect_update` behind a panic guard: the throw-away document it builds is the
/// only state a decoder crash can touch, so a crash is a refused frame, not a dead host.
fn inspect(update: &[u8]) -> Result<UpdateInfo, OpError> {
    std::panic::catch_unwind(|| inspect_update(update)).unwrap_or_else(|_| {
        Err(OpError::from_message(
            "ccvl: the frame crashed the decoder and was refused",
        ))
    })
}

/// Write the ids the engine would generate into the op, batch sub-ops included.
fn stamp_ids(op: &mut Value, ids: &mut dyn FnMut() -> String) {
    fn missing(value: Option<&Value>) -> bool {
        value.is_none_or(Value::is_null)
    }
    match op.get("kind").and_then(Value::as_str) {
        Some("node.create") => {
            if let Some(node) = op.get_mut("node").and_then(Value::as_object_mut)
                && missing(node.get("id"))
            {
                node.insert("id".into(), Value::String(ids()));
            }
        }
        Some("cvl.duplicate_as_variant") => {
            if let Some(map) = op.as_object_mut() {
                if missing(map.get("new_id")) {
                    map.insert("new_id".into(), Value::String(ids()));
                }
                if missing(map.get("new_wrapper_id")) {
                    let wrapper: String = ids().chars().take(8).collect();
                    map.insert(
                        "new_wrapper_id".into(),
                        Value::String(format!("vp-{wrapper}")),
                    );
                }
            }
        }
        Some("batch") => {
            if let Some(sub_ops) = op.get_mut("ops").and_then(Value::as_array_mut) {
                for entry in sub_ops {
                    if let Some(inner) = entry.get_mut("op") {
                        stamp_ids(inner, ids);
                    }
                }
            }
        }
        _ => {}
    }
}

impl Workspace {
    /// Load the sub-documents from stored Yjs state (update format v1).
    /// A sub-document without stored state starts empty.
    pub fn open(states: &[(SubDoc, &[u8])]) -> Result<Self, OpError> {
        let mut docs = BTreeMap::new();
        for sub in SUB_DOCS {
            let mut doc = Doc::scoped(sub.root()).expect("a profile root");
            if let Some((_, bytes)) = states.iter().find(|(s, _)| *s == sub) {
                doc.receive(bytes)?;
            }
            docs.insert(sub, doc);
        }
        let mut ws = Self {
            docs,
            shadows: BTreeMap::new(),
            known: Known::default(),
            known_stale: true,
            known_changed: false,
            held: VecDeque::new(),
            held_bytes: 0,
            foreign_now: BTreeSet::new(),
            foreign_before: BTreeSet::new(),
            resync: false,
            stats: Stats::default(),
        };
        ws.refresh_known();
        ws.known_changed = false;
        Ok(ws)
    }

    fn doc(&self, sub: SubDoc) -> &Doc {
        self.docs.get(&sub).expect("every sub-document is opened")
    }

    fn doc_mut(&mut self, sub: SubDoc) -> &mut Doc {
        self.docs
            .get_mut(&sub)
            .expect("every sub-document is opened")
    }

    /// Apply an op, or a batch, to the sub-documents it targets.
    ///
    /// A batch that spans both sub-documents is checked against copies first, so
    /// it either applies everywhere or nowhere.
    pub fn apply(&mut self, op: &Value, actor: ActorClass) -> Result<Applied, OpError> {
        let planned = plan(op)?;
        if planned.len() > 1 {
            for (sub, group) in &planned {
                self.doc(*sub).dry_run_as(group, actor)?;
            }
        }
        let mut applied = Applied::default();
        for (sub, group) in &planned {
            let changed = self.doc_mut(*sub).apply_op_as(group, actor)?;
            applied.merge(*sub, changed);
        }
        self.known_stale = true;
        Ok(applied)
    }

    /// Build and validate an op envelope without applying it.
    ///
    /// For a server that applies ops itself and fans out its own canonical
    /// bytes: a local optimistic apply would be applied a second time when those
    /// bytes arrive. Send `envelope`, then feed the server's bytes to
    /// [`Workspace::receive`]. Nothing is applied and nothing becomes outgoing.
    pub fn prepare(&mut self, op: &Value, actor: ActorClass) -> Result<Prepared, OpError> {
        // Ids the engine would generate are generated here and written into the
        // envelope, so the server that applies it produces the same ones.
        let mut stamped = op.clone();
        let doc = self.doc_mut(SubDoc::CvProfile);
        stamp_ids(&mut stamped, &mut || doc.generate_id());
        let planned = plan(&stamped)?;
        let mut would_change = Applied::default();
        let mut routes = Vec::new();
        for (sub, group) in &planned {
            let changed = self.doc(*sub).dry_run_as(group, actor)?;
            routes.push(*sub);
            would_change.merge(*sub, changed);
        }
        let mut envelope: Map<String, Value> = stamped.as_object().cloned().unwrap_or_default();
        if envelope
            .get("opId")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            let id = self.doc_mut(SubDoc::CvProfile).generate_id();
            envelope.insert("opId".into(), Value::String(id));
        }
        Ok(Prepared {
            envelope: Value::Object(envelope),
            routes,
            would_change,
        })
    }

    /// Take update bytes from the network or a store.
    ///
    /// With `hint` the frame goes to that sub-document, unless the frame itself says
    /// otherwise. Without it (relay frames carry no sub-document tag) the frame is
    /// attributed by structure, never by what it would change on screen, and never by
    /// forking a document:
    ///
    /// * the roots a frame names, and the clients its structs depend on, are read once
    ///   (the frame is decoded once and applied to an empty throw-away document); `yrs`
    ///   records one dependency per stack of blocks it cannot place, so the list is never
    ///   treated as complete;
    /// * a client that a profile sub-document holds belongs to it (on the web every
    ///   sub-document has its own client id); one that a loaded shadow holds, or that was
    ///   seen in foreign frames, is foreign;
    /// * a frame goes to a sub-document only when every dependency and every author that
    ///   is not new attributes to that one place, and no root says otherwise. A frame of
    ///   two places is **mixed**: it cannot be split, so it is not applied, counted, and
    ///   the resync flag is raised, so a document is never polluted. A frame with a
    ///   dependency nobody holds is **held**;
    /// * a purely foreign frame is applied to the shadow that holds its dependencies (if
    ///   the host loaded one), its authors are remembered as foreign (two generations, so
    ///   the memory ages out), and it is dropped and counted otherwise. A frame that is
    ///   foreign but also has an unknown dependency is held and teaches nothing;
    /// * held frames wait in a bounded buffer ([`MAX_HELD`] frames, [`MAX_HELD_BYTES`]
    ///   bytes), retried when a document gained a client or the foreign memory grew
    ///   (hinted receives too), and pushed out oldest first when it is full: counted,
    ///   flag raised. A held frame that fails is dropped alone.
    ///
    /// A frame that is too large ([`MAX_FRAME_BYTES`]), does not decode, or crashes the
    /// decoder is refused: counted in `dropped`, listed in `Applied::refused`, flag raised;
    /// `receive` still settles what it can. Failures of one sub-document do not hide what
    /// another applied: see `Applied::failed`.
    pub fn receive(&mut self, update: &[u8], hint: Option<SubDoc>) -> Result<Applied, OpError> {
        let mut applied = Applied::default();
        if self.known_stale {
            self.refresh_known();
        }
        if update.len() > MAX_FRAME_BYTES {
            self.refuse(
                OpError::from_message(format!(
                    "ccvl: a frame of {} bytes is larger than the {MAX_FRAME_BYTES} byte limit",
                    update.len()
                )),
                &mut applied,
            );
        } else {
            match inspect(update) {
                Err(error) => self.refuse(error, &mut applied),
                Ok(info) => {
                    let mut route = self.classify(&info);
                    if let (Some(sub), Route::Unknown) = (hint, route) {
                        route = Route::Ours(sub);
                    }
                    self.dispatch(route, update, info, &mut applied);
                }
            }
        }
        self.retry_held(&mut applied);
        applied.unrouted = self.held.len();
        self.stats.held = self.held.len();
        self.stats.foreign_clients = self.foreign_now.len() + self.foreign_before.len();
        Ok(applied)
    }

    /// Load the state of a non-profile sub-document as a routing shadow. A shadow is
    /// used only to attribute frames: it is not an engine document, reports nothing,
    /// produces no bytes, is never persisted and never synced. Frames that attribute
    /// to it are applied to it, so it keeps learning clients and can resolve the
    /// parents and left items of later frames; with the shadows loaded every frame
    /// attributes exactly. Without them frames of those sub-documents are recognised
    /// only by what was seen of them. A whole state has no frame cap.
    pub fn load_shadow(&mut self, which: ForeignSub, state: &[u8]) -> Result<(), OpError> {
        self.shadows.entry(which).or_default().receive(state)?;
        self.known_stale = true;
        Ok(())
    }

    /// Frames waiting for a sub-document.
    #[must_use]
    pub fn unrouted(&self) -> usize {
        self.held.len()
    }

    /// Lifetime counters.
    #[must_use]
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// True once if frames were lost, mixed or refused since the last call: the host
    /// should resync the affected sub-documents. Clears the flag. A resync is a set of
    /// REST reads against a deployed Worker: hosts resync at most once every five
    /// minutes, however often the flag is raised.
    pub fn take_resync(&mut self) -> bool {
        std::mem::take(&mut self.resync)
    }

    fn raise_resync(&mut self, applied: &mut Applied) {
        self.resync = true;
        applied.resync = true;
    }

    fn refuse(&mut self, error: OpError, applied: &mut Applied) {
        applied.refused.push(error);
        applied.dropped += 1;
        self.stats.dropped += 1;
        self.raise_resync(applied);
    }

    /// The clients each document holds: one state-vector walk per document.
    fn refresh_known(&mut self) {
        let fresh = Known {
            cv: self.doc(SubDoc::CvProfile).known_clients(),
            cl: self.doc(SubDoc::ClProfile).known_clients(),
            shadows: self
                .shadows
                .iter()
                .map(|(which, shadow)| (*which, shadow.known_clients()))
                .collect(),
        };
        if fresh != self.known {
            self.known_changed = true;
            self.known = fresh;
        }
        self.known_stale = false;
    }

    fn apply_to(&mut self, sub: SubDoc, update: &[u8], applied: &mut Applied) {
        match self.doc_mut(sub).receive(update) {
            Ok(changed) => applied.merge(sub, changed),
            Err(e) => {
                applied.failed.push((sub, e));
                applied.dropped += 1;
                self.stats.dropped += 1;
                self.raise_resync(applied);
            }
        }
        self.refresh_known();
    }

    /// Remember clients as foreign: two generations that age out.
    fn learn_foreign<'a>(&mut self, clients: impl IntoIterator<Item = &'a u64>) {
        for client in clients {
            if self.foreign_now.contains(client) || self.foreign_before.contains(client) {
                continue;
            }
            self.foreign_now.insert(*client);
            self.known_changed = true;
            if self.foreign_now.len() >= MAX_FOREIGN_CLIENTS / 2 {
                self.foreign_before = std::mem::take(&mut self.foreign_now);
            }
        }
    }

    fn dispatch(&mut self, route: Route, bytes: &[u8], info: UpdateInfo, applied: &mut Applied) {
        match route {
            Route::Nothing => {}
            Route::Foreign => {
                applied.foreign += 1;
                self.stats.foreign += 1;
                self.learn_foreign(info.clients.iter());
                self.apply_to_shadows(bytes, &info);
            }
            Route::Ours(sub) => self.apply_to(sub, bytes, applied),
            Route::Mixed => {
                applied.mixed += 1;
                self.stats.mixed += 1;
                // The authors that created types under a foreign root are foreign, whatever
                // else the frame carries (unless a profile sub-document already holds them).
                let learn: Vec<u64> = info
                    .foreign_authors
                    .iter()
                    .copied()
                    .filter(|c| !self.known.cv.contains(c) && !self.known.cl.contains(c))
                    .collect();
                self.learn_foreign(learn.iter());
                self.raise_resync(applied);
            }
            Route::Unknown => self.hold(bytes, info, applied),
        }
    }

    /// Apply a foreign frame to the shadows that hold what it depends on, or that own its root.
    fn apply_to_shadows(&mut self, bytes: &[u8], info: &UpdateInfo) {
        let mut touched = false;
        for (which, shadow) in &mut self.shadows {
            let holds = self.known.shadows.get(which).is_some_and(|known| {
                info.depends_on
                    .iter()
                    .chain(&info.clients)
                    .any(|c| known.contains(c))
            });
            if (holds || info.roots.contains(which.root())) && shadow.receive(bytes).is_ok() {
                touched = true;
            }
        }
        if touched {
            self.refresh_known();
        }
    }

    fn hold(&mut self, bytes: &[u8], info: UpdateInfo, applied: &mut Applied) {
        if self.held.iter().any(|held| held.bytes == bytes) {
            return;
        }
        while self.held.len() >= MAX_HELD
            || (self.held_bytes + bytes.len() > MAX_HELD_BYTES && !self.held.is_empty())
        {
            if let Some(old) = self.held.pop_front() {
                self.held_bytes = self.held_bytes.saturating_sub(old.bytes.len());
            }
            applied.dropped += 1;
            self.stats.dropped += 1;
            self.raise_resync(applied);
        }
        self.held_bytes += bytes.len();
        self.held.push_back(Held {
            bytes: bytes.to_vec(),
            info,
        });
    }

    /// Attribute held frames again, but only while something changed that could
    /// attribute them differently. One frame that fails is dropped and counted; the
    /// frames behind it are still tried.
    fn retry_held(&mut self, applied: &mut Applied) {
        while self.known_changed && !self.held.is_empty() {
            self.known_changed = false;
            for _ in 0..self.held.len() {
                let Some(frame) = self.held.pop_front() else {
                    break;
                };
                self.held_bytes = self.held_bytes.saturating_sub(frame.bytes.len());
                self.stats.reclassified += 1;
                let route = self.classify(&frame.info);
                if matches!(route, Route::Unknown) {
                    self.held_bytes += frame.bytes.len();
                    self.held.push_back(frame);
                } else {
                    self.dispatch(route, &frame.bytes, frame.info, applied);
                }
            }
        }
        self.known_changed = false;
    }

    /// Where a client belongs, by what the documents hold.
    fn places_of(&self, client: u64) -> BTreeSet<Place> {
        let mut places = BTreeSet::new();
        if self.known.cv.contains(&client) {
            places.insert(Place::Profile(SubDoc::CvProfile));
        }
        if self.known.cl.contains(&client) {
            places.insert(Place::Profile(SubDoc::ClProfile));
        }
        if self.known.shadows.values().any(|k| k.contains(&client))
            || self.foreign_now.contains(&client)
            || self.foreign_before.contains(&client)
        {
            places.insert(Place::Foreign);
        }
        places
    }

    fn classify(&self, info: &UpdateInfo) -> Route {
        if info.empty {
            return Route::Nothing;
        }
        let mut places: BTreeSet<Place> = info
            .roots
            .iter()
            .map(|root| SubDoc::from_root(root).map_or(Place::Foreign, Place::Profile))
            .collect();
        let mut unattributed = false;
        for client in &info.depends_on {
            let found = self.places_of(*client);
            // A frame that depends on its own, unseen author (earlier clocks, deleted
            // items of its own) says nothing about where it belongs.
            let new_author = found.is_empty() && info.clients.contains(client);
            unattributed |= found.is_empty() && !new_author;
            places.extend(found);
        }
        // An author nobody has seen is new: no evidence. One that is known is.
        for client in &info.clients {
            places.extend(self.places_of(*client));
        }
        if places.len() > 1 {
            return Route::Mixed;
        }
        match places.into_iter().next() {
            _ if unattributed => Route::Unknown,
            None => Route::Unknown,
            Some(Place::Profile(sub)) => Route::Ours(sub),
            Some(Place::Foreign) => Route::Foreign,
        }
    }

    /// The update bytes local edits produced since the last call, per sub-document.
    /// If one sub-document cannot hand its bytes over, none are taken.
    pub fn take_outgoing(&mut self) -> Result<Vec<Outgoing>, OutgoingError> {
        let mut taken: Vec<Outgoing> = Vec::new();
        for sub in SUB_DOCS {
            match self.doc_mut(sub).take_outgoing() {
                Ok(update) if update.is_empty() => {}
                Ok(update) => taken.push(Outgoing::new(sub, update)),
                Err(e) => {
                    self.restore_outgoing(taken);
                    return Err(e);
                }
            }
        }
        Ok(taken)
    }

    /// Give bytes taken with [`Workspace::take_outgoing`] back, ahead of anything newer,
    /// when the host could not send or store them.
    pub fn restore_outgoing(&mut self, bytes: Vec<Outgoing>) {
        for given_back in bytes {
            self.doc_mut(given_back.sub_doc)
                .restore_outgoing(given_back.update);
        }
    }

    /// One node and its subtree as JSON; `node_path` is the dot-joined id path from the tree root.
    pub fn view(&self, sub: SubDoc, branch_path: &str, node_path: &str) -> Result<Value, OpError> {
        self.doc(sub).view(branch_path, node_path)
    }

    /// One node by its location, as `Applied` reports it (ids may contain dots).
    pub fn view_ref(&self, at: &NodeRef) -> Result<Value, OpError> {
        let sub = route_path(&at.branch).ok_or_else(|| {
            OpError::from_message(format!(
                "ccvl: '{}' is not a profile branch (cv_profile.<lang>, cl_profile.<lang>)",
                at.branch
            ))
        })?;
        self.doc(sub).view_ref(at)
    }

    /// Replace `range` (UTF-16 units) of a prose field: the keystroke path. A refused
    /// call changes nothing; a non-human actor cannot edit a `FIXED` node.
    pub fn type_text(
        &mut self,
        field: &TextField<'_>,
        range: Range<u32>,
        replacement: &str,
        actor: ActorClass,
    ) -> Result<Changed, OpError> {
        self.known_stale = true;
        self.doc_mut(field.sub).type_text(
            field.branch_path,
            field.node_path,
            field.key,
            range,
            replacement,
            actor,
        )
    }

    /// The whole sub-document as one update, for a store to keep.
    #[must_use]
    pub fn encode_state(&self, sub: SubDoc) -> Vec<u8> {
        self.doc(sub).encode_state()
    }

    /// The state vector of a sub-document, for a store to keep beside its state.
    #[must_use]
    pub fn state_vector(&self, sub: SubDoc) -> Vec<u8> {
        self.doc(sub).state_vector()
    }

    /// True when a sub-document holds updates that wait for missing history.
    #[must_use]
    pub fn has_pending(&self, sub: SubDoc) -> bool {
        self.doc(sub).has_pending()
    }
}

#[cfg(all(feature = "remote", not(target_arch = "wasm32")))]
pub mod remote;
#[cfg(all(feature = "storage", not(target_arch = "wasm32")))]
pub mod storage;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_held_frame_that_fails_is_dropped_and_the_frames_behind_it_still_settle() {
        let tree = json!({ "id": "root", "kind": "cv-root", "tier": "FIXED",
            "data": {}, "selection": { "active": ["b"] }, "children": [
                { "id": "b", "kind": "bullet", "tier": "HYBRID", "data": { "text": "words" },
                  "selection": { "active": [] }, "children": [] }] });
        let mut ws = Workspace::open(&[]).expect("open");
        ws.apply(
            &json!({ "kind": "cv_profile.replace", "lang": "en", "tree": tree }),
            ActorClass::Human,
        )
        .expect("seed");
        let seed = inspect_update(&ws.encode_state(SubDoc::CvProfile)).expect("inspect");
        let known = UpdateInfo {
            depends_on: seed.clients,
            ..UpdateInfo::default()
        };

        let mut peer = Workspace::open(&[(SubDoc::CvProfile, &ws.encode_state(SubDoc::CvProfile))])
            .expect("peer");
        let field = TextField {
            sub: SubDoc::CvProfile,
            branch_path: "cv_profile.en",
            node_path: "root.b",
            key: "text",
        };
        peer.type_text(&field, 0..0, "good ", ActorClass::Human)
            .expect("typed");
        let good = peer.take_outgoing().expect("outgoing").remove(0).update;

        // A frame that cannot be applied sits in front of a good one.
        ws.held.push_back(Held {
            bytes: vec![1, 2, 3],
            info: known.clone(),
        });
        ws.held.push_back(Held {
            bytes: good,
            info: known,
        });
        let mut applied = Applied::default();
        ws.refresh_known();
        ws.known_changed = true;
        ws.retry_held(&mut applied);
        assert_eq!(applied.dropped, 1, "the bad frame is dropped and counted");
        assert_eq!(applied.failed.len(), 1);
        assert!(applied.resync);
        assert!(
            applied.of(SubDoc::CvProfile).is_some(),
            "the frame behind it still applied"
        );
        assert_eq!(ws.unrouted(), 0);
        assert_eq!(
            ws.view(SubDoc::CvProfile, "cv_profile.en", "root.b")
                .expect("view")["data"]["text"],
            "good words"
        );
    }
}
