//! The package store (SECURITY.md "Packages: staging and install" rules 7,
//! 8 and 10): the theme packages a person installed on the TV, under
//! `<files>/packages`. Each package is `<sha256>.zip`; `index.toml` says
//! which are installed and which are kept (an update's previous version).
//! Only [`crate::tv::TvHistory`] writes it, through its journal.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use rusttv_panels::{PackageId, Version};
use serde::{Deserialize, Serialize};

use crate::HistoryError;
use crate::door::is_hash;

/// At most this many packages, installed and kept (PANEL_STUDIO.md § 5.1).
pub const MAX_PACKAGES: usize = 32;

/// At most this many bytes of packages, installed and kept.
pub const MAX_BYTES: u64 = 64 << 20;

/// One package in the store: facts the launcher derived from it when it was
/// reviewed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    /// `user.<name>`.
    pub id: String,
    /// `<major>.<minor>.<patch>`.
    pub version: String,
    /// The zip's SHA-256, lowercase hex.
    pub sha256: String,
    /// The zip's size in bytes.
    pub size: u64,
    /// Its names in every language, lowercased, only to refuse a second
    /// package with the same name (never shown).
    #[serde(default)]
    pub names: Vec<String>,
}

impl Package {
    fn validate(&self) -> Result<(), HistoryError> {
        let bad = |what: &str| HistoryError::invalid(format!("package store: bad {what}"));
        PackageId::parse(&self.id).map_err(|_| bad("id"))?;
        Version::parse(&self.version).map_err(|_| bad("version"))?;
        if !is_hash(&self.sha256) {
            return Err(bad("hash"));
        }
        if self.size == 0 || self.size > crate::stage::MAX_STAGED {
            return Err(bad("size"));
        }
        if self.names.len() > 64 || self.names.iter().any(|name| name.chars().count() > 256) {
            return Err(bad("names"));
        }
        Ok(())
    }

    fn version(&self) -> Option<Version> {
        Version::parse(&self.version).ok()
    }
}

/// What the store holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Index {
    /// Installed, one per id.
    #[serde(default)]
    pub installed: Vec<Package>,
    /// Previous versions kept after an update, one per id.
    #[serde(default)]
    pub kept: Vec<Package>,
}

impl Index {
    /// Checks every entry and that ids are not repeated.
    ///
    /// # Errors
    ///
    /// [`HistoryError::Invalid`] for anything else.
    pub fn validate(&self) -> Result<(), HistoryError> {
        for list in [&self.installed, &self.kept] {
            for (at, package) in list.iter().enumerate() {
                package.validate()?;
                if list[..at].iter().any(|other| other.id == package.id) {
                    return Err(HistoryError::invalid(format!(
                        "package store: {} twice",
                        package.id
                    )));
                }
            }
        }
        if self.installed.len() + self.kept.len() > MAX_PACKAGES || self.bytes() > MAX_BYTES {
            return Err(HistoryError::invalid("package store: over its quota"));
        }
        Ok(())
    }

    /// The installed package with `id`.
    pub fn installed(&self, id: &str) -> Option<&Package> {
        self.installed.iter().find(|package| package.id == id)
    }

    /// Bytes held, installed and kept; a file both name counts once.
    fn bytes(&self) -> u64 {
        let mut seen: Vec<&str> = Vec::new();
        let mut total = 0;
        for package in self.installed.iter().chain(&self.kept) {
            if !seen.contains(&package.sha256.as_str()) {
                seen.push(&package.sha256);
                total += package.size;
            }
        }
        total
    }

    /// Whether `sha256` is named by any entry.
    pub fn holds(&self, sha256: &str) -> bool {
        self.installed
            .iter()
            .chain(&self.kept)
            .any(|package| package.sha256 == sha256)
    }

