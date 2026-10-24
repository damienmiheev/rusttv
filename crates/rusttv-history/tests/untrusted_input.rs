//! No panics on untrusted input (HISTORY.md § 3): the Android build aborts
//! on panic, so everything that parses door input must return errors.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers should panic on unexpected failures; clippy only exempts #[test] fns"
)]

use std::sync::atomic::{AtomicU32, Ordering};

use proptest::prelude::*;
use rusttv_history::door::{CommitResult, ExportHeader, Message, PendingList};
use rusttv_history::{Author, EntryId, Limits, Meta, State, Store};

fn temp(name: &str) -> std::path::PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "rusttv-fuzz-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// A real export of a small history, to mutate.
fn sample_export() -> (Vec<u8>, EntryId) {
    let dir = temp("source");
    let mut store = Store::open_or_init(&dir).unwrap();
    for n in 0..3 {
        let mut state = State::new();
        state.launcher.order = vec![format!("com.example.a{n}/.Main")];
        store.record(&state, &Meta::new(Author::Tv), n).unwrap();
    }
    let export = store.export(None).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    (export.pack, export.head.unwrap())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn parsers_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        let _ = Message::parse(&bytes);
        let _ = ExportHeader::split(&bytes);
        let text = String::from_utf8_lossy(&bytes);
        let _ = State::parse(&text);
        let _ = CommitResult::parse(&text);
        let _ = PendingList::parse(&text);
        let _ = EntryId::parse(&text);
    }

    #[test]
    fn toml_shaped_input_never_panics(text in "(kind|format|order|settings|packages|approval|change|hash) = (\"[a-z0-9._/-]{0,20}\"|[0-9]{1,3}|\\[\\]|true)\n{1,4}") {
        let _ = Message::parse(text.as_bytes());
        let _ = State::parse(&text);
        let _ = PendingList::parse(&text);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn a_damaged_pack_is_refused_without_panic(flips in proptest::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..8), cut in any::<prop::sample::Index>()) {
        let (mut pack, head) = sample_export();
        for (index, byte) in flips {
            let at = index.index(pack.len());
            pack[at] ^= byte | 1;
        }
        let cut = cut.index(pack.len() + 1);
        pack.truncate(cut);
        let dir = temp("target");
        let mut store = Store::open_or_init(&dir).unwrap();
        let _ = store.import(&pack, &head, Limits::default());
        // Whatever happened, the store is either untouched or a valid copy.
        if let Some(now) = store.head().unwrap() {
            prop_assert_eq!(now, head);
            prop_assert_eq!(store.log(10).unwrap().len(), 3);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
