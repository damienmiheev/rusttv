//! The git store: commits on `main` in a bare repository, packing, and
//! moving history between repositories as packs (HISTORY.md § 3).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use gix::refs::transaction::PreviousValue;
use gix_pack::data::output;

use crate::HistoryError;
use crate::diff::{diff, summary};
use crate::entry::{Author, Entry, EntryId, Meta};
use crate::state::State;

/// The branch every entry is on.
const MAIN: &str = "refs/heads/main";

/// The one file in every entry's tree.
const STATE_FILE: &str = "state.toml";

/// Who the commits say wrote them. Git needs a name; this one is no person.
const COMMITTER: (&str, &str) = ("RustTV", "history@rusttv.invalid");

/// Loose objects that make [`Store::needs_packing`] true.
const PACK_LOOSE_AT: usize = 256;

/// Packs that make [`Store::needs_packing`] true (a full repack).
const REPACK_AT: usize = 8;

/// Size limits for a received pack (HISTORY.md § 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Bytes of one pack.
    pub pack_bytes: usize,
    /// Objects in one pack.
    pub objects: u32,
    /// Bytes of one object, after inflating.
    pub object_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            pack_bytes: 64 << 20,
            objects: 200_000,
            object_bytes: 1 << 20,
        }
    }
}

/// A history's entries after some entry, as a git pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Export {
    /// The newest entry, `None` for an empty history.
    pub head: Option<EntryId>,
    /// A pack of the commits after `since` and everything they need; empty
    /// when there is nothing new.
    pub pack: Vec<u8>,
}

/// What [`Store::pack`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackOutcome {
    /// Objects written to the new pack.
    pub objects: usize,
    /// Size of the new pack file.
    pub pack_bytes: u64,
    /// Whether every object was repacked into one pack.
    pub full: bool,
}

/// A history in a bare git repository.
#[derive(Debug)]
pub struct Store {
    dir: PathBuf,
    repo: gix::Repository,
}

impl Store {
    /// Opens the repository at `dir`, creating an empty one if there is none.
    pub fn open_or_init(dir: &Path) -> Result<Self, HistoryError> {
        let repo = if dir.join("HEAD").is_file() {
            gix::open(dir).map_err(HistoryError::storage)?
        } else {
            gix::init_bare(dir).map_err(HistoryError::storage)?
        };
        Ok(Self {
            dir: dir.to_owned(),
            repo,
        })
    }

    /// Reopens the repository, so objects and packs written meanwhile (or by
    /// another handle) are seen.
    fn reopen(&mut self) -> Result<(), HistoryError> {
        self.repo = gix::open(&self.dir).map_err(HistoryError::storage)?;
        Ok(())
    }

    /// The newest entry, `None` while the history is empty.
    pub fn head(&self) -> Result<Option<EntryId>, HistoryError> {
        match self
            .repo
            .try_find_reference(MAIN)
            .map_err(HistoryError::storage)?
        {
            None => Ok(None),
            Some(mut reference) => {
                let id = reference.peel_to_id().map_err(HistoryError::storage)?;
                Ok(Some(EntryId::from_object(id.detach())))
            }
        }
    }

    /// The newest snapshot, `None` while the history is empty.
    pub fn head_state(&self) -> Result<Option<State>, HistoryError> {
        self.head()?.map(|id| self.state(&id)).transpose()
    }

