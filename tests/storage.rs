//! `Stored` on the file and `SQLite` backends: cold load, commits, conflicts and refresh.
#![cfg(all(feature = "storage", not(target_arch = "wasm32")))]
mod common;

use ccvl::session::SubDoc;
use ccvl::session::storage::{Edit, Stored};
use common::{bullet, seed, set_bullet, writer};
use cvws::Backend;

fn round_trip(open: impl Fn() -> Backend) {
    let mut s = Stored::open(open(), "ws1", writer("a")).expect("open");
    seed(&mut s);
    assert_eq!(s.clock(SubDoc::CvProfile), 1);
    let saved = s.edit(&set_bullet("Shipped things")).expect("edit");
    assert!(
        matches!(saved, Edit::Saved(ref c) if c[&SubDoc::CvProfile] == 2),
        "{saved:?}"
    );
    // The same edit again changes nothing and writes nothing.
    assert_eq!(
        s.edit(&set_bullet("Shipped things")).expect("edit"),
        Edit::NoChange
    );
    assert_eq!(s.clock(SubDoc::CvProfile), 2);
    drop(s);
    let again = Stored::open(open(), "ws1", writer("a")).expect("reopen");
    assert_eq!(bullet(&again), "Shipped things");
    assert_eq!(again.clock(SubDoc::CvProfile), 2);
    assert_eq!(again.clock(SubDoc::ClProfile), 1);
}

#[test]
fn a_file_workspace_survives_a_reopen() {
    let dir = tempfile::tempdir().unwrap();
    round_trip(|| Backend::open_fs(dir.path()).unwrap());
}

#[test]
fn a_sqlite_workspace_survives_a_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ws.db");
    round_trip(|| Backend::open_sqlite(&path).unwrap());
}

#[test]
fn a_second_writer_conflicts_and_merges_after_a_refresh() {
    let dir = tempfile::tempdir().unwrap();
    let open = || Backend::open_fs(dir.path()).unwrap();
    let mut a = Stored::open(open(), "ws1", writer("a")).unwrap();
    seed(&mut a);
    let mut b = Stored::open(open(), "ws1", writer("b")).unwrap();
    a.edit(&set_bullet("From a")).unwrap();
    // b still builds on the clock before a's edit: the store refuses, nothing is lost locally.
    let err = b.edit(&set_bullet("From b")).expect_err("conflict");
    assert!(err.is_conflict(), "{err}");
    assert_eq!(bullet(&b), "From b");
    let applied = b.refresh().unwrap();
    assert!(applied.of(SubDoc::CvProfile).is_some());
    assert_eq!(b.clock(SubDoc::CvProfile), a.clock(SubDoc::CvProfile));
    // The refused bytes ride with the next edit.
    b.edit(&set_bullet("From b, again")).unwrap();
    a.refresh().unwrap();
    assert_eq!(bullet(&a), bullet(&b));
}
