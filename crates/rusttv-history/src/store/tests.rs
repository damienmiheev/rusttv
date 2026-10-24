use std::sync::atomic::{AtomicU32, Ordering};

use super::*;
use crate::entry::ApprovedBy;
use crate::state::PackageState;

/// A fresh empty directory under the system temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rusttv-history-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn store(&self, name: &str) -> Store {
        Store::open_or_init(&self.0.join(name)).unwrap()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The `n`-th of a series of distinct states.
fn state(n: usize) -> State {
    let mut state = State::new();
    state.launcher.order = (0..20)
        .map(|k| format!("com.example.app{}/.Main", (k + n) % 20))
        .collect();
    state
        .launcher
        .hidden
        .insert(format!("com.example.app{}/.Main", n % 20));
    state
        .settings
        .global
        .insert("animator_duration_scale".into(), format!("0.{}", n % 1000));
    state
}

fn tv() -> Meta {
    Meta::new(Author::Tv)
}

#[test]
fn records_reads_back_and_skips_unchanged_states() {
    let dir = TempDir::new();
    let mut store = dir.store("tv.git");
    assert_eq!(store.head().unwrap(), None);
    assert!(store.log(10).unwrap().is_empty());

    let first = store
        .record(&state(1), &tv(), 1_790_000_000)
        .unwrap()
        .unwrap();
    assert_eq!(store.record(&state(1), &tv(), 1_790_000_001).unwrap(), None);
    let mac = Meta {
        author: Author::Mac,
        journal: Some("1792843149859-optimize.toml".into()),
        approved_by: Some(ApprovedBy::Terminal),
    };
    let note = store
        .record(&state(1), &mac, 1_790_000_002)
        .unwrap()
        .unwrap();
    let second = store
        .record(&state(2), &tv(), 1_790_000_003)
        .unwrap()
        .unwrap();

    let log = store.log(10).unwrap();
    assert_eq!(
        log.iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
        [second.clone(), note.clone(), first.clone()]
    );
    assert_eq!(log[1].meta, mac);
    assert_eq!(log[1].summary, "No change");
    assert_eq!(log[2].parent, None);
    assert_eq!(log[0].time_unix, 1_790_000_003);
    assert_eq!(store.state(&second).unwrap(), state(2));
    assert_eq!(store.log(2).unwrap().len(), 2);
}

