//! A whole package: the zip's entries checked by name and kind, then read
//! and checked against each other (PANEL_STUDIO.md § 5.1).
//!
//! ```text
//! package.json                  meta + document (required)
//! strings/<lang>.json           message tables
//! assets/<name>.png             images
//! assets/<lang>/<name>.png      a language's variant of an image
//! assets/<mode>/<name>.png      a theme mode's variant (light, dark, high-contrast, night)
//! thumbnail.png                 for the studio and the Panels page
//! ```
//!
//! Images are kept as PNG bytes with their signature and end chunk checked
//! (so no archive hides behind one); decoding,
//! with its size checks and memory budget, is the renderer's (§ 6).

use crate::document::{AssetName, NodeKind, each_node};
use crate::error::{Code, PackageError, PackageFile, Position};
use crate::messages::{Messages, check_document_messages, check_messages, parse_messages};
use crate::meta::{Capabilities, limits};
use crate::text::Language;
use crate::zip::{self, MAX_ZIP};
use crate::{MAX_PACKAGE_JSON, Package, parse_package};
use std::collections::{BTreeMap, BTreeSet};

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
/// The last chunk of every PNG: nothing (another archive, say) may follow.
const PNG_END: [u8; 12] = [0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82];

/// A theme mode an image can have its own variant for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum AssetMode {
    /// `light`.
    Light,
    /// `dark`.
    Dark,
    /// `high-contrast`.
    HighContrast,
    /// `night`.
    Night,
}

impl AssetMode {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            "high-contrast" => Some(Self::HighContrast),
            "night" => Some(Self::Night),
            _ => None,
        }
    }
}

/// Which variant of an image an asset is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum AssetVariant {
    /// `assets/<name>.png`, used when no variant fits.
    Base,
    /// `assets/<lang>/<name>.png`.
    Language(Language),
    /// `assets/<mode>/<name>.png`.
    Mode(AssetMode),
}

/// One image, as PNG bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Asset {
    /// The name image nodes use.
    pub name: AssetName,
    /// Which variant.
    pub variant: AssetVariant,
    /// The PNG file.
    pub png: Vec<u8>,
}

/// A package read from its zip and checked as a whole.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Contents {
    /// `package.json`.
    pub package: Package,
    /// The message tables by language.
    pub strings: BTreeMap<Language, Messages>,
    /// The images.
    pub assets: Vec<Asset>,
    /// `thumbnail.png`, if there is one.
    pub thumbnail: Option<Vec<u8>>,
}

/// What an entry is, from its name.
enum Kind {
    PackageJson,
    Strings(Language),
    Asset(AssetName, AssetVariant),
    Thumbnail,
}

fn kind(name: &str) -> Option<Kind> {
    if name == "package.json" {
        return Some(Kind::PackageJson);
    }
    if name == "thumbnail.png" {
        return Some(Kind::Thumbnail);
    }
    if let Some(language) = name
        .strip_prefix("strings/")
        .and_then(|rest| rest.strip_suffix(".json"))
    {
        return Language::parse(language).ok().map(Kind::Strings);
    }
    let rest = name.strip_prefix("assets/")?.strip_suffix(".png")?;
    let (variant, file) = match rest.split_once('/') {
        None => (AssetVariant::Base, rest),
        Some((folder, file)) => {
            let variant = match (Language::parse(folder), AssetMode::parse(folder)) {
                (Ok(language), _) => AssetVariant::Language(language),
                (_, Some(mode)) => AssetVariant::Mode(mode),
                _ => return None,
            };
            (variant, file)
        }
    };
    Some(Kind::Asset(AssetName::parse(file).ok()?, variant))
}

/// The most an entry may inflate to.
fn cap(name: &str) -> Result<usize, Code> {
    match kind(name).ok_or(Code::BadEntryName)? {
        Kind::PackageJson => Ok(MAX_PACKAGE_JSON),
        Kind::Strings(_) => Ok(limits::STRINGS_JSON),
        // A PNG is already compressed, so one larger than a whole package
        // zip can only be a bomb.
        Kind::Asset(..) | Kind::Thumbnail => Ok(MAX_ZIP),
    }
}

