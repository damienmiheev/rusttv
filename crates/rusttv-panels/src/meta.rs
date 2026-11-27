//! A package's meta (PANEL_STUDIO.md § 5.2) and the version check that runs
//! before strict parsing (§ 5.1).

use crate::error::{Code, PackageError, Position};
use crate::knob::{Knob, RawKnob};
use crate::text::{Language, Text};
use serde::{Deserialize, Deserializer};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::Deref;

/// The package format this crate reads.
pub const FORMAT: u32 = 2;

/// Most entries in `requires`, `faces`, `sources` and `actions` each.
pub const MAX_LIST: usize = 32;

/// The names of RustTV's own tiles and pages (`crates/rusttv-launcher/src/model.rs`).
/// A user package id can't use them, nor anything starting with `rusttv`,
/// so it can't pose as one. (User ids live under `user.`, so they never
/// collide with `panel:<id>` anyway.)
pub const RESERVED_NAMES: [&str; 7] = [
    "about",
    "settings",
    "remote",
    "history",
    "hidden-apps",
    "themes",
    "panels",
];

/// A version, `<major>.<minor>.<patch>`: decimal numbers of at most 9 digits
/// without leading zeros, no pre-release or build part. Ordered by number.
///
/// The same rule as RustTV's release versions (RELEASE.md § Version); its own
/// copy, because this crate doesn't depend on `rusttv-core`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    major: u32,
    minor: u32,
    patch: u32,
}

impl Version {
    /// Parses `1.2.3`.
    ///
    /// # Errors
    ///
    /// [`Code::BadVersion`] for anything else (`1.2`, `1.2.3-rc1`, `01.2.3`).
    pub fn parse(text: &str) -> Result<Self, Code> {
        let number = |part: Option<&str>| {
            let part = part.ok_or(Code::BadVersion)?;
            let plain = (1..=9).contains(&part.len())
                && part.bytes().all(|b| b.is_ascii_digit())
                && (part == "0" || !part.starts_with('0'));
            if plain {
                part.parse().map_err(|_| Code::BadVersion)
            } else {
                Err(Code::BadVersion)
            }
        };
        let mut parts = text.split('.');
        let version = Self {
            major: number(parts.next())?,
            minor: number(parts.next())?,
            patch: number(parts.next())?,
        };
        if parts.next().is_some() {
            Err(Code::BadVersion)
        } else {
            Ok(version)
        }
    }

    /// A version from its numbers.
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

deserialize_via!(Version, Version::parse);

/// `true` for `[a-z0-9-]{1,32}` that neither starts nor ends with `-`.
fn is_name(text: &str) -> bool {
    (1..=32).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !text.starts_with('-')
        && !text.ends_with('-')
}

/// A short machine name: `[a-z0-9-]{1,32}`, not starting or ending with `-`.
/// Used for features, sources and actions.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Name(String);

impl Name {
    /// The name in `text`.
    ///
    /// # Errors
    ///
    /// [`Code::BadName`] if `text` isn't a name.
    pub fn parse(text: &str) -> Result<Self, Code> {
        if is_name(text) {
            Ok(Self(text.to_owned()))
        } else {
            Err(Code::BadName)
        }
    }

    /// The name.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `default`, a variant's state when it names none.
    /// The state a variant has when it names none.
    pub(crate) const DEFAULT_STATE: &str = "default";