    /// Records `state` as a new entry at `time_unix` (seconds, the
    /// recording device's clock). An unchanged state is recorded only for
    /// the Mac's notes and imports (they record that a plan or an import
    /// happened); otherwise returns `None`.
    pub fn record(
        &mut self,
        state: &State,
        meta: &Meta,
        time_unix: i64,
    ) -> Result<Option<EntryId>, HistoryError> {
        meta.validate()?;
        let text = state.to_toml()?;
        let parent = self.head()?;
        let old = match &parent {
            Some(id) => self.state(id)?,
            None => State::new(),
        };
        let changes = diff(&old, state);
        let changed = parent.is_none() || !old.same_as(state);
        if !changed && !matches!(meta.author, Author::Mac | Author::Import) {
            return Ok(None);
        }
        let blob = self
            .repo
            .write_blob(text.as_bytes())
            .map_err(HistoryError::storage)?
            .detach();
        let tree = gix::objs::Tree {
            entries: vec![gix::objs::tree::Entry {
                mode: gix::objs::tree::EntryKind::Blob.into(),
                filename: STATE_FILE.into(),
                oid: blob,
            }],
        };
        let tree = self
            .repo
            .write_object(&tree)
            .map_err(HistoryError::storage)?
            .detach();
        let time = format!("{time_unix} +0000");
        let signature = gix::actor::SignatureRef {
            name: COMMITTER.0.into(),
            email: COMMITTER.1.into(),
            time: time.as_str(),
        };
        let parents = parent.map(|id| id.object()).transpose()?;
        let message = meta.message(&summary(&changes));
        let id = self
            .repo
            .commit_as(signature, signature, MAIN, message, tree, parents)
            .map_err(HistoryError::storage)?
            .detach();
        Ok(Some(EntryId::from_object(id)))
    }

    /// Up to `limit` entries, newest first.
    pub fn log(&self, limit: usize) -> Result<Vec<Entry>, HistoryError> {
        let Some(head) = self.head()? else {
            return Ok(Vec::new());
        };
        let mut entries = Vec::new();
        let mut next = Some(head);
        while let Some(id) = next.take() {
            if entries.len() == limit {
                break;
            }
            let entry = self.entry(&id)?;
            next.clone_from(&entry.parent);
            entries.push(entry);
        }
        Ok(entries)
    }

    /// One entry's metadata.
    pub fn entry(&self, id: &EntryId) -> Result<Entry, HistoryError> {
        let commit = self.commit(id)?;
        let parents: Vec<_> = commit.parent_ids().map(|parent| parent.detach()).collect();
        if parents.len() > 1 {
            return Err(HistoryError::invalid(format!("entry {id} is a merge")));
        }
        let message = commit.message_raw().map_err(HistoryError::storage)?;
        let message = std::str::from_utf8(message)
            .map_err(|_| HistoryError::invalid(format!("entry {id}: message is not UTF-8")))?;
        let (summary, meta) = Meta::from_message(message)?;
        let time_unix = commit.time().map_err(HistoryError::storage)?.seconds;
        Ok(Entry {
            id: id.clone(),
            parent: parents.first().map(|parent| EntryId::from_object(*parent)),
            time_unix,
            summary,
            meta,
        })
    }

    /// One entry's snapshot.
    pub fn state(&self, id: &EntryId) -> Result<State, HistoryError> {
        let commit = self.commit(id)?;
        let tree = commit.tree().map_err(HistoryError::storage)?;
        let decoded = tree.decode().map_err(HistoryError::storage)?;
        let [entry] = decoded.entries.as_slice() else {
            return Err(HistoryError::invalid(format!(
                "entry {id}: tree must hold one file"
            )));
        };
        if entry.filename != STATE_FILE || !entry.mode.is_blob() || entry.mode.is_executable() {
            return Err(HistoryError::invalid(format!(
                "entry {id}: tree must hold {STATE_FILE}"
            )));
        }
        let blob = self
            .repo
            .find_object(entry.oid)
            .map_err(HistoryError::storage)?;
        let text = std::str::from_utf8(&blob.data)
            .map_err(|_| HistoryError::invalid(format!("entry {id}: state is not UTF-8")))?;
        State::parse(text)
    }

