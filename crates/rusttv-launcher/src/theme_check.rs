//! Checking the package waiting in the staging slot before its review
//! (SECURITY.md "Packages: staging and install" rules 5, 6 and 11). Runs off
//! the UI thread; no UI or platform code.

use std::path::Path;

use rusttv_history::packages::{Package, Store, Verdict};
use rusttv_history::stage::{MAX_STAGED, Moment, Slot, sha256_hex};
use rusttv_history::tv::ThemeInstall;
use rusttv_panels::{Capabilities, Kind, Runtime, Version, read_package};

/// A package that passed the checks, ready for its review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ThemeReview {
    /// What the person approves.
    pub install: ThemeInstall,
    /// Its name in its default language, as the package states it (cleaned
    /// by the validator): shown marked "unverified", never on Home.
    pub name: String,
}

/// How checking ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Checked {
    /// Nothing is waiting (it expired or was replaced meanwhile).
    Nothing,
    /// Ready for the review.
    Review(Box<ThemeReview>),
    /// Refused with this code; the review copy is gone, and so is the slot
    /// when the package itself is broken (rule 5).
    Refused(&'static str),
}

/// The validator panicked on a build that unwinds (on the TV it aborts and
/// the crash marker takes over): the package goes, as after a crash.
pub(crate) fn panicked(root: &Path) -> Checked {
    let slot = Slot::new(root.join("staging"));
    let sha256 = std::fs::read_to_string(root.join("staging/checking")).unwrap_or_default();
    slot.discard(Some(sha256.trim()));
    let _ = slot.set_checking(None);
    Checked::Refused("PACKAGE_INVALID")
}