    pub(crate) fn default_state() -> Self {
        Self(Self::DEFAULT_STATE.to_owned())
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

deserialize_via!(Name, Name::parse);

/// A user package id: `user.<name>`, e.g. `user.weather-tiles`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageId(String);

impl PackageId {
    /// The id in `text`.
    ///
    /// # Errors
    ///
    /// [`Code::BadId`] for the wrong shape, [`Code::ReservedId`] for a name
    /// RustTV keeps for itself.
    pub fn parse(text: &str) -> Result<Self, Code> {
        let name = text
            .strip_prefix("user.")
            .filter(|name| is_name(name))
            .ok_or(Code::BadId)?;
        if name.starts_with("rusttv") || RESERVED_NAMES.contains(&name) {
            return Err(Code::ReservedId);
        }
        Ok(Self(text.to_owned()))
    }

    /// The id, e.g. `user.weather-tiles`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PackageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

deserialize_via!(PackageId, PackageId::parse);

word_enum! {
    /// What a package is.
    Kind {
        /// A panel: faces on RustTV's surfaces.
        Panel => "panel",
        /// A theme: tokens and assets.
        Theme => "theme",
    }
}

word_enum! {
    /// Where a panel can show (PANEL_STUDIO.md § 5.3, § 8.1).
    Face {
        /// A Home tile.
        Tile => "tile",
        /// A row in Settings or the menu.
        Row => "row",
        /// A full page.
        Page => "page",
        /// A glyph in the status corner.
        Corner => "corner",
        /// A banner card.
        Banner => "banner",
        /// The idle screen.
        Ambient => "ambient",
        /// The hero area above Home.
        Hero => "hero",
        /// The phone's Home mirror.
        Phone => "phone",
    }
}

/// Lengths of the meta's texts, in characters after cleaning.
pub mod limits {
    /// A package's name (PANEL_STUDIO.md § 4 name rules).
    pub const NAME: usize = 24;
    /// The author, as the package states it.
    pub const AUTHOR: usize = 40;
    /// The licence, e.g. `MIT OR Apache-2.0`.
    pub const LICENCE: usize = 64;
    /// A description.
    pub const DESCRIPTION: usize = 280;
    /// The homepage URL.
    pub const URL: usize = 200;
    /// A text node's text.
    pub const TEXT: usize = 500;
    /// A node's accessibility label.
    pub const LABEL: usize = 200;
    /// Nodes in one variant.
    pub const NODES: usize = 500;
    /// Nesting of nodes in one variant (the root is 1).
    pub const DEPTH: usize = 8;
    /// Drop shadows in one variant.
    pub const SHADOWS: usize = 4;
    /// Strings in one message table (each plural form counts).
    pub const STRINGS: usize = 2000;
    /// Placeholders in one text.
    pub const PLACEHOLDERS: usize = 16;
    /// Characters of a value filled into a placeholder; longer ones are cut.
    pub const ARG: usize = 500;
    /// Largest `strings/<lang>.json`, in bytes.
    pub const STRINGS_JSON: usize = 1 << 20;
    /// Bindings in a variant.
    pub const BINDINGS: usize = 100;
    /// Nesting of one expression.
    pub const EXPR_DEPTH: usize = 8;
    /// Parts of one expression (literals, sources, calls…).
    pub const EXPR_NODES: usize = 64;
    /// Variants in a document.
    pub const VARIANTS: usize = 32;
    /// JSON objects inside `document` (nodes, variants, strokes, shadows),
    /// counted before parsing so a large file can't build a large tree.
    pub const DOCUMENT_OBJECTS: usize = 8192;
    /// Knobs in a package.
    pub const KNOBS: usize = 32;
    /// Options of a choice knob.
    pub const KNOB_OPTIONS: usize = 32;
    /// Longest text a text knob holds, in characters.
    pub const KNOB_TEXT: usize = 200;
    /// Most steps from a number knob's `min` to its `max`.
    pub const KNOB_STEPS: usize = 10_000;
}

/// The homepage: an `https://` link with a plain host (ASCII letters,
/// digits, dots, hyphens, an optional port) and a path without spaces,
/// quotes, angle brackets or backslashes. Shown as text, never opened by the TV.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomepageUrl(String);

impl HomepageUrl {
    /// The link in `text`.
    ///
    /// # Errors
    ///
    /// [`Code::BadUrl`] if `text` isn't such a link.
    pub fn parse(text: &str) -> Result<Self, Code> {
        let rest = text.strip_prefix("https://").ok_or(Code::BadUrl)?;
        let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, path) = rest.split_at(authority_end);
        let (host, port) = authority.split_once(':').unwrap_or((authority, "443"));
        let host_ok = host.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
        let port_ok =
            port.bytes().all(|b| b.is_ascii_digit()) && port.parse::<u16>().is_ok_and(|p| p > 0);
        let path_ok = path
            .bytes()
            .all(|b| b.is_ascii_graphic() && !b"\"'<>\\`".contains(&b));
        if text.len() <= limits::URL && host_ok && port_ok && path_ok {
            Ok(Self(text.to_owned()))
        } else {
            Err(Code::BadUrl)
        }
    }

    /// The link.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for HomepageUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

deserialize_via!(HomepageUrl, HomepageUrl::parse);

/// A list of distinct entries, at most [`MAX_LIST`], in the package's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct List<T>(Vec<T>);

impl<T> Default for List<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<T> Deref for List<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &self.0
    }
}

impl<'de, T: Deserialize<'de> + Ord> Deserialize<'de> for List<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let items = Vec::<T>::deserialize(deserializer)?;
        if items.len() > MAX_LIST {
            return Err(serde::de::Error::custom(Code::TooMany.as_str()));
        }
        let distinct: BTreeSet<&T> = items.iter().collect();
        if distinct.len() != items.len() {
            return Err(serde::de::Error::custom(Code::Duplicate.as_str()));
        }
        Ok(Self(items))
    }
}

