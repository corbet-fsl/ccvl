//! Persistence of a [`Workspace`] through any `cvws` [`Store`]: files, `SQLite` or a light server.
//!
//! [`Stored`] is the one place where the engine reads and writes bytes: it cold-loads the profile
//! sub-documents from the store, applies an op, and commits the bytes the op produced as one
//! compare-and-swap write per sub-document. It never interprets what a store keeps beyond the
//! Yjs bytes and clocks the `Store` contract defines.
//!
//! Like the rest of [`Workspace`], every call that touches received bytes must run on a thread
//! whose stack fits the engine's frame cap (see [`crate::doc`]).

use std::collections::BTreeMap;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use cvws::{Commit, Draft, Kind, Op, Origin, Store, StoreError, Yjs};
use serde_json::{Map, Value, json};

use super::{ActorClass, Applied, OpError, Outgoing, SUB_DOCS, SubDoc, Workspace};

/// Record schema of one Yjs sub-document.
pub const SUBDOC_SCHEMA: &str = "careervector.yjs-subdoc/v1";

/// Why a stored session call failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The store refused or could not complete the call.
    Store(StoreError),
    /// The engine refused the op or the bytes.
    Op(OpError),
}

impl Error {
    /// The store holds a newer clock than the commit built on (another writer got in first).
    #[must_use]
    pub fn is_conflict(&self) -> bool {
        matches!(self, Self::Store(StoreError::Conflict { .. }))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(e) => write!(f, "ccvl store: {e}"),
            Self::Op(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<StoreError> for Error {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

impl From<OpError> for Error {
    fn from(e: OpError) -> Self {
        Self::Op(e)
    }
}

/// Who writes: stamped on every committed update.
#[derive(Clone, Debug)]
pub struct Writer {
    pub actor_id: String,
    pub device: String,
}

/// What [`Stored::edit`] came to.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Edit {
    /// The op changed nothing.
    NoChange,
    /// Every sub-document it touched is committed, at these clocks.
    Saved(BTreeMap<SubDoc, u64>),
    /// The store could not be reached: the edit is applied locally and waits in the store's own
    /// durable queue (a remote store's outbox).
    Queued,
}

#[derive(Clone, Copy, Debug, Default)]
struct Position {
    revision: u64,
    clock: u64,
}

/// A workspace and the store it persists to.
pub struct Stored<S: Store> {
    ws: Workspace,
    store: S,
    ws_id: String,
    writer: Writer,
    at: BTreeMap<SubDoc, Position>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn is_offline(e: &StoreError) -> bool {
    matches!(e, StoreError::Io(io) if io.kind() == std::io::ErrorKind::WouldBlock)
}

/// The clock a stored snapshot stands at. A store that keeps a head and a tail (the light server)
/// says so in the record metadata; everywhere else the snapshot stands at the record's clock.
fn snapshot_clock(metadata: &str, yjs: Option<&Yjs>) -> u64 {
    serde_json::from_str::<Value>(metadata)
        .ok()
        .and_then(|m| m.get("headClock").and_then(Value::as_u64))
        .or_else(|| yjs.map(|y| y.clock))
        .unwrap_or(0)
}

impl<S: Store> Stored<S> {
    /// Cold-load the profile sub-documents of workspace `ws_id` from `store`.
    pub fn open(store: S, ws_id: &str, writer: Writer) -> Result<Self, Error> {
        let mut states: Vec<(SubDoc, Vec<u8>)> = Vec::new();
        let mut at = BTreeMap::new();
        let mut tails: Vec<(SubDoc, Vec<Vec<u8>>)> = Vec::new();
        for sub in SUB_DOCS {
            let Some(rec) = store.get_record(ws_id, sub.as_str())? else {
                at.insert(sub, Position::default());
                continue;
            };
            let clock = rec.yjs.as_ref().map_or(0, |y| y.clock);
            at.insert(
                sub,
                Position {
                    revision: rec.revision,
                    clock,
                },
            );
            let head = snapshot_clock(&rec.metadata, rec.yjs.as_ref());
            if clock > head {
                let ops = store.ops_since(ws_id, sub.as_str(), head)?;
                tails.push((sub, ops.into_iter().map(|o| o.bytes).collect()));
            }
            // A light server that never folded has an empty head: everything is in the tail.
            if !rec.bytes.is_empty() {
                states.push((sub, rec.bytes));
            }
        }
        let refs: Vec<(SubDoc, &[u8])> = states.iter().map(|(s, b)| (*s, b.as_slice())).collect();
        let mut ws = Workspace::open(&refs)?;
        for (sub, frames) in tails {
            for frame in frames {
                ws.receive(&frame, Some(sub))?;
            }
        }
        Ok(Self {
            ws,
            store,
            ws_id: ws_id.to_owned(),
            writer,
            at,
        })
    }

    /// The engine document set, for reads.
    #[must_use]
    pub fn workspace(&self) -> &Workspace {
        &self.ws
    }

    /// The engine document set, for hosts that route frames themselves (live updates).
    pub fn workspace_mut(&mut self) -> &mut Workspace {
        &mut self.ws
    }

    /// The store.
    #[must_use]
    pub fn store(&self) -> &S {
        &self.store
    }

    /// The workspace id this session reads and writes.
    #[must_use]
    pub fn ws_id(&self) -> &str {
        &self.ws_id
    }

    /// The clock each sub-document is committed at, as far as this session knows.
    #[must_use]
    pub fn clock(&self, sub: SubDoc) -> u64 {
        self.at.get(&sub).map_or(0, |p| p.clock)
    }

    /// One human action: a single op or a `batch`. The op is checked, applied locally and the
    /// bytes it produced are committed, one write per sub-document touched. If a commit fails the
    /// bytes not yet stored stay in the engine for the next call, and the error is returned.
    pub fn edit(&mut self, op: &Value) -> Result<Edit, Error> {
        let prepared = self.ws.prepare(op, ActorClass::Human)?;
        if prepared.would_change.is_empty() {
            return Ok(Edit::NoChange);
        }
        self.ws.apply(&prepared.envelope, ActorClass::Human)?;
        self.save(&prepared.envelope)
    }

    /// Commit whatever the engine holds as unsaved bytes (after [`Stored::edit`] failed, or after
    /// the host typed through [`Workspace::type_text`] on [`Stored::workspace_mut`]).
    pub fn save(&mut self, envelope: &Value) -> Result<Edit, Error> {
        let mut outgoing = self
            .ws
            .take_outgoing()
            .map_err(|e| OpError::from_message(e.to_string()))?;
        if outgoing.is_empty() {
            return Ok(Edit::NoChange);
        }
        let op_id = envelope
            .get("opId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let many = outgoing.len() > 1;
        let mut saved = BTreeMap::new();
        let mut queued = false;
        while !outgoing.is_empty() {
            let o = outgoing.remove(0);
            let id = op_id.as_ref().map(|id| {
                if many {
                    format!("{id}:{}", o.sub_doc.as_str())
                } else {
                    id.clone()
                }
            });
            match self.commit_one(&o, envelope, id.as_deref()) {
                Ok(Some(clock)) => {
                    saved.insert(o.sub_doc, clock);
                }
                Ok(None) => queued = true,
                Err(e) => {
                    outgoing.insert(0, o);
                    self.ws.restore_outgoing(outgoing);
                    return Err(e.into());
                }
            }
        }
        Ok(if queued {
            Edit::Queued
        } else {
            Edit::Saved(saved)
        })
    }

    fn commit_one(
        &mut self,
        o: &Outgoing,
        envelope: &Value,
        op_id: Option<&str>,
    ) -> Result<Option<u64>, StoreError> {
        let sub = o.sub_doc;
        let pos = self.at.get(&sub).copied().unwrap_or_default();
        let (revision, clock) = (pos.revision + 1, pos.clock + 1);
        let mut meta = Map::new();
        meta.insert("op".into(), envelope.clone());
        if let Some(id) = op_id {
            meta.insert("opId".into(), json!(id));
        }
        let mut draft = Draft::new(
            SUBDOC_SCHEMA,
            sub.as_str(),
            Kind::CrdtSnapshot,
            self.ws.encode_state(sub),
        );
        draft.metadata = meta;
        draft.origin = Some(Origin {
            actor: self.writer.actor_id.clone(),
            device: self.writer.device.clone(),
            parent_revision: pos.revision,
        });
        let at_ms = now_ms();
        let record = draft
            .at(
                revision,
                Some(Yjs {
                    state_vector: self.ws.state_vector(sub),
                    clock,
                }),
            )
            .to_record();
        let commit = Commit {
            record,
            op: Some(Op {
                clock,
                actor_class: "human".into(),
                actor_id: self.writer.actor_id.clone(),
                at_ms,
                bytes: o.update.clone(),
            }),
            at_ms,
        };
        match self.store.commit(&self.ws_id, &commit) {
            Ok(receipt) => {
                self.at.insert(
                    sub,
                    Position {
                        revision: receipt.revision,
                        clock: receipt.clock.unwrap_or(clock),
                    },
                );
                Ok(Some(receipt.clock.unwrap_or(clock)))
            }
            Err(e) if is_offline(&e) => {
                // Queued by the store: the document holds the edit and the next commit builds on it.
                self.at.insert(sub, Position { revision, clock });
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    /// Take what other writers committed since this session last looked, and set the clocks.
    /// The engine merges: bytes this session already holds change nothing.
    pub fn refresh(&mut self) -> Result<Applied, Error> {
        let mut all = Applied::default();
        for sub in SUB_DOCS {
            let key = sub.as_str();
            let known = self.at.get(&sub).copied().unwrap_or_default();
            let Some(rec) = self.store.get_record(&self.ws_id, key)? else {
                continue;
            };
            let clock = rec.yjs.as_ref().map_or(0, |y| y.clock);
            if clock == known.clock && rec.revision == known.revision {
                continue;
            }
            let head = snapshot_clock(&rec.metadata, rec.yjs.as_ref());
            let from = if known.clock < head {
                // Folded into the head: take the head, then its tail.
                if !rec.bytes.is_empty() {
                    merge(&mut all, self.ws.receive(&rec.bytes, Some(sub))?);
                }
                head
            } else {
                known.clock
            };
            for op in self.store.ops_since(&self.ws_id, key, from)? {
                merge(&mut all, self.ws.receive(&op.bytes, Some(sub))?);
            }
            self.at.insert(
                sub,
                Position {
                    revision: rec.revision,
                    clock,
                },
            );
        }
        Ok(all)
    }
}

fn merge(into: &mut Applied, from: Applied) {
    for (sub, changed) in from.changed {
        match into.changed.iter_mut().find(|(s, _)| *s == sub) {
            Some((_, existing)) => {
                existing.nodes.extend(changed.nodes);
                existing.formats.extend(changed.formats);
            }
            None => into.changed.push((sub, changed)),
        }
    }
    into.foreign += from.foreign;
    into.mixed += from.mixed;
    into.dropped += from.dropped;
    into.unrouted = from.unrouted;
    into.failed.extend(from.failed);
    into.refused.extend(from.refused);
    into.resync |= from.resync;
}
