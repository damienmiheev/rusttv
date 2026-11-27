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
//! code/<name>.slint             a `runtime: code` theme's layout (home.slint required)
//! scripts/<name>.rhai           its scripts
//! ```
//!
//! `code/` and `scripts/` files are only read here: UTF-8 with no control
//! character but tab and line breaks and no invisible or direction-changing
//! character (so code can't read differently from what it does in any
//! viewer, diff or editor), each
//! capped and all together capped; nothing interprets them yet
//! (PANEL_STUDIO.md § 5.9).
//!
//! Images are kept as PNG bytes with their signature and end chunk checked
//! (so no archive hides behind one); decoding,
//! with its size checks and memory budget, is the renderer's (§ 6).

use crate::document::{AssetName, NodeKind, each_node};
use crate::error::{Code, PackageError, PackageFile, Position};
use crate::messages::{Messages, check_document_messages, check_messages, parse_messages};
use crate::meta::{Capabilities, Runtime, limits};
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

/// A `code/` or `scripts/` file of a `runtime: code` theme.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Source {
    /// Its name without folder and extension, e.g. `home`; the same rule as
    /// image names (`[a-z0-9_-]{1,40}`).
    pub name: AssetName,
    /// Its text.
    pub text: String,
}

impl std::fmt::Debug for Source {
    // The text stays out of logs and error reports.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Source")
            .field("name", &self.name)
            .field("bytes", &self.text.len())
            .finish()
    }
}

/// The most all `code/` and `scripts/` files of one theme may hold together.
pub const MAX_SOURCES: usize = 1 << 20;

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
    /// `code/*.slint`, in zip order.
    pub code: Vec<Source>,
    /// `scripts/*.rhai`, in zip order.
    pub scripts: Vec<Source>,
}

/// What an entry is, from its name.
enum Kind {
    PackageJson,
    Strings(Language),
    Asset(AssetName, AssetVariant),
    Thumbnail,
    Code(AssetName),
    Script(AssetName),
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
    if let Some(file) = name
        .strip_prefix("code/")
        .and_then(|rest| rest.strip_suffix(".slint"))
    {
        return AssetName::parse(file).ok().map(Kind::Code);
    }
    if let Some(file) = name
        .strip_prefix("scripts/")
        .and_then(|rest| rest.strip_suffix(".rhai"))
    {
        return AssetName::parse(file).ok().map(Kind::Script);
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
        Kind::Code(_) | Kind::Script(_) => Ok(limits::SOURCE),
    }
}

/// `bytes` as source text: UTF-8 with no control character but tab and
/// line feed (and carriage return only before a line feed), none of the
/// invisible or direction-changing characters package text cleans out or
/// keeps for display ([`crate::text`]: zero-width joiners, direction marks
/// and isolates, emoji style selectors, a byte order mark), no braille
/// blank and no noncharacter. User-facing text belongs in
/// `strings/<lang>.json`, where those characters are handled.
fn source_text(bytes: Vec<u8>) -> Option<String> {
    let text = String::from_utf8(bytes).ok()?;
    let bad = |c: char| {
        ((crate::text::removed(c) || crate::text::is_format(c)) && !matches!(c, '\t' | '\n' | '\r'))
            || matches!(c, '\u{2800}' | '\u{FFFC}'..='\u{FFFF}' | '\u{FE0E}' | '\u{FE0F}' | '\u{2066}'..='\u{2069}' | '\u{061C}')
    };
    let lone_cr = text
        .char_indices()
        .any(|(i, c)| c == '\r' && !text[i + 1..].starts_with('\n'));
    (!lone_cr && !text.contains(bad)).then_some(text)
}

