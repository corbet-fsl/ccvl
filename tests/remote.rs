//! `RemoteSession` against the in-process light server of `cvdb`: appends, offline queueing and
//! replay, concurrent writers, folded heads and live frames.
#![cfg(all(feature = "remote", not(target_arch = "wasm32")))]
mod common;

use std::sync::Arc;

use ccvl::session::remote::RemoteSession;
use ccvl::session::storage::Edit;
use ccvl::session::{SubDoc, Workspace};
use common::{CV, seed_remote, set_bullet, writer};
use cvdb::fake::FakeServer;

fn open(server: &FakeServer, dir: &std::path::Path, who: &str) -> RemoteSession {
    RemoteSession::open(
        Arc::new(server.clone()),
        "https://cv.invalid/api",
        "ws1",
        dir,
        writer(who),
    )
    .expect("open")
}

fn bullet(s: &RemoteSession) -> String {
    s.view(SubDoc::CvProfile, CV, "root.item.b1").expect("view")["data"]["text"]
        .as_str()
        .expect("text")
        .to_owned()
}

#[test]
fn edits_are_appended_and_a_new_session_cold_loads_them() {
    let server = FakeServer::new();
    let (d1, d2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut a = open(&server, d1.path(), "a");
    seed_remote(&mut a);
    let saved = a.edit(&set_bullet("Shipped")).unwrap();
    assert!(
        matches!(saved, Edit::Saved(ref c) if c[&SubDoc::CvProfile] == 2),
        "{saved:?}"
    );
    assert_eq!(server.tail("cv-profile").len(), 2);
    let b = open(&server, d2.path(), "b");
    assert_eq!(bullet(&b), "Shipped");
}

#[test]
fn an_offline_edit_queues_and_replays_in_order() {
    let server = FakeServer::new();
    let dir = tempfile::tempdir().unwrap();
    let mut a = open(&server, dir.path(), "a");
    seed_remote(&mut a);
    server.set_offline(true);
    assert_eq!(a.edit(&set_bullet("one")).unwrap(), Edit::Queued);
    assert_eq!(a.edit(&set_bullet("two")).unwrap(), Edit::Queued);
    assert_eq!(a.queued(), 2);
    assert_eq!(bullet(&a), "two");
    server.set_offline(false);
    let flushed = a.flush().unwrap();
    assert_eq!((flushed.acked, flushed.left), (2, 0));
    let d2 = tempfile::tempdir().unwrap();
    assert_eq!(bullet(&open(&server, d2.path(), "b")), "two");
}

#[test]
fn a_queued_edit_survives_a_restart() {
    let server = FakeServer::new();
    let dir = tempfile::tempdir().unwrap();
    {
        let mut a = open(&server, dir.path(), "a");
        seed_remote(&mut a);
        server.set_offline(true);
        assert_eq!(a.edit(&set_bullet("kept")).unwrap(), Edit::Queued);
    }
    server.set_offline(false);
    let mut a = open(&server, dir.path(), "a");
    assert_eq!(a.queued(), 1);
    a.flush().unwrap();
    // The session saw the server before the flush; refresh brings its own edit back.
    a.refresh().unwrap();
    assert_eq!(bullet(&a), "kept");
}

#[test]
fn an_edit_after_reconnecting_goes_out_behind_the_queue() {
    let server = FakeServer::new();
    let dir = tempfile::tempdir().unwrap();
    let mut a = open(&server, dir.path(), "a");
    seed_remote(&mut a);
    server.set_offline(true);
    a.edit(&set_bullet("queued")).unwrap();
    server.set_offline(false);
    let saved = a.edit(&set_bullet("after")).unwrap();
    assert!(matches!(saved, Edit::Saved(_)), "{saved:?}");
    assert_eq!(a.queued(), 0);
    assert_eq!(server.tail("cv-profile").len(), 3);
}

#[test]
fn a_stale_writer_conflicts_then_refreshes_and_merges() {
    let server = FakeServer::new();
    let (d1, d2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut a = open(&server, d1.path(), "a");
    seed_remote(&mut a);
    let mut b = open(&server, d2.path(), "b");
    a.edit(&set_bullet("from a")).unwrap();
    let err = b.edit(&set_bullet("from b")).expect_err("conflict");
    assert!(err.is_conflict(), "{err}");
    let applied = b.refresh().unwrap();
    assert!(applied.of(SubDoc::CvProfile).is_some());
    b.edit(&set_bullet("from b again")).unwrap();
    a.refresh().unwrap();
    assert_eq!(bullet(&a), bullet(&b));
}

#[test]
fn a_head_folded_by_the_server_is_taken_whole() {
    let server = FakeServer::new();
    let (d1, d2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut a = open(&server, d1.path(), "a");
    seed_remote(&mut a);
    let mut b = open(&server, d2.path(), "b");
    a.edit(&set_bullet("one")).unwrap();
    a.edit(&set_bullet("two")).unwrap();
    server.fold("cv-profile", &a.workspace().encode_state(SubDoc::CvProfile));
    b.refresh().unwrap();
    assert_eq!(bullet(&b), "two");
    let d3 = tempfile::tempdir().unwrap();
    assert_eq!(bullet(&open(&server, d3.path(), "c")), "two");
}

#[test]
fn live_frames_apply_without_a_read() {
    let server = FakeServer::new();
    let (d1, d2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut a = open(&server, d1.path(), "a");
    seed_remote(&mut a);
    let mut b = open(&server, d2.path(), "b");
    a.edit(&set_bullet("live")).unwrap();
    let frame = server.tail("cv-profile").pop().expect("update");
    let before = server.requests().len();
    let applied = b.apply_live(&frame).unwrap();
    assert!(applied.of(SubDoc::CvProfile).is_some());
    assert_eq!(bullet(&b), "live");
    assert_eq!(server.requests().len(), before);
}

#[test]
fn a_foreign_sub_document_loads_as_a_routing_shadow() {
    let server = FakeServer::new();
    let dir = tempfile::tempdir().unwrap();
    let mut a = open(&server, dir.path(), "a");
    seed_remote(&mut a);
    a.load_shadows().unwrap();
    let _: &Workspace = a.workspace();
}