/// The meta of a format-2 package. Only [`crate::parse_package`] makes one,
/// so every `Meta` has passed the version check and every rule.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Meta {
    /// The oldest launcher that reads it.
    pub min_launcher: Version,
    /// Renderer features it needs.
    pub requires: List<Name>,
    /// Its id.
    pub id: PackageId,
    /// Panel or theme.
    pub kind: Kind,
    /// Its version.
    pub version: Version,
    /// The language its texts fall back to.
    pub default_language: Language,
    /// Its name per language; has the default language.
    pub name: BTreeMap<Language, Text<{ limits::NAME }>>,
    /// Its description per language.
    pub description: BTreeMap<Language, Text<{ limits::DESCRIPTION }>>,
    /// The author as the package states it; shown "unverified".
    pub author: Option<Text<{ limits::AUTHOR }>>,
    /// Its licence, as stated.
    pub licence: Option<Text<{ limits::LICENCE }>>,
    /// A link, shown as text.
    pub homepage: Option<HomepageUrl>,
    /// The faces it implements.
    pub faces: List<Face>,
    /// The source groups it reads (see [`crate::SOURCES`]).
    pub sources: List<Name>,
    /// The actions it uses (checked against the action list in a later slice).
    pub actions: List<Name>,
    /// The settings it offers (§ 7).
    pub knobs: Vec<Knob>,
}

/// `meta` as written, strictly parsed; [`check_meta`] turns it into a [`Meta`].
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawMeta {
    format: u32,
    min_launcher: Version,
    #[serde(default)]
    requires: List<Name>,
    id: PackageId,
    kind: Kind,
    version: Version,
    default_language: Language,
    name: BTreeMap<Language, Text<{ limits::NAME }>>,
    #[serde(default)]
    description: BTreeMap<Language, Text<{ limits::DESCRIPTION }>>,
    #[serde(default)]
    author: Option<Text<{ limits::AUTHOR }>>,
    #[serde(default)]
    licence: Option<Text<{ limits::LICENCE }>>,
    #[serde(default)]
    homepage: Option<HomepageUrl>,
    #[serde(default)]
    faces: List<Face>,
    #[serde(default)]
    sources: List<Name>,
    #[serde(default)]
    actions: List<Name>,
    #[serde(default)]
    knobs: Vec<RawKnob>,
}

/// What the launcher reading a package can do.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Capabilities {
    /// The launcher's version.
    pub launcher: Version,
    /// Renderer features it draws.
    pub features: BTreeSet<String>,
}

impl Capabilities {
    /// A launcher at `launcher` that draws `features`.
    pub fn new(launcher: Version, features: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            launcher,
            features: features.into_iter().map(Into::into).collect(),
        }
    }
}

/// The fields read before strict parsing, leniently, so a package from a
/// newer format gets [`Code::PackageTooNew`] rather than "unknown field".
///
/// Every future format must keep `meta.format` where it is: a package that
/// moves it gets [`Code::MissingField`] here, not "too new".
#[derive(Deserialize)]
struct Head {
    meta: HeadMeta,
}

#[derive(Deserialize)]
struct HeadMeta {
    format: serde_json::Value,
    #[serde(default)]
    min_launcher: Option<serde_json::Value>,
    #[serde(default)]
    requires: Option<serde_json::Value>,
}

/// Step 1 of reading a package: its format, `min_launcher` and `requires`
/// against what this launcher can do.
pub(crate) fn check_head(text: &str, can: &Capabilities) -> Result<(), PackageError> {
    let head: Head = serde_json::from_str(text).map_err(|e| PackageError::from_typed_json(&e))?;
    let meta = head.meta;
    let error = |code, path| PackageError::new(code, Position::field(path));
    let needs = meta
        .min_launcher
        .as_ref()
        .and_then(|v| v.as_str())
        .and_then(|v| Version::parse(v).ok());
    let format = meta
        .format
        .as_u64()
        .ok_or_else(|| error(Code::WrongType, "meta.format"))?;
    if format > u64::from(FORMAT) {
        return Err(PackageError {
            needs,
            ..error(Code::PackageTooNew, "meta.format")
        });
    }
    if format < u64::from(FORMAT) {
        return Err(error(Code::FormatUnsupported, "meta.format"));
    }
    let min = meta
        .min_launcher
        .ok_or_else(|| error(Code::MissingField, "meta.min_launcher"))?;
    let min = min
        .as_str()
        .and_then(|v| Version::parse(v).ok())
        .ok_or_else(|| error(Code::BadVersion, "meta.min_launcher"))?;
    if min > can.launcher {
        return Err(PackageError {
            needs: Some(min),
            ..error(Code::PackageTooNew, "meta.min_launcher")
        });
    }
    if let Some(requires) = meta.requires.filter(|v| !v.is_null()) {
        let list = requires
            .as_array()
            .ok_or_else(|| error(Code::WrongType, "meta.requires"))?;
        for (index, feature) in list.iter().enumerate() {
            let at = Position::Field {
                path: "meta.requires",
                index: Some(index),
            };
            let name = feature
                .as_str()
                .ok_or(PackageError::new(Code::WrongType, at))?;
            if !can.features.contains(name) {
                return Err(PackageError::new(Code::RequiresFeature, at));
            }
        }
    }
    Ok(())
}