    /// What installing `new` would do (rule 7).
    pub fn verdict(&self, new: &Package) -> Verdict {
        let Some(version) = new.version() else {
            return Verdict::Refused("PACKAGE_INVALID");
        };
        let taken = self.installed.iter().any(|other| {
            other.id != new.id && other.names.iter().any(|name| new.names.contains(name))
        });
        if taken {
            return Verdict::Refused("PACKAGE_NAME_TAKEN");
        }
        let from = match self.installed(&new.id) {
            None => None,
            Some(old) => match old.version().map(|old| version.cmp(&old)) {
                Some(std::cmp::Ordering::Greater) => Some(old.version.clone()),
                Some(std::cmp::Ordering::Equal) if old.sha256 == new.sha256 => {
                    return Verdict::AlreadyInstalled;
                }
                Some(std::cmp::Ordering::Equal) => return Verdict::Refused("PACKAGE_SAME_VERSION"),
                _ => return Verdict::Refused("PACKAGE_OLDER"),
            },
        };
        let after = self.with(new.clone());
        if after.installed.len() + after.kept.len() > MAX_PACKAGES || after.bytes() > MAX_BYTES {
            return Verdict::Refused("PACKAGE_STORE_FULL");
        }
        match from {
            None => Verdict::New,
            Some(from) => Verdict::Update { from },
        }
    }

    /// The index after installing `new`: an installed version of the same
    /// id becomes the kept one (replacing an older kept one).
    pub fn with(&self, new: Package) -> Self {
        let mut index = self.clone();
        if let Some(at) = index.installed.iter().position(|old| old.id == new.id) {
            let old = index.installed.remove(at);
            index.kept.retain(|kept| kept.id != old.id);
            index.kept.push(old);
        }
        index.installed.push(new);
        index
    }
}

/// What installing a package would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// A package not installed yet.
    New,
    /// A higher version of an installed one.
    Update {
        /// The version installed now.
        from: String,
    },
    /// This exact package is installed: nothing to do.
    AlreadyInstalled,
    /// Refused, with the code the launcher shows.
    Refused(&'static str),
}

/// The store's folder.
#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// The store under the launcher's files dir.
    pub fn new(root: &Path) -> Self {
        Self {
            dir: root.join("packages"),
        }
    }

    /// What it holds; an empty store when there is no index yet.
    ///
    /// # Errors
    ///
    /// A broken index ([`HistoryError::Invalid`]) or storage error.
    pub fn index(&self) -> Result<Index, HistoryError> {
        let index = match fs::read_to_string(self.dir.join("index.toml")) {
            Ok(text) => toml::from_str::<Index>(&text)
                .map_err(|error| HistoryError::invalid(format!("package store: {error}")))?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Index::default(),
            Err(error) => return Err(HistoryError::storage(error)),
        };
        index.validate()?;
        Ok(index)
    }

    /// Replaces the index (a synced temp file, then a rename).
    pub(crate) fn write_index(&self, index: &Index) -> Result<(), HistoryError> {
        index.validate()?;
        let text =
            toml::to_string(index).map_err(|error| HistoryError::invalid(error.to_string()))?;
        fs::create_dir_all(&self.dir).map_err(HistoryError::storage)?;
        write_synced(&self.dir.join("index.toml"), text.as_bytes()).map_err(HistoryError::storage)
    }

    /// Copies `from` into the store as `<sha256>.zip`, then checks the
    /// stored copy's hash (rule 8). A file already there with that hash is
    /// kept as it is.
    pub(crate) fn put(&self, from: &Path, sha256: &str, size: u64) -> Result<(), String> {
        let path = self.file(sha256);
        let matches = |path: &Path| {
            crate::stage::hash_file(path, size)
                .ok()
                .flatten()
                .is_some_and(|(_, hash)| hash == sha256)
        };
        if matches(&path) {
            return Ok(());
        }
        fs::create_dir_all(&self.dir).map_err(|error| error.to_string())?;
        fs::read(from)
            .and_then(|bytes| write_synced(&path, &bytes))
            .map_err(|error| format!("copying the package: {error}"))?;
        if matches(&path) {
            Ok(())
        } else {
            let _ = fs::remove_file(&path);
            Err("the stored copy doesn't match".to_owned())
        }
    }

    /// Deletes package files no entry of `index` names, and stray temp files.
    pub(crate) fn prune(&self, index: &Index) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let keep = name == "index.toml"
                || name
                    .strip_suffix(".zip")
                    .is_some_and(|sha| index.holds(sha));
            if !keep {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    fn file(&self, sha256: &str) -> PathBuf {
        self.dir.join(format!("{sha256}.zip"))
    }
}