/// Reads a package zip for a launcher that can do `can`, and checks it as a
/// whole: entry names, `package.json`, the message tables against each
/// other and the document, PNG signatures, and that every image node's
/// asset (and every variant's base image) is there.
///
/// # Errors
///
/// A [`PackageError`] for the first problem found, in [`PackageFile::Zip`]
/// for the zip and its entries and in the file concerned otherwise.
pub fn read_package(zip: &[u8], can: &Capabilities) -> Result<Contents, PackageError> {
    let entries = zip::read(zip, cap)?;
    let in_zip =
        |code, index| PackageError::new(code, Position::Entry { index }).in_file(PackageFile::Zip);
    let utf8 = |bytes: &[u8], file| {
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| PackageError::new(Code::NotJson, Position::File).in_file(file))
    };

    let mut package_json = None;
    let mut strings = BTreeMap::new();
    let mut assets = Vec::new();
    let mut asset_entries = Vec::new();
    let mut thumbnail = None;
    for (index, entry) in entries.into_iter().enumerate() {
        let is_png = entry.bytes.starts_with(&PNG_SIGNATURE) && entry.bytes.ends_with(&PNG_END);
        match kind(&entry.name).ok_or_else(|| in_zip(Code::BadEntryName, index))? {
            Kind::PackageJson => {
                package_json = Some(utf8(&entry.bytes, PackageFile::PackageJson)?);
            }
            Kind::Strings(language) => {
                let text = utf8(&entry.bytes, PackageFile::Strings(language))?;
                strings.insert(language, parse_messages(language, &text)?);
            }
            Kind::Asset(name, variant) if is_png => {
                asset_entries.push(index);
                assets.push(Asset {
                    name,
                    variant,
                    png: entry.bytes,
                });
            }
            Kind::Thumbnail if is_png => thumbnail = Some(entry.bytes),
            Kind::Asset(..) | Kind::Thumbnail => return Err(in_zip(Code::BadImage, index)),
        }
    }

    let text = package_json.ok_or_else(|| {
        PackageError::new(Code::MissingFile, Position::File).in_file(PackageFile::Zip)
    })?;
    let package = parse_package(&text, can)?;
    check_messages(&package.meta, &strings)?;

    let bases: BTreeSet<&str> = assets
        .iter()
        .filter(|a| a.variant == AssetVariant::Base)
        .map(|a| a.name.as_str())
        .collect();
    // A language or mode variant needs its base image to fall back to.
    if let Some(orphan) = assets.iter().position(|a| !bases.contains(a.name.as_str())) {
        let index = asset_entries.get(orphan).copied().unwrap_or_default();
        return Err(in_zip(Code::MissingAsset, index));
    }
    if let Some(document) = &package.document {
        check_document_messages(&package.meta, document, &strings)?;
        each_node(document, &mut |variant, node, n| match &n.asset {
            Some(asset) if n.kind == NodeKind::Image && !bases.contains(asset.as_str()) => Err(
                PackageError::new(Code::MissingAsset, Position::Node { variant, node }),
            ),
            _ => Ok(()),
        })?;
    }
    Ok(Contents {
        package,
        strings,
        assets,
        thumbnail,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Version;
    use crate::zip::tests::zip;

    const PNG: &[u8] = &[
        0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0, 0, 0, b'I', b'E', b'N', b'D',
        0xae, 0x42, 0x60, 0x82,
    ];

    fn package_json(children: &str) -> String {
        format!(
            r#"{{"meta": {{"format": 2, "min_launcher": "0.5.0", "id": "user.test", "kind": "panel",
               "version": "1.0.0", "default_language": "en", "name": {{"en": "Test"}}, "faces": ["tile"],
               "sources": ["history"]}},
              "document": {{"variants": [{{"face": "tile", "root": {{"id": "root", "type": "frame",
               "w": 250, "h": 141, "children": [{children}]}}}}]}}}}"#
        )
    }

    const LOGO: &str = r#"{"id": "logo", "type": "image", "w": 48, "h": 48, "asset": "logo"}"#;
    const UNREAD: &str = r#"{"id": "t", "type": "text", "w": 10, "h": 10, "text": "…", "size": 10,
        "bindings": {"text": {"msg": "unread", "args": {"count": {"source": "history.waiting"}}}}}"#;
    const EN: &[u8] =
        br#"{"messages": {"unread": {"plural": "count", "one": "1 new", "other": "{count} new"}}}"#;

    fn read(files: &[(&str, &[u8])]) -> Result<Contents, PackageError> {
        let files: Vec<(&str, &[u8], bool)> = files.iter().map(|(n, b)| (*n, *b, true)).collect();
        read_package(
            &zip(&files),
            &Capabilities::new(Version::new(0, 5, 0), [""; 0]),
        )
    }

    fn code(files: &[(&str, &[u8])]) -> Option<(Code, PackageFile)> {
        read(files).err().map(|e| (e.code, e.file))
    }

    #[test]
    fn a_whole_package_reads() {
        let json = package_json(&format!("{LOGO}, {UNREAD}"));
        let contents = read(&[
            ("package.json", json.as_bytes()),
            ("strings/en.json", EN),
            ("strings/uk.json", br#"{"messages": {}}"#),
            ("assets/logo.png", PNG),
            ("assets/dark/logo.png", PNG),
            ("assets/uk/logo.png", PNG),
            ("thumbnail.png", PNG),
        ])
        .unwrap();
        assert_eq!(contents.strings.len(), 2);
        assert_eq!(contents.assets.len(), 3);
        assert!(contents.thumbnail.is_some());
        assert_eq!(
            contents.assets[1].variant,
            AssetVariant::Mode(AssetMode::Dark)
        );
    }

    #[test]
    fn only_known_names_are_allowed() {
        let json = package_json("");
        for name in [
            "readme.txt",
            "assets/logo.svg",
            "assets/Logo.png",
            "assets/",
            "assets/fr_CA/logo.png",
            "assets/dark/x/logo.png",
            "strings/pt-BR.json",
            "fonts/a.ttf",
            "package.json/",
            "../package.json",
        ] {
            assert_eq!(
                code(&[("package.json", json.as_bytes()), (name, PNG)]),
                Some((Code::BadEntryName, PackageFile::Zip)),
                "{name}"
            );
        }
    }

    #[test]
    fn files_and_assets_must_be_there_and_be_pngs() {
        let json = package_json(LOGO);
        assert_eq!(
            code(&[("assets/logo.png", PNG)]),
            Some((Code::MissingFile, PackageFile::Zip))
        );
        assert_eq!(
            code(&[("package.json", json.as_bytes())]).map(|c| c.0),
            Some(Code::MissingAsset)
        );
        assert_eq!(
            code(&[
                ("package.json", json.as_bytes()),
                ("assets/logo.png", b"GIF89a")
            ]),
            Some((Code::BadImage, PackageFile::Zip))
        );
        let mut tail = PNG.to_vec();
        tail.extend(b"PK\x03\x04");
        assert_eq!(
            code(&[
                ("package.json", json.as_bytes()),
                ("assets/logo.png", &tail)
            ]),
            Some((Code::BadImage, PackageFile::Zip))
        );
        let plain = package_json("");
        assert_eq!(
            code(&[
                ("package.json", plain.as_bytes()),
                ("assets/dark/logo.png", PNG)
            ])
            .map(|c| c.0),
            Some(Code::MissingAsset)
        );
    }

    #[test]
    fn tables_are_checked_with_the_package() {
        let json = package_json(UNREAD);
        assert_eq!(
            code(&[("package.json", json.as_bytes())]).map(|c| c.0),
            Some(Code::NoDefaultStrings)
        );
        let uk_extra = br#"{"messages": {"other": "x"}}"#;
        let lang = Language::parse("uk").unwrap();
        assert_eq!(
            code(&[
                ("package.json", json.as_bytes()),
                ("strings/en.json", EN),
                ("strings/uk.json", uk_extra)
            ]),
            Some((Code::UnknownMessage, PackageFile::Strings(lang)))
        );
        assert_eq!(
            code(&[("package.json", b"\xff\xfe")]),
            Some((Code::NotJson, PackageFile::PackageJson))
        );
    }

    #[test]
    fn per_file_caps_hold_inside_the_zip() {
        let huge = vec![b' '; MAX_PACKAGE_JSON + 1];
        let stored = zip(&[("package.json", &huge, false)]);
        let can = Capabilities::new(Version::new(0, 5, 0), [""; 0]);
        let error = read_package(&stored, &can).unwrap_err();
        assert_eq!((error.code, error.file), (Code::TooLarge, PackageFile::Zip));
    }
}