/// Copies the waiting package for review and checks it at `at`, for a
/// launcher at `launcher`; `builtin` holds the built-in tiles' names,
/// lowercased, that no package may take. The crash marker is set for as
/// long as the validator runs.
pub(crate) fn check(root: &Path, at: Moment, launcher: Version, builtin: &[String]) -> Checked {
    let slot = Slot::new(root.join("staging"));
    let waiting = match slot.copy_for_review(at) {
        Ok(Some(waiting)) => waiting,
        Ok(None) => return Checked::Nothing,
        Err(error) => {
            log::warn!("staging: copying for review failed: {error}");
            return Checked::Refused("STAGING_STORAGE");
        }
    };
    let refuse = |code, broken: bool| {
        slot.discard(broken.then_some(waiting.sha256.as_str()));
        Checked::Refused(code)
    };
    let bytes = match std::fs::read(slot.review_path()) {
        Ok(bytes) if bytes.len() as u64 <= MAX_STAGED && sha256_hex(&bytes) == waiting.sha256 => {
            bytes
        }
        _ => return refuse("STAGING_MISMATCH", false),
    };
    if let Err(error) = slot.set_checking(Some(&waiting.sha256)) {
        log::warn!("staging: setting the crash marker failed: {error}");
        return refuse("STAGING_STORAGE", false);
    }
    let read = read_package(&bytes, &Capabilities::new(launcher, Vec::<String>::new()));
    if let Err(error) = slot.set_checking(None) {
        // The next start would delete this package as if it had crashed.
        log::warn!("staging: clearing the crash marker failed: {error}");
    }
    let contents = match read {
        Ok(contents) => contents,
        Err(error) => return refuse(error.code.as_str(), true),
    };
    let meta = &contents.package.meta;
    if meta.kind != Kind::Theme {
        return refuse("NOT_A_THEME", true);
    }
    if meta.runtime == Runtime::Code {
        // Rule 11: until "Allow packages with code" exists (T5).
        return refuse("PACKAGE_HAS_CODE", false);
    }
    let names: Vec<String> = meta
        .name
        .values()
        .map(|name| name.as_str().to_lowercase())
        .collect();
    if names.iter().any(|name| builtin.contains(name)) {
        return refuse("PACKAGE_NAME_TAKEN", false);
    }
    let package = Package {
        id: meta.id.as_str().to_owned(),
        version: meta.version.to_string(),
        sha256: waiting.sha256.clone(),
        size: waiting.size,
        names,
    };
    let index = match Store::new(root).index() {
        Ok(index) => index,
        Err(error) => {
            log::warn!("package store: {error}");
            return refuse("STAGING_STORAGE", false);
        }
    };
    match index.verdict(&package) {
        Verdict::New | Verdict::Update { .. } => {}
        Verdict::AlreadyInstalled => return refuse("PACKAGE_INSTALLED", false),
        Verdict::Refused(code) => return refuse(code, false),
    }
    let name = meta
        .name
        .get(&meta.default_language)
        .map(|name| name.as_str().to_owned())
        .unwrap_or_default();
    Checked::Review(Box::new(ThemeReview {
        install: ThemeInstall {
            from: index.installed(&package.id).cloned(),
            package,
        },
        name,
    }))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fs;
    use std::path::PathBuf;

    use rusttv_history::packages::Index;

    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("rusttv-theme-check-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("staging")).unwrap();
            Self(dir)
        }

        /// Puts `bytes` in the staging slot, as `stage end` leaves them.
        fn stage(&self, bytes: &[u8]) {
            let staging = self.0.join("staging");
            fs::write(staging.join("slot"), bytes).unwrap();
            let meta = format!("{} {} 7 1000\n", bytes.len(), sha256_hex(bytes));
            fs::write(staging.join("slot.meta"), meta).unwrap();
        }

        fn slot_left(&self) -> bool {
            self.0.join("staging/slot").exists()
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const AT: Moment = Moment { boot: 7, ms: 2_000 };

    pub(crate) fn stored_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        fn crc32(bytes: &[u8]) -> u32 {
            let mut crc = !0_u32;
            for &byte in bytes {
                crc ^= u32::from(byte);
                for _ in 0..8 {
                    crc = if crc & 1 == 1 {
                        (crc >> 1) ^ 0xEDB8_8320
                    } else {
                        crc >> 1
                    };
                }
            }
            !crc
        }
        let mut out = Vec::new();
        let mut directory = Vec::new();
        for (name, bytes) in files {
            let offset = u32::try_from(out.len()).unwrap();
            let head = |sig: u32, central: bool| {
                let mut h = Vec::new();
                h.extend(sig.to_le_bytes());
                if central {
                    h.extend(20_u16.to_le_bytes());
                }
                h.extend(20_u16.to_le_bytes());
                h.extend([0; 2]); // flags
                h.extend([0; 2]); // stored
                h.extend([0; 4]); // time and date
                h.extend(crc32(bytes).to_le_bytes());
                h.extend(u32::try_from(bytes.len()).unwrap().to_le_bytes());
                h.extend(u32::try_from(bytes.len()).unwrap().to_le_bytes());
                h.extend(u16::try_from(name.len()).unwrap().to_le_bytes());
                h.extend([0; 2]);
                if central {
                    h.extend([0; 10]);
                    h.extend(offset.to_le_bytes());
                }
                h.extend(name.as_bytes());
                h
            };
            out.extend(head(0x0403_4b50, false));
            out.extend(*bytes);
            directory.extend(head(0x0201_4b50, true));
        }
        let start = u32::try_from(out.len()).unwrap();
        out.extend(&directory);
        out.extend(0x0605_4b50_u32.to_le_bytes());
        out.extend([0; 4]);
        let n = u16::try_from(files.len()).unwrap();
        out.extend(n.to_le_bytes());
        out.extend(n.to_le_bytes());
        out.extend(u32::try_from(directory.len()).unwrap().to_le_bytes());
        out.extend(start.to_le_bytes());
        out.extend([0; 2]);
        out
    }

    /// A theme package `user.dusk` 1.0.0 named `name`.
    pub(crate) fn theme(name: &str, runtime: &str) -> Vec<u8> {
        theme_with("user.dusk", name, runtime)
    }

    /// A theme package `id` 1.0.0 named `name`.
    pub(crate) fn theme_with(id: &str, name: &str, runtime: &str) -> Vec<u8> {
        let uk = if name == "Dusk" {
            "Сутінки"
        } else {
            name
        };
        let json = format!(
            r#"{{"meta": {{"format": 2, "min_launcher": "0.1.0", "id": "{id}", "kind": "theme",
               "version": "1.0.0", "default_language": "en", "runtime": "{runtime}",
               "name": {{"en": "{name}", "uk": "{uk}"}}}}}}"#
        );
        let mut files: Vec<(&str, &[u8])> = vec![("package.json", json.as_bytes())];
        if runtime == "code" {
            files.push(("code/home.slint", b"export component Home {}\n"));
        }
        stored_zip(&files)
    }

    fn check_in(dir: &TempDir) -> Checked {
        let builtin = vec!["settings".to_owned()];
        check(&dir.0, AT, Version::parse("1.0.0").unwrap(), &builtin)
    }

    #[test]
    fn a_good_theme_is_copied_and_reviewed() {
        let dir = TempDir::new("good");
        let bytes = theme("Dusk", "data");
        dir.stage(&bytes);
        let Checked::Review(review) = check_in(&dir) else {
            panic!("not reviewed");
        };
        assert_eq!(review.name, "Dusk");
        let package = &review.install.package;
        assert_eq!(package.id, "user.dusk");
        assert_eq!(package.version, "1.0.0");
        assert_eq!(package.sha256, sha256_hex(&bytes));
        assert_eq!(package.names, ["dusk", "сутінки"]);
        assert_eq!(review.install.from, None);
        assert_eq!(fs::read(dir.0.join("staging/review")).unwrap(), bytes);
        assert!(!dir.0.join("staging/checking").exists(), "marker cleared");
    }

    #[test]
    fn a_broken_package_is_deleted_and_others_are_only_refused() {
        let dir = TempDir::new("refused");
        dir.stage(b"PK not really a zip");
        assert!(matches!(check_in(&dir), Checked::Refused(_)));
        assert!(!dir.slot_left(), "broken: deleted");
        assert!(!dir.0.join("staging/review").exists());

        dir.stage(&theme("Dusk", "code"));
        assert_eq!(check_in(&dir), Checked::Refused("PACKAGE_HAS_CODE"));
        assert!(dir.slot_left(), "kept until it expires");

        dir.stage(&theme("Settings", "data"));
        assert_eq!(check_in(&dir), Checked::Refused("PACKAGE_NAME_TAKEN"));

        let bytes = theme("Dusk", "data");
        dir.stage(&bytes);
        let Checked::Review(review) = check_in(&dir) else {
            panic!("not reviewed");
        };
        let installed = Index::default().with(review.install.package.clone());
        fs::create_dir_all(dir.0.join("packages")).unwrap();
        fs::write(
            dir.0.join("packages/index.toml"),
            toml::to_string(&installed).unwrap(),
        )
        .unwrap();
        assert_eq!(check_in(&dir), Checked::Refused("PACKAGE_INSTALLED"));
        assert!(!dir.0.join("staging/review").exists());

        fs::remove_file(dir.0.join("staging/slot")).unwrap();
        assert_eq!(check_in(&dir), Checked::Nothing);
    }
}