/// Writes `bytes` to a temp file beside `path`, syncs it, renames it into
/// place and syncs the folder, so a power cut leaves the old file or the
/// new one.
pub(crate) fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("new");
    let mut file = fs::File::create(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, path)?;
    if let Some(dir) = path.parent()
        && let Ok(dir) = fs::File::open(dir)
    {
        let _ = dir.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(id: &str, version: &str, sha: char, name: &str) -> Package {
        Package {
            id: id.to_owned(),
            version: version.to_owned(),
            sha256: sha.to_string().repeat(64),
            size: 1_000,
            names: vec![name.to_owned()],
        }
    }

    #[test]
    fn verdicts_follow_the_version_and_the_hash() {
        let index = Index::default().with(package("user.dusk", "1.2.0", 'a', "dusk"));
        assert_eq!(
            index.verdict(&package("user.dawn", "1.0.0", 'b', "dawn")),
            Verdict::New
        );
        assert_eq!(
            index.verdict(&package("user.dusk", "1.3.0", 'b', "dusk")),
            Verdict::Update {
                from: "1.2.0".to_owned()
            }
        );
        assert_eq!(
            index.verdict(&package("user.dusk", "1.2.0", 'a', "dusk")),
            Verdict::AlreadyInstalled
        );
        assert_eq!(
            index.verdict(&package("user.dusk", "1.2.0", 'b', "dusk")),
            Verdict::Refused("PACKAGE_SAME_VERSION")
        );
        assert_eq!(
            index.verdict(&package("user.dusk", "1.1.9", 'b', "dusk")),
            Verdict::Refused("PACKAGE_OLDER")
        );
        // Another package's name, under a different id.
        assert_eq!(
            index.verdict(&package("user.other", "1.0.0", 'c', "dusk")),
            Verdict::Refused("PACKAGE_NAME_TAKEN")
        );
    }

    #[test]
    fn an_update_keeps_one_previous_version() {
        let index = Index::default()
            .with(package("user.dusk", "1.0.0", 'a', "dusk"))
            .with(package("user.dusk", "1.1.0", 'b', "dusk"))
            .with(package("user.dusk", "1.2.0", 'c', "dusk"));
        assert_eq!(index.installed("user.dusk").unwrap().version, "1.2.0");
        assert_eq!(index.kept.len(), 1);
        assert_eq!(index.kept[0].version, "1.1.0");
        index.validate().unwrap();
    }

    #[test]
    fn the_store_is_full_by_count_and_by_bytes() {
        let mut index = Index::default();
        for n in 0..MAX_PACKAGES {
            let mut next = package(&format!("user.p{n}"), "1.0.0", 'a', &format!("p{n}"));
            next.sha256 = format!("{n:064x}");
            index = index.with(next);
        }
        let mut one_more = package("user.last", "1.0.0", '0', "last");
        assert_eq!(
            index.verdict(&one_more),
            Verdict::Refused("PACKAGE_STORE_FULL")
        );
        let big = |id: &str, sha: char| Package {
            size: crate::stage::MAX_STAGED,
            ..package(id, "1.0.0", sha, id)
        };
        let mut index = Index::default();
        for (n, sha) in ('a'..='p').enumerate() {
            index = index.with(big(&format!("user.b{n}"), sha));
        }
        one_more.size = 1;
        assert_eq!(
            index.verdict(&one_more),
            Verdict::Refused("PACKAGE_STORE_FULL")
        );
    }

    #[test]
    fn a_broken_index_is_refused() {
        for bad in [
            package("dusk", "1.0.0", 'a', "dusk"),
            package("user.dusk", "1.0", 'a', "dusk"),
            package("user.dusk", "1.0.0", 'G', "dusk"),
            Package {
                size: 0,
                ..package("user.dusk", "1.0.0", 'a', "dusk")
            },
        ] {
            let index = Index {
                installed: vec![bad],
                kept: Vec::new(),
            };
            assert!(index.validate().is_err());
        }
        let twice = Index {
            installed: vec![
                package("user.dusk", "1.0.0", 'a', "dusk"),
                package("user.dusk", "1.1.0", 'b', "dusk"),
            ],
            kept: Vec::new(),
        };
        assert!(twice.validate().is_err());
    }
}
