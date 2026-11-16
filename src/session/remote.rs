//! The server-bound session: a [`Stored`] workspace on `cvws::RemoteStore`, the byte transport
//! to a `CareerVector` light server.
//!
//! Edits are applied locally, then appended to the server as the bytes they produced (human
//! actor class only: the server refuses agent bytes). A write made while the server is
//! unreachable stays in the transport's durable outbox and is replayed in order by
//! [`RemoteSession::flush`]. What other writers committed comes back through
//! [`RemoteSession::refresh`] or, from a live connection, [`RemoteSession::apply_live`].
//!
//! The server-applied write path (`/ops`, `writePath: legacy`) is not carried: the transport
//! speaks the append path only, and a server that is not on it answers a refused append.

use std::path::Path;
use std::sync::Arc;

use cvdb::Http;
use cvws::RemoteStore;
use serde_json::Value;

use super::storage::{Edit, Error, Stored, Writer};
use super::{Applied, ForeignSub, SubDoc, Workspace};

pub use cvdb::remote::Flushed;

/// The non-profile sub-documents, loaded once as routing shadows so that foreign relay frames
/// are attributed exactly instead of being held and evicted into resyncs.
const FOREIGN: [ForeignSub; 5] = [
    ForeignSub::Jobs,
    ForeignSub::Settings,
    ForeignSub::Layout,
    ForeignSub::OrderState,
    ForeignSub::Notes,
];

/// A workspace on a light server.
pub struct RemoteSession {
    inner: Stored<RemoteStore>,
}

impl RemoteSession {
    /// Read the profile sub-documents from the server. `dir` holds the durable outbox.
    pub fn open(
        http: Arc<dyn Http>,
        api_base: &str,
        ws_id: &str,
        dir: &Path,
        writer: Writer,
    ) -> Result<Self, Error> {
        let store = RemoteStore::open(http, api_base, ws_id, dir)?;
        Ok(Self {
            inner: Stored::open(store, ws_id, writer)?,
        })
    }

    /// Load the routing shadows of the foreign sub-documents (one read each). A sub-document the
    /// server does not hold is skipped.
    pub fn load_shadows(&mut self) -> Result<(), Error> {
        use cvws::Store;
        for which in FOREIGN {
            let (store, ws_id) = (self.inner.store(), self.inner.ws_id().to_owned());
            let Some(rec) = store.get_record(&ws_id, which.as_str())? else {
                continue;
            };
            let head = rec.yjs.as_ref().map_or(0, |y| y.clock);
            let head_clock = serde_json::from_str::<Value>(&rec.metadata)
                .ok()
                .and_then(|m| m.get("headClock").and_then(Value::as_u64))
                .unwrap_or(head);
            let tail = store.ops_since(&ws_id, which.as_str(), head_clock)?;
            let ws = self.inner.workspace_mut();
            ws.load_shadow(which, &rec.bytes)?;
            for op in tail {
                ws.load_shadow(which, &op.bytes)?;
            }
        }
        Ok(())
    }

    /// One human action (a single op or a `batch`). Call it once per committed field edit, never
    /// per keystroke. Entries still waiting in the outbox are replayed first, so an edit made
    /// after the connection came back goes out behind the older ones.
    pub fn edit(&mut self, op: &Value) -> Result<Edit, Error> {
        if self.inner.store().queued() > 0 {
            self.flush()?;
        }
        self.inner.edit(op)
    }

    /// Replay the outbox in order; stops at the first entry the server cannot take yet.
    pub fn flush(&mut self) -> Result<Flushed, Error> {
        Ok(self.inner.store().flush()?)
    }

    /// Entries still waiting for the server.
    #[must_use]
    pub fn queued(&self) -> usize {
        self.inner.store().queued()
    }

    /// Take what other writers committed since the last call; see [`Stored::refresh`].
    pub fn refresh(&mut self) -> Result<Applied, Error> {
        self.inner.refresh()
    }

    /// Apply bytes a live connection delivered. The frame is routed by what it contains.
    pub fn apply_live(&mut self, update: &[u8]) -> Result<Applied, Error> {
        Ok(self.inner.workspace_mut().receive(update, None)?)
    }

    /// Read one node (never the network).
    pub fn view(&self, sub: SubDoc, branch: &str, node: &str) -> Result<Value, Error> {
        Ok(self.inner.workspace().view(sub, branch, node)?)
    }

    /// The document set, for reads.
    #[must_use]
    pub fn workspace(&self) -> &Workspace {
        self.inner.workspace()
    }

    /// The underlying stored session.
    #[must_use]
    pub fn stored(&self) -> &Stored<RemoteStore> {
        &self.inner
    }
}
