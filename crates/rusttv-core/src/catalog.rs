//! The package catalog and a person's privacy choices (OPTIMIZER.md
//! § Generic design: catalog, device profiles, your choices).
//!
//! - The catalog (`compat/packages/*.toml`, compiled in) says once what each
//!   package is, its category and what stops working without it.
//! - Device profiles ([`crate::DeviceProfile`]) say which catalog packages
//!   were tested on a model.
//! - The choices file (local, never in the repository) says which categories
//!   to apply, which packages to keep, and which to add.

use std::collections::HashSet;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::command::PackageName;
use crate::compat::PrivacyCategory;
use crate::plan::PlanError;

/// Version of the `compat/packages/*.toml` format this build understands.
pub const CATALOG_SCHEMA_VERSION: u32 = 1;

/// Every file in `compat/packages/`, compiled in. A test fails if one is
/// missing here.
const BUNDLED_CATALOG: &[(&str, &str)] = &[
    (
        "android.toml",
        include_str!("../../../compat/packages/android.toml"),
    ),
    (
        "tp-vision.toml",
        include_str!("../../../compat/packages/tp-vision.toml"),
    ),
    (
        "acr.toml",
        include_str!("../../../compat/packages/acr.toml"),
    ),
    (
        "amazon.toml",
        include_str!("../../../compat/packages/amazon.toml"),
    ),
    (
        "sony.toml",
        include_str!("../../../compat/packages/sony.toml"),
    ),
    (
        "tcl.toml",
        include_str!("../../../compat/packages/tcl.toml"),
    ),
    (
        "mediatek.toml",
        include_str!("../../../compat/packages/mediatek.toml"),
    ),
    (
        "xiaomi.toml",
        include_str!("../../../compat/packages/xiaomi.toml"),
    ),
    (
        "hisense.toml",
        include_str!("../../../compat/packages/hisense.toml"),
    ),
    (
        "skyworth.toml",
        include_str!("../../../compat/packages/skyworth.toml"),
    ),
    (
        "other-vendors.toml",
        include_str!("../../../compat/packages/other-vendors.toml"),
    ),
    (
        "streaming-apps.toml",
        include_str!("../../../compat/packages/streaming-apps.toml"),
    ),
];

/// One package of the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntry {
    /// The package; validated like every package name.
    pub package: PackageName,
    /// What it is for; decides whether it is on by default.
    pub category: PrivacyCategory,
    /// What it does, in plain words.
    pub what: String,
    /// What stops working while it is disabled, in plain words.
    pub side_effects: String,
    /// Where the facts come from (a device it was seen on, another project).
    pub sources: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogFile {
    schema_version: u32,
    packages: Vec<CatalogEntry>,
}

/// The bundled catalog, in file order. A file that can't be read is left
/// out, which only means fewer packages are offered; a test checks every
/// file reads.
pub fn bundled_catalog() -> Vec<CatalogEntry> {
    BUNDLED_CATALOG
        .iter()
        .filter_map(|(_, text)| toml::from_str::<CatalogFile>(text).ok())
        .filter(|file| file.schema_version == CATALOG_SCHEMA_VERSION)
        .flat_map(|file| file.packages)
        .collect()
}

/// A person's privacy choices: `<config dir>/privacy.toml` or `--choices`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivacyChoices {
    /// Catalog categories to apply. Default: telemetry, ads, extras.
    #[serde(default = "default_categories")]
    pub categories: Vec<PrivacyCategory>,
    /// Never disable these, whatever else says so.
    #[serde(default)]
    pub keep: Vec<PackageName>,
    /// Catalog packages to apply although their category isn't chosen.
    #[serde(default)]
    pub also: Vec<PackageName>,
    /// Packages that aren't in the catalog; always untested.
    #[serde(default)]
    pub extra: Vec<PackageName>,
}

fn default_categories() -> Vec<PrivacyCategory> {
    PrivacyCategory::DEFAULT.to_vec()
}

impl Default for PrivacyChoices {
    fn default() -> Self {
        Self {
            categories: default_categories(),
            keep: Vec::new(),
            also: Vec::new(),
            extra: Vec::new(),
        }
    }
}