    fn commit(&self, id: &EntryId) -> Result<gix::Commit<'_>, HistoryError> {
        let object = self
            .repo
            .try_find_object(id.object()?)
            .map_err(HistoryError::storage)?
            .ok_or_else(|| HistoryError::NoSuchEntry(id.to_string()))?;
        object
            .try_into_commit()
            .map_err(|_| HistoryError::NoSuchEntry(id.to_string()))
    }

    /// The commits from `head` back to (not including) `since`, oldest
    /// first. Fails if `since` isn't in the chain.
    fn commits_after(
        &self,
        head: &EntryId,
        since: Option<&EntryId>,
    ) -> Result<Vec<gix::ObjectId>, HistoryError> {
        let mut commits = Vec::new();
        let mut next = Some(head.clone());
        while let Some(id) = next.take() {
            if Some(&id) == since {
                commits.reverse();
                return Ok(commits);
            }
            let commit = self.commit(&id)?;
            let mut parents = commit.parent_ids();
            next = parents
                .next()
                .map(|parent| EntryId::from_object(parent.detach()));
            if parents.next().is_some() {
                return Err(HistoryError::invalid(format!("entry {id} is a merge")));
            }
            commits.push(id.object()?);
        }
        match since {
            None => {
                commits.reverse();
                Ok(commits)
            }
            Some(since) => Err(HistoryError::NotAContinuation(format!(
                "{since} is not in this history"
            ))),
        }
    }

    /// The entries after `since` (all of them for `None`) as a pack.
    pub fn export(&self, since: Option<&EntryId>) -> Result<Export, HistoryError> {
        let head = self.head()?;
        let pack = match &head {
            None if since.is_some() => {
                return Err(HistoryError::NotAContinuation(
                    "this history is empty".to_owned(),
                ));
            }
            None => Vec::new(),
            Some(head) => {
                let commits = self.commits_after(head, since)?;
                if commits.is_empty() {
                    Vec::new()
                } else {
                    self.write_pack(
                        commits,
                        output::count::objects::ObjectExpansion::TreeContents,
                    )?
                }
            }
        };
        Ok(Export { head, pack })
    }

    /// A pack of `ids`, expanded as `expansion` says: whole objects, no deltas.
    fn write_pack(
        &self,
        ids: Vec<gix::ObjectId>,
        expansion: output::count::objects::ObjectExpansion,
    ) -> Result<Vec<u8>, HistoryError> {
        let mut db = gix::odb::at(self.dir.join("objects"), gix::hash::Kind::Sha1)
            .map_err(HistoryError::storage)?
            .into_arc()
            .map_err(HistoryError::storage)?;
        db.prevent_pack_unload();
        let interrupt = AtomicBool::new(false);
        let (counts, _) = output::count::objects_unthreaded(
            &db,
            &mut ids.into_iter().map(Ok),
            &gix::progress::Discard,
            &interrupt,
            expansion,
        )
        .map_err(HistoryError::storage)?;
        let count = u32::try_from(counts.len())
            .map_err(|_| HistoryError::invalid("too many objects to pack".to_owned()))?;
        let entries = output::entry::iter_from_counts(
            counts,
            db,
            Box::new(gix::progress::Discard),
            output::entry::iter_from_counts::Options {
                thread_limit: Some(1),
                ..Default::default()
            },
        )
        .map_err(HistoryError::storage)?;
        let mut bytes = output::bytes::FromEntriesIter::new(
            entries.map(|chunk| chunk.map(|(_, entries)| entries)),
            Vec::new(),
            count,
            gix_pack::data::Version::V2,
            gix::hash::Kind::Sha1,
        );
        for written in bytes.by_ref() {
            written.map_err(HistoryError::storage)?;
        }
        Ok(bytes.into_write())
    }

    /// Adds the entries in `pack` (from [`Store::export`] of another copy)
    /// up to `head`, if they continue this history: either this history is
    /// empty and the pack holds a whole one, or the pack's oldest entry
    /// follows this history's head. Every object is checked before `main`
    /// moves; a refused pack leaves the history as it was.
    pub fn import(
        &mut self,
        pack: &[u8],
        head: &EntryId,
        limits: Limits,
    ) -> Result<(), HistoryError> {
        let current = self.head()?;
        if current.as_ref() == Some(head) {
            return Ok(());
        }
        if pack.len() > limits.pack_bytes {
            return Err(HistoryError::invalid(format!(
                "pack of {} bytes is too big",
                pack.len()
            )));
        }
        let objects = pack_object_count(pack)?;
        if objects > limits.objects {
            return Err(HistoryError::invalid(format!(
                "pack of {objects} objects is too big"
            )));
        }
        let pack_dir = self.dir.join("objects").join("pack");
        fs::create_dir_all(&pack_dir).map_err(HistoryError::storage)?;
        let written = gix_pack::Bundle::write_to_directory(
            &mut &pack[..],
            Some(&pack_dir),
            &mut gix::progress::Discard,
            &AtomicBool::new(false),
            None::<gix::odb::Cache<gix::odb::store::Handle<std::sync::Arc<gix::odb::Store>>>>,
            gix::hash::Kind::Sha1,
            gix_pack::bundle::write::Options {
                thread_limit: Some(1),
                alloc_limit_bytes: Some(limits.object_bytes),
                ..Default::default()
            },
        )
        .map_err(|error| HistoryError::invalid(format!("received pack: {error}")))?;
        let checked = self
            .reopen()
            .and_then(|()| self.check_received(head, current.as_ref(), limits));
        if let Err(error) = checked {
            for path in [written.data_path, written.index_path, written.keep_path]
                .into_iter()
                .flatten()
            {
                let _ = fs::remove_file(path);
            }
            let _ = self.reopen();
            return Err(error);
        }
        if let Some(keep) = &written.keep_path {
            let _ = fs::remove_file(keep);
        }
        let previous = match &current {
            Some(id) => PreviousValue::MustExistAndMatch(gix::refs::Target::Object(id.object()?)),
            None => PreviousValue::MustNotExist,
        };
        self.repo
            .reference(MAIN, head.object()?, previous, "import")
            .map_err(HistoryError::storage)?;
        Ok(())
    }

    /// Checks every entry from `head` back to `current` (or to the first
    /// entry): one parent at most, a valid message, a tree of one valid
    /// `state.toml` within the size limit.
    fn check_received(
        &self,
        head: &EntryId,
        current: Option<&EntryId>,
        limits: Limits,
    ) -> Result<(), HistoryError> {
        let commits = self
            .commits_after(head, current)
            .map_err(|error| match error {
                HistoryError::NotAContinuation(_) | HistoryError::NoSuchEntry(_) => {
                    HistoryError::NotAContinuation(format!(
                        "{head} doesn't follow {}",
                        current.map_or("an empty history", EntryId::as_str)
                    ))
                }
                other => other,
            })?;
        for id in commits {
            let id = EntryId::from_object(id);
            self.entry(&id)?;
            let commit = self.commit(&id)?;
            let tree = commit.tree().map_err(HistoryError::storage)?;
            if let Some(entry) = tree
                .decode()
                .map_err(HistoryError::storage)?
                .entries
                .first()
            {
                let header = self
                    .repo
                    .find_header(entry.oid)
                    .map_err(HistoryError::storage)?;
                if header.size() > limits.object_bytes as u64 {
                    return Err(HistoryError::invalid(format!("entry {id}: state too big")));
                }
            }
            self.state(&id)?;
        }
        Ok(())
    }

    /// Loose objects and packs now.
    fn object_files(&self) -> Result<(Vec<gix::ObjectId>, usize), HistoryError> {
        let objects = self.dir.join("objects");
        let mut loose = Vec::new();
        let mut packs = 0;
        for dir in fs::read_dir(&objects).map_err(HistoryError::storage)? {
            let dir = dir.map_err(HistoryError::storage)?;
            let name = dir.file_name();
            let name = name.to_string_lossy();
            if name == "pack" {
                for file in fs::read_dir(dir.path()).map_err(HistoryError::storage)? {
                    let file = file.map_err(HistoryError::storage)?;
                    if file.path().extension().is_some_and(|ext| ext == "pack") {
                        packs += 1;
                    }
                }
            } else if name.len() == 2 && name.bytes().all(|b| b.is_ascii_hexdigit()) {
                for file in fs::read_dir(dir.path()).map_err(HistoryError::storage)? {
                    let file = file.map_err(HistoryError::storage)?;
                    let hex = format!("{name}{}", file.file_name().to_string_lossy());
                    if let Ok(id) = gix::ObjectId::from_hex(hex.as_bytes()) {
                        loose.push(id);
                    }
                }
            }
        }
        Ok((loose, packs))
    }

    /// Whether [`Store::pack`] has work to do.
    pub fn needs_packing(&self) -> Result<bool, HistoryError> {
        let (loose, packs) = self.object_files()?;
        Ok(loose.len() >= PACK_LOOSE_AT || packs >= REPACK_AT)
    }

    /// Packs the loose objects into one new pack, or, with
    /// [`REPACK_AT`] packs or more, everything into one pack; then removes
    /// what the new pack holds. `None` when there was nothing to do. Safe to
    /// stop at any point: a pack is used only once complete, and objects are
    /// removed only after it is.
    pub fn pack(&mut self) -> Result<Option<PackOutcome>, HistoryError> {
        let (loose, packs) = self.object_files()?;
        let full = packs >= REPACK_AT;
        if !full && loose.len() < PACK_LOOSE_AT {
            return Ok(None);
        }
        let Some(head) = self.head()? else {
            return Ok(None);
        };
        let old_packs: Vec<PathBuf> = if full { self.pack_files()? } else { Vec::new() };
        let bytes = if full {
            let commits = self.commits_after(&head, None)?;
            self.write_pack(
                commits,
                output::count::objects::ObjectExpansion::TreeContents,
            )?
        } else {
            self.write_pack(loose.clone(), output::count::objects::ObjectExpansion::AsIs)?
        };
        let objects = pack_object_count(&bytes)? as usize;
        let pack_dir = self.dir.join("objects").join("pack");
        fs::create_dir_all(&pack_dir).map_err(HistoryError::storage)?;
        let written = gix_pack::Bundle::write_to_directory(
            &mut &bytes[..],
            Some(&pack_dir),
            &mut gix::progress::Discard,
            &AtomicBool::new(false),
            None::<gix::odb::Cache<gix::odb::store::Handle<std::sync::Arc<gix::odb::Store>>>>,
            gix::hash::Kind::Sha1,
            gix_pack::bundle::write::Options {
                thread_limit: Some(1),
                ..Default::default()
            },
        )
        .map_err(HistoryError::storage)?;
        let new_pack = written.data_path.clone();
        if let Some(keep) = &written.keep_path {
            let _ = fs::remove_file(keep);
        }
        // Remove only what the new pack is known to hold.
        let index = written
            .index_path
            .as_deref()
            .map(|path| gix_pack::index::File::at(path, gix::hash::Kind::Sha1))
            .transpose()
            .map_err(HistoryError::storage)?;
        let index = index.ok_or_else(|| HistoryError::storage("pack written without an index"))?;
        for id in &loose {
            if index.lookup(id).is_some() {
                let hex = id.to_hex().to_string();
                let _ = fs::remove_file(self.dir.join("objects").join(&hex[..2]).join(&hex[2..]));
            }
        }
        for old in old_packs
            .iter()
            .filter(|old| Some(*old) != new_pack.as_ref())
        {
            let _ = fs::remove_file(old.with_extension("idx"));
            let _ = fs::remove_file(old);
        }
        self.reopen()?;
        // The history must still read in full from what is left.
        self.commits_after(&head, None)?;
        Ok(Some(PackOutcome {
            objects,
            pack_bytes: bytes.len() as u64,
            full,
        }))
    }

    fn pack_files(&self) -> Result<Vec<PathBuf>, HistoryError> {
        let dir = self.dir.join("objects").join("pack");
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut packs = Vec::new();
        for file in fs::read_dir(dir).map_err(HistoryError::storage)? {
            let path = file.map_err(HistoryError::storage)?.path();
            if path.extension().is_some_and(|ext| ext == "pack") {
                packs.push(path);
            }
        }
        Ok(packs)
    }
}

/// The object count in a pack's header (`PACK`, version, count).
fn pack_object_count(pack: &[u8]) -> Result<u32, HistoryError> {
    match pack {
        [b'P', b'A', b'C', b'K', 0, 0, 0, 2 | 3, a, b, c, d, ..] => {
            Ok(u32::from_be_bytes([*a, *b, *c, *d]))
        }
        _ => Err(HistoryError::invalid("not a git pack".to_owned())),
    }
}

#[cfg(test)]
mod tests;