/// Reads a package zip for a launcher that can do `can`, and checks it as a
/// whole: entry names, `package.json`, the message tables against each
/// other and the document, PNG signatures, that every image node's asset
/// (and every variant's base image) is there, and for a theme with
/// `runtime: code` its `code/` and `scripts/` files (only it may have them;
/// `code/home.slint` is required; text rules and caps as the module says).
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
    // Source files stay bytes until `package.json` says the theme runs
    // code, so the wrong kind of package gets `NOT_FOR_KIND` before any
    // check of their content.
    let mut sources = Vec::new();
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
            Kind::Code(name) => sources.push((index, true, name, entry.bytes)),
            Kind::Script(name) => sources.push((index, false, name, entry.bytes)),
            Kind::Asset(..) | Kind::Thumbnail => return Err(in_zip(Code::BadImage, index)),
        }
    }

    let text = package_json.ok_or_else(|| {
        PackageError::new(Code::MissingFile, Position::File).in_file(PackageFile::Zip)
    })?;
    let package = parse_package(&text, can)?;
    check_messages(&package.meta, &strings)?;
    if package.meta.runtime != Runtime::Code
        && let Some((index, ..)) = sources.first()
    {
        return Err(in_zip(Code::NotForKind, *index));
    }
    let (mut code, mut scripts, mut total) = (Vec::new(), Vec::new(), 0_usize);
    for (index, is_code, name, bytes) in sources {
        total += bytes.len();
        if total > MAX_SOURCES {
            return Err(in_zip(Code::TooLarge, index));
        }
        let text = source_text(bytes).ok_or_else(|| in_zip(Code::BadSource, index))?;
        let list = if is_code { &mut code } else { &mut scripts };
        list.push(Source { name, text });
    }
    if package.meta.runtime == Runtime::Code && !code.iter().any(|s| s.name.as_str() == "home") {
        return Err(
            PackageError::new(Code::MissingFile, Position::field("code/home.slint"))
                .in_file(PackageFile::Zip),
        );
    }

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
        code,
        scripts,
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
    fn knob_labels_are_checked_in_the_zip() {
        let json = package_json("").replace(
            r#""sources": ["history"]"#,
            r#""sources": ["history"], "knobs": [{"name": "on", "type": "toggle", "label": "on", "default": true}]"#,
        );
        let files =
            |en: &'static [u8]| [("package.json", json.as_bytes()), ("strings/en.json", en)];
        let contents = read(&files(br#"{"messages": {"on": "On"}}"#)).unwrap();
        assert_eq!(contents.package.meta.knobs.len(), 1);
        let error = read(&files(br#"{"messages": {"off": "Off"}}"#)).unwrap_err();
        assert_eq!(
            (error.code, error.file, error.at),
            (
                Code::UnknownMessage,
                PackageFile::PackageJson,
                Position::Field {
                    path: "meta.knobs",
                    index: Some(0)
                }
            )
        );
        assert_eq!(
            code(&[("package.json", json.as_bytes())]).map(|c| c.0),
            Some(Code::NoDefaultStrings)
        );
    }

    type File<'a> = (&'a str, &'a [u8]);

    fn theme_json(runtime: &str) -> String {
        format!(
            r#"{{"meta": {{"format": 2, "min_launcher": "0.5.0", "id": "user.dock", "kind": "theme",
               "version": "1.0.0", "default_language": "en", "name": {{"en": "Dock"}}{runtime}}}}}"#
        )
    }

    #[test]
    fn a_code_theme_carries_its_slint_and_scripts() {
        let json = theme_json(r#", "runtime": "code""#);
        let contents = read(&[
            ("package.json", json.as_bytes()),
            (
                "code/home.slint",
                b"import { RustTV } from \"@rusttv/theme.slint\";",
            ),
            ("code/tile.slint", b"// a tile"),
            ("scripts/sort.rhai", b"fn key(app) { app.name }"),
        ])
        .unwrap();
        assert_eq!(contents.package.meta.runtime, Runtime::Code);
        let names: Vec<&str> = contents.code.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["home", "tile"]);
        assert_eq!(contents.scripts[0].text, "fn key(app) { app.name }");
        // A data theme (the default) has none.
        let data = theme_json("");
        assert_eq!(
            read(&[("package.json", data.as_bytes())])
                .unwrap()
                .package
                .meta
                .runtime,
            Runtime::Data
        );
    }

    #[test]
    fn code_files_have_their_rules() {
        let json = theme_json(r#", "runtime": "code""#);
        let data = theme_json(r#", "runtime": "data""#);
        let home: (&str, &[u8]) = ("code/home.slint", b"x");
        let cases: [(&[File<'_>], Code); 7] = [
            (&[("package.json", json.as_bytes())], Code::MissingFile),
            (
                &[("package.json", json.as_bytes()), ("code/tile.slint", b"x")],
                Code::MissingFile,
            ),
            (&[("package.json", data.as_bytes()), home], Code::NotForKind),
            (
                &[("package.json", data.as_bytes()), ("scripts/a.rhai", b"x")],
                Code::NotForKind,
            ),
            (
                &[
                    ("package.json", json.as_bytes()),
                    ("code/home.slint", b"a\xffb"),
                ],
                Code::BadSource,
            ),
            (
                &[
                    ("package.json", json.as_bytes()),
                    ("code/home.slint", "a\u{202e}b".as_bytes()),
                ],
                Code::BadSource,
            ),
            (
                &[
                    ("package.json", json.as_bytes()),
                    home,
                    ("scripts/a.rhai", b"a\0b"),
                ],
                Code::BadSource,
            ),
        ];
        for (files, want) in cases {
            assert_eq!(read(files).err().map(|e| e.code), Some(want));
        }
        for name in [
            "code/Home.slint",
            "code/home.rs",
            "code/a/home.slint",
            "scripts/a.lua",
        ] {
            assert_eq!(
                code(&[("package.json", json.as_bytes()), home, (name, b"x")]),
                Some((Code::BadEntryName, PackageFile::Zip)),
                "{name}"
            );
        }
        let big = vec![b'a'; limits::SOURCE + 1];
        assert_eq!(
            code(&[("package.json", json.as_bytes()), ("code/home.slint", &big)]),
            Some((Code::TooLarge, PackageFile::Zip))
        );
        let at_limit = vec![b'a'; limits::SOURCE];
        let stored = zip(&[
            ("package.json", json.as_bytes(), false),
            ("code/home.slint", &at_limit, false),
        ]);
        let can = Capabilities::new(Version::new(0, 5, 0), [""; 0]);
        assert!(read_package(&stored, &can).is_ok());
    }

    #[test]
    fn source_text_refuses_hidden_characters_and_keeps_the_rest_exact() {
        let json = theme_json(r#", "runtime": "code""#);
        let with = |text: &str| {
            read(&[
                ("package.json", json.as_bytes()),
                ("code/home.slint", text.as_bytes()),
            ])
        };
        assert_eq!(with("a\rb").unwrap_err().code, Code::BadSource);
        assert_eq!(with("\u{FEFF}a").unwrap_err().code, Code::BadSource);
        let exact = "a\tb\r\nc\n  d  ";
        assert_eq!(with(exact).unwrap().code[0].text, exact);
        assert!(with("emoji ✓ und Grüße").is_ok());
        for c in [
            '\0',
            '\u{1B}',
            '\u{7F}',
            '\u{85}',
            '\u{0B}',
            '\u{0C}',
            '\u{08}',
            '\u{200B}',
            '\u{200C}',
            '\u{200D}',
            '\u{200E}',
            '\u{200F}',
            '\u{061C}',
            '\u{202A}',
            '\u{202E}',
            '\u{2066}',
            '\u{2069}',
            '\u{2028}',
            '\u{2029}',
            '\u{2060}',
            '\u{FEFF}',
            '\u{00AD}',
            '\u{FE0F}',
            '\u{E0041}',
        ] {
            let error = with(&format!("a{c}b")).unwrap_err();
            assert_eq!(
                (error.code, error.at, error.file),
                (
                    Code::BadSource,
                    Position::Entry { index: 1 },
                    PackageFile::Zip
                ),
                "U+{:04X}",
                u32::from(c)
            );
        }
    }

    #[test]
    fn code_entries_belong_to_code_themes_only() {
        let code_theme = theme_json(r#", "runtime": "code""#);
        let data_theme = theme_json(r#", "runtime": "data""#);
        let panel = package_json("");
        // The wrong kind of package is told so before its content is read.
        for json in [&data_theme, &panel] {
            let error = read(&[
                ("package.json", json.as_bytes()),
                ("code/home.slint", b"a\0b"),
            ])
            .unwrap_err();
            assert_eq!(
                (error.code, error.at),
                (Code::NotForKind, Position::Entry { index: 1 })
            );
        }
        assert!(read(&[("package.json", panel.as_bytes())]).is_ok());
        // Scripts alone don't make a home.
        let error = read(&[
            ("package.json", code_theme.as_bytes()),
            ("scripts/home.rhai", b"x"),
        ])
        .unwrap_err();
        assert_eq!(
            (error.code, error.at),
            (Code::MissingFile, Position::field("code/home.slint"))
        );
        let home: (&str, &[u8]) = ("code/home.slint", b"x");
        let forty = format!("code/{}.slint", "a".repeat(40));
        assert!(
            read(&[
                ("package.json", code_theme.as_bytes()),
                home,
                (&forty, b"x")
            ])
            .is_ok()
        );
        let forty_one = format!("code/{}.slint", "a".repeat(41));
        for name in [
            "code/x.rhai",
            "scripts/x.slint",
            "scripts/a/b.rhai",
            "code/.slint",
            "code/home.SLINT",
            forty_one.as_str(),
        ] {
            assert_eq!(
                code(&[("package.json", code_theme.as_bytes()), home, (name, b"x")]),
                Some((Code::BadEntryName, PackageFile::Zip)),
                "{name}"
            );
        }
        assert_eq!(
            code(&[("package.json", code_theme.as_bytes()), home, home]).map(|c| c.0),
            Some(Code::DuplicateEntry)
        );
        // All sources together are capped too (stored, to stay under the ratio cap).
        let part = vec![b'a'; limits::SOURCE];
        let names: Vec<String> = (0..5).map(|i| format!("code/p{i}.slint")).collect();
        let mut files = vec![
            ("package.json", code_theme.as_bytes(), false),
            ("code/home.slint", b"x".as_slice(), false),
        ];
        files.extend(names.iter().map(|n| (n.as_str(), part.as_slice(), false)));
        let can = Capabilities::new(Version::new(0, 5, 0), [""; 0]);
        let error = read_package(&zip(&files), &can).unwrap_err();
        assert_eq!(
            (error.code, error.at),
            // 1 byte + 4 × 256 KiB passes 1 MiB at the fourth part, entry 5.
            (Code::TooLarge, Position::Entry { index: 5 })
        );
    }

    #[test]
    fn only_a_theme_may_run_code() {
        let panel = package_json("").replacen(
            r#""kind": "panel","#,
            r#""kind": "panel", "runtime": "code","#,
            1,
        );
        let error = read(&[("package.json", panel.as_bytes())]).unwrap_err();
        assert_eq!(
            (error.code, error.at),
            (Code::NotForKind, Position::field("meta.runtime"))
        );
        let odd = theme_json(r#", "runtime": "wasm""#);
        assert_eq!(
            read(&[("package.json", odd.as_bytes())]).unwrap_err().code,
            Code::BadValue
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