impl PrivacyChoices {
    /// Checks the choices against the catalog: `also` must name catalog
    /// packages and `extra` packages that aren't in it.
    pub fn validate(&self, catalog: &[CatalogEntry]) -> Result<(), PlanError> {
        let known: HashSet<&str> = catalog.iter().map(|entry| entry.package.as_str()).collect();
        if let Some(package) = self.also.iter().find(|p| !known.contains(p.as_str())) {
            return Err(PlanError::invalid_choices(format!(
                "`also` lists {package}, which isn't in the catalog; list it under `extra` instead"
            )));
        }
        if let Some(package) = self.extra.iter().find(|p| known.contains(p.as_str())) {
            return Err(PlanError::invalid_choices(format!(
                "`extra` lists {package}, which is in the catalog; list it under `also` instead"
            )));
        }
        Ok(())
    }
}

/// Choices and the file they came from (`None`: no file, the defaults).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoadedChoices {
    /// The file read, if any.
    pub path: Option<PathBuf>,
    /// What it says.
    pub choices: PrivacyChoices,
}

/// RustTV's config directory: `RUSTTV_CONFIG_DIR` if set, else
/// `~/Library/Application Support/rusttv` on macOS and
/// `$XDG_CONFIG_HOME/rusttv` (default `~/.config/rusttv`) elsewhere.
pub fn config_dir() -> Result<PathBuf, PlanError> {
    if let Some(dir) = env::var_os("RUSTTV_CONFIG_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
        .ok_or_else(|| PlanError::invalid_choices("HOME is not set; set RUSTTV_CONFIG_DIR"))?;
    Ok(if cfg!(target_os = "macos") {
        home.join("Library/Application Support/rusttv")
    } else {
        env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .unwrap_or_else(|| home.join(".config"))
            .join("rusttv")
    })
}

/// Reads the choices from `file`, or from `<config dir>/privacy.toml` if it
/// exists, or returns the defaults. A file that exists but can't be read or
/// parsed is an error (`INVALID_CHOICES`), never silently the defaults.
pub fn load_choices(file: Option<&Path>) -> Result<LoadedChoices, PlanError> {
    let (path, required) = match file {
        Some(file) => (file.to_owned(), true),
        None => (config_dir()?.join("privacy.toml"), false),
    };
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if !required && error.kind() == io::ErrorKind::NotFound => {
            return Ok(LoadedChoices::default());
        }
        Err(error) => {
            return Err(PlanError::invalid_choices(format!(
                "{}: {error}",
                path.display()
            )));
        }
    };
    let choices = toml::from_str(&text).map_err(|error| {
        PlanError::invalid_choices(format!("{}: {}", path.display(), error.message()))
    })?;
    Ok(LoadedChoices {
        path: Some(path),
        choices,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packages_dir() -> &'static Path {
        Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../compat/packages"
        ))
    }

    #[test]
    fn bundled_catalog_is_every_committed_file_and_each_reads() {
        let mut committed: Vec<String> = fs::read_dir(packages_dir())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| name.ends_with(".toml"))
            .collect();
        committed.sort();
        let mut bundled: Vec<&str> = BUNDLED_CATALOG.iter().map(|(name, _)| *name).collect();
        bundled.sort_unstable();
        assert_eq!(bundled, committed, "update BUNDLED_CATALOG");
        let mut total = 0;
        for (name, text) in BUNDLED_CATALOG {
            let file: CatalogFile =
                toml::from_str(text).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(file.schema_version, CATALOG_SCHEMA_VERSION, "{name}");
            assert!(!file.packages.is_empty(), "{name}");
            total += file.packages.len();
        }
        assert_eq!(bundled_catalog().len(), total);
    }

    #[test]
    fn catalog_entries_are_unique_complete_and_never_protected() {
        let mut seen = HashSet::new();
        for entry in bundled_catalog() {
            let name = entry.package.as_str();
            assert!(seen.insert(name.to_owned()), "{name} is listed twice");
            assert_eq!(
                crate::statically_protected(name),
                None,
                "{name} is on compat/protected.toml"
            );
            assert!(!entry.what.trim().is_empty(), "{name}: empty `what`");
            assert!(
                !entry.side_effects.trim().is_empty(),
                "{name}: empty `side_effects`"
            );
            assert!(
                !entry.sources.is_empty() && entry.sources.iter().all(|s| !s.trim().is_empty()),
                "{name}: `sources` must say where the facts come from"
            );
        }
    }

    #[test]
    fn derived_files_carry_the_cleanyourtv_licence_notice() {
        for (name, text) in BUNDLED_CATALOG {
            if text.contains("\"cleanyourtv") {
                assert!(
                    text.contains("MIT License, Copyright (c) 2026 Gathin"),
                    "{name} cites cleanyourtv without its licence notice"
                );
            }
        }
        let licence = include_str!("../../../licenses/cleanyourtv/LICENSE");
        assert!(licence.contains("Copyright (c) 2026 Gathin"));
    }

    #[test]
    fn every_tested_package_of_a_bundled_profile_is_in_the_catalog() {
        let catalog: HashSet<String> = bundled_catalog()
            .into_iter()
            .map(|entry| entry.package.to_string())
            .collect();
        for profile in crate::bundled_profiles() {
            for tested in profile.privacy.iter().flat_map(|privacy| &privacy.tested) {
                assert!(
                    catalog.contains(tested.package.as_str()),
                    "{}: {} is tested but not in the catalog",
                    crate::profile_file_name(&profile),
                    tested.package
                );
                assert_eq!(tested.checked.len(), 10, "{}", tested.package);
            }
        }
    }

    #[test]
    fn choices_parse_with_defaults_and_reject_unknown_fields() {
        let choices: PrivacyChoices = toml::from_str("").unwrap();
        assert_eq!(choices, PrivacyChoices::default());
        assert_eq!(choices.categories, PrivacyCategory::DEFAULT);

        let choices: PrivacyChoices = toml::from_str(
            "categories = [\"telemetry\", \"casting\"]\n\
             keep = [\"com.google.android.apps.mediashell\"]\n\
             also = [\"org.droidtv.amazonalexa\"]\n\
             extra = [\"com.example.preinstalled\"]\n",
        )
        .unwrap();
        assert_eq!(
            choices.categories,
            [PrivacyCategory::Telemetry, PrivacyCategory::Casting]
        );
        choices.validate(&bundled_catalog()).unwrap();

        for bad in [
            "categories = [\"bloat\"]",
            "keep = [\"com.x; reboot\"]",
            "remove = [\"com.example\"]",
        ] {
            assert!(toml::from_str::<PrivacyChoices>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn choices_must_put_catalog_packages_under_also_and_others_under_extra() {
        let catalog = bundled_catalog();
        let package = |name: &str| PackageName::new(name).unwrap();
        let also = PrivacyChoices {
            also: vec![package("com.example.unknown")],
            ..PrivacyChoices::default()
        };
        let error = also.validate(&catalog).unwrap_err();
        assert_eq!(error.code(), "INVALID_CHOICES");
        assert!(error.to_string().contains("`extra`"), "{error}");
        let extra = PrivacyChoices {
            extra: vec![package("org.droidtv.gel")],
            ..PrivacyChoices::default()
        };
        let error = extra.validate(&catalog).unwrap_err();
        assert!(error.to_string().contains("`also`"), "{error}");
    }

    #[test]
    fn a_missing_default_file_means_the_defaults_but_a_missing_given_file_fails() {
        let dir = std::env::temp_dir().join(format!("rusttv-choices-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("nope.toml");
        let error = load_choices(Some(&missing)).unwrap_err();
        assert_eq!(error.code(), "INVALID_CHOICES");

        let file = dir.join("privacy.toml");
        fs::write(&file, "categories = [\"ads\"]\n").unwrap();
        let loaded = load_choices(Some(&file)).unwrap();
        assert_eq!(loaded.path.as_deref(), Some(file.as_path()));
        assert_eq!(loaded.choices.categories, [PrivacyCategory::Ads]);

        fs::write(&file, "categories = \"ads\"\n").unwrap();
        let error = load_choices(Some(&file)).unwrap_err();
        assert_eq!(error.code(), "INVALID_CHOICES");
        assert!(error.to_string().contains("privacy.toml"), "{error}");
        fs::remove_dir_all(dir).unwrap();
    }
}