/// The checks across fields, after strict parsing; the format is checked
/// again so no `Meta` exists for any other format.
pub(crate) fn check_meta(raw: RawMeta) -> Result<Meta, PackageError> {
    if raw.format != FORMAT {
        return Err(PackageError::new(
            Code::FormatUnsupported,
            Position::field("meta.format"),
        ));
    }
    if !raw.name.contains_key(&raw.default_language) {
        return Err(PackageError::new(
            Code::NoDefaultName,
            Position::field("meta.name"),
        ));
    }
    Ok(Meta {
        min_launcher: raw.min_launcher,
        requires: raw.requires,
        id: raw.id,
        kind: raw.kind,
        version: raw.version,
        default_language: raw.default_language,
        name: raw.name,
        description: raw.description,
        author: raw.author,
        licence: raw.licence,
        homepage: raw.homepage,
        faces: raw.faces,
        sources: raw.sources,
        actions: raw.actions,
        knobs: crate::knob::check(raw.knobs)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_three_plain_numbers() {
        assert_eq!(Version::parse("0.4.0"), Ok(Version::new(0, 4, 0)));
        for bad in [
            "0.4",
            "0.4.0.1",
            "01.2.3",
            "1.2.3-rc1",
            "1..3",
            "",
            "9999999999.0.0",
            "١.٢.٣",
        ] {
            assert_eq!(Version::parse(bad), Err(Code::BadVersion), "{bad}");
        }
        assert!(Version::new(0, 10, 0) > Version::new(0, 9, 9));
    }

    #[test]
    fn ids_are_user_names_and_rusttvs_names_are_reserved() {
        assert!(PackageId::parse("user.weather-tiles").is_ok());
        for bad in [
            "weather",
            "user.",
            "user.Weather",
            "user.-x",
            "user.x-",
            "panel:about",
            "user.a.b",
        ] {
            assert_eq!(PackageId::parse(bad), Err(Code::BadId), "{bad}");
        }
        assert_eq!(
            PackageId::parse(&format!("user.{}", "a".repeat(33))),
            Err(Code::BadId)
        );
        for reserved in [
            "user.about",
            "user.history",
            "user.hidden-apps",
            "user.rusttv",
            "user.rusttv-clock",
        ] {
            assert_eq!(
                PackageId::parse(reserved),
                Err(Code::ReservedId),
                "{reserved}"
            );
        }
    }

    #[test]
    fn homepages_are_plain_https_links() {
        for good in [
            "https://example.org/panels?x=1",
            "https://example.org:8443/a#b",
            "https://a-b.example",
        ] {
            assert!(HomepageUrl::parse(good).is_ok(), "{good}");
        }
        for bad in [
            "http://example.org",
            "https://",
            "https://user@evil.org",
            "https://a b",
            "javascript:x",
            "https://:443",
            "https://[::1]",
            r"https://a.b\.evil.com",
            r#"https://x<y>"'"#,
            "https://ex..ample.org",
            "https://example.org:99999x",
            "https://a:99999/",
            "https://a:0",
            "https://a.-b",
            "https://a-.b",
        ] {
            assert_eq!(HomepageUrl::parse(bad), Err(Code::BadUrl), "{bad}");
        }
    }

    #[test]
    fn lists_refuse_duplicates_and_overlong() {
        let dup: Result<List<Face>, _> = serde_json::from_str(r#"["tile","tile"]"#);
        assert!(dup.unwrap_err().to_string().starts_with("DUPLICATE"));
        let many = format!(
            "[{}]",
            (0..33)
                .map(|i| format!("\"f{i}\""))
                .collect::<Vec<_>>()
                .join(",")
        );
        let long: Result<List<Name>, _> = serde_json::from_str(&many);
        assert!(long.unwrap_err().to_string().starts_with("TOO_MANY"));
    }

    #[test]
    fn kinds_and_faces_print_their_spelling() {
        assert_eq!(Face::Ambient.to_string(), "ambient");
        assert_eq!(Kind::Theme.as_str(), "theme");
    }
}