#[test]
fn a_whole_history_moves_to_an_empty_copy_and_then_only_what_is_new() {
    let dir = TempDir::new();
    let mut tv_store = dir.store("tv.git");
    let mut mac = dir.store("mac.git");
    for n in 0..5 {
        tv_store
            .record(&state(n), &tv(), 1_790_000_000 + n as i64)
            .unwrap();
    }
    let export = tv_store.export(None).unwrap();
    mac.import(
        &export.pack,
        export.head.as_ref().unwrap(),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(mac.log(100).unwrap(), tv_store.log(100).unwrap());

    for n in 5..8 {
        tv_store
            .record(&state(n), &tv(), 1_790_000_000 + n as i64)
            .unwrap();
    }
    let since = mac.head().unwrap();
    let export = tv_store.export(since.as_ref()).unwrap();
    mac.import(
        &export.pack,
        export.head.as_ref().unwrap(),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(mac.log(100).unwrap(), tv_store.log(100).unwrap());

    // Nothing new: an empty pack, nothing to do.
    let export = tv_store.export(mac.head().unwrap().as_ref()).unwrap();
    assert!(export.pack.is_empty());
    mac.import(
        &export.pack,
        export.head.as_ref().unwrap(),
        Limits::default(),
    )
    .unwrap();
}

#[test]
fn refuses_a_pack_that_does_not_continue_the_history() {
    let dir = TempDir::new();
    let mut a = dir.store("a.git");
    let mut b = dir.store("b.git");
    a.record(&state(1), &tv(), 1).unwrap();
    b.record(&state(2), &tv(), 2).unwrap();
    let before = b.head().unwrap();
    let export = a.export(None).unwrap();
    let error = b
        .import(
            &export.pack,
            export.head.as_ref().unwrap(),
            Limits::default(),
        )
        .unwrap_err();
    assert!(
        matches!(error, HistoryError::NotAContinuation(_)),
        "{error}"
    );
    assert_eq!(b.head().unwrap(), before);
    // The refused pack's files were removed again.
    assert!(b.pack_files().unwrap().is_empty());
}

#[test]
fn refuses_a_damaged_pack_and_one_over_the_limits() {
    let dir = TempDir::new();
    let mut tv_store = dir.store("tv.git");
    for n in 0..3 {
        tv_store.record(&state(n), &tv(), n as i64).unwrap();
    }
    let export = tv_store.export(None).unwrap();
    let head = export.head.unwrap();

    let mut damaged = export.pack.clone();
    let middle = damaged.len() / 2;
    damaged[middle] ^= 0xff;
    let mut mac = dir.store("mac.git");
    assert!(mac.import(&damaged, &head, Limits::default()).is_err());
    assert_eq!(mac.head().unwrap(), None);

    let tight = Limits {
        objects: 2,
        ..Limits::default()
    };
    assert!(matches!(
        mac.import(&export.pack, &head, tight),
        Err(HistoryError::Invalid(_))
    ));
    assert!(mac.import(b"not a pack", &head, Limits::default()).is_err());
    assert_eq!(mac.head().unwrap(), None);
}

#[test]
fn refuses_entries_whose_tree_is_not_one_state_file() {
    let dir = TempDir::new();
    // A history written by hand with an extra file next to state.toml.
    let forged = gix::init_bare(dir.0.join("forged.git")).unwrap();
    let text = state(1).to_toml().unwrap();
    let blob = forged.write_blob(text.as_bytes()).unwrap().detach();
    let tree = gix::objs::Tree {
        entries: vec![
            gix::objs::tree::Entry {
                mode: gix::objs::tree::EntryKind::Blob.into(),
                filename: "evil.sh".into(),
                oid: blob,
            },
            gix::objs::tree::Entry {
                mode: gix::objs::tree::EntryKind::Blob.into(),
                filename: STATE_FILE.into(),
                oid: blob,
            },
        ],
    };
    let tree = forged.write_object(&tree).unwrap().detach();
    let signature = gix::actor::SignatureRef {
        name: "x".into(),
        email: "x@x".into(),
        time: "1 +0000",
    };
    let commit = forged
        .commit_as(
            signature,
            signature,
            MAIN,
            Meta::new(Author::Tv).message("x"),
            tree,
            None::<gix::ObjectId>,
        )
        .unwrap()
        .detach();
    let forged = Store::open_or_init(&dir.0.join("forged.git")).unwrap();
    let export = forged.export(None).unwrap();
    let mut mac = dir.store("mac.git");
    let error = mac
        .import(
            &export.pack,
            &EntryId::from_object(commit),
            Limits::default(),
        )
        .unwrap_err();
    assert!(matches!(error, HistoryError::Invalid(_)), "{error}");
    assert_eq!(mac.head().unwrap(), None);
}

#[test]
fn packs_loose_objects_and_then_everything_into_one_pack() {
    let dir = TempDir::new();
    let mut store = dir.store("tv.git");
    let mut n = 0;
    fn record_until_packing(store: &mut Store, n: &mut usize) {
        while !store.needs_packing().unwrap() {
            store.record(&state(*n), &tv(), *n as i64).unwrap();
            *n += 1;
        }
    }
    record_until_packing(&mut store, &mut n);
    let outcome = store.pack().unwrap().unwrap();
    assert!(!outcome.full);
    assert!(outcome.objects >= PACK_LOOSE_AT);
    let (loose, packs) = store.object_files().unwrap();
    assert!(loose.is_empty(), "{} loose objects left", loose.len());
    assert_eq!(packs, 1);
    let entries = store.log(usize::MAX).unwrap().len();
    assert_eq!(entries, n);

    // Seven more packs make the next pack a full repack into one.
    for _ in 0..7 {
        record_until_packing(&mut store, &mut n);
        assert!(!store.pack().unwrap().unwrap().full);
    }
    record_until_packing(&mut store, &mut n);
    let outcome = store.pack().unwrap().unwrap();
    assert!(outcome.full);
    assert_eq!(store.object_files().unwrap(), (Vec::new(), 1));
    assert_eq!(store.log(usize::MAX).unwrap().len(), n);
    let head = store.head().unwrap().unwrap();
    assert_eq!(store.state(&head).unwrap(), state(n - 1));

    // A packed history still exports and imports.
    let export = store.export(None).unwrap();
    let mut mac = dir.store("mac.git");
    mac.import(&export.pack, &head, Limits::default()).unwrap();
    assert_eq!(mac.log(usize::MAX).unwrap().len(), n);
}

#[test]
fn packages_and_device_facts_are_kept() {
    let dir = TempDir::new();
    let mut store = dir.store("tv.git");
    let mut observed = state(1);
    observed
        .packages
        .insert("com.example.telemetry".into(), PackageState::Disabled);
    observed.device.fingerprint = Some("Philips/x:14/y:user/release-keys".into());
    let id = store
        .record(&observed, &Meta::new(Author::Drift), 5)
        .unwrap()
        .unwrap();
    assert_eq!(store.state(&id).unwrap(), observed);
    assert_eq!(store.entry(&id).unwrap().meta.author, Author::Drift);
}
