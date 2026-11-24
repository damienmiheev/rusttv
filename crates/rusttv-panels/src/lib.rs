//! RustTV panel and theme packages, format 2 (PANEL_STUDIO.md § 5).
//!
//! One reader for the TV, the CLI and the studio: `package.json` (its
//! `meta` and document) and the message tables (`strings/<lang>.json`,
//! [`parse_messages`] and [`check_messages`]). Expressions and the zip
//! reader follow in later slices of step 2a (§ 15).
//!
//! [`parse_package`] runs in a fixed order, each step refusing with a
//! [`Code`] and a [`Position`] and never with the package's own text:
//!
//! 1. size ([`MAX_PACKAGE_JSON`]);
//! 2. a raw pass: valid JSON, the file and its `meta` are objects, no
//!    duplicate keys, depth ≤ [`MAX_JSON_DEPTH`];
//! 3. the version check: `meta.format`, `meta.min_launcher` and
//!    `meta.requires` against [`Capabilities`], so a newer package says
//!    [`Code::PackageTooNew`] rather than failing on fields it doesn't know;
//! 4. strict parsing: unknown fields are errors, every value checked;
//! 5. checks across fields.

/// Deserializes a type from a JSON string through its validating `parse`,
/// which returns a [`Code`]; the code becomes serde's error text, which
/// [`PackageError`] reads back.
macro_rules! deserialize_via {
    ($type:ty, $parse:path) => {
        impl<'de> serde::Deserialize<'de> for $type {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let raw = String::deserialize(deserializer)?;
                $parse(&raw).map_err(|code: crate::Code| serde::de::Error::custom(code.as_str()))
            }
        }
    };
}

mod document;
mod error;
mod json;
mod messages;
mod meta;
mod plural;
mod text;

pub use document::{
    Align, AssetName, Color, Document, Fit, Node, NodeId, NodeKind, Shadow, SizeClass, Stroke,
    Variant, features,
};
pub use error::{Code, PackageError, PackageFile, Position};
pub use messages::{
    Arg, Message, MessageId, Messages, Piece, PlaceholderName, Template, check_messages,
    fallback_chain, lookup, parse_messages,
};
pub use meta::{
    Capabilities, FORMAT, Face, HomepageUrl, Kind, List, MAX_LIST, Meta, Name, PackageId,
    RESERVED_NAMES, Version, limits,
};
pub use plural::{PluralCategory, plural_categories, plural_category};
pub use text::{LANGUAGES, Language, Text, clean_text};

use serde::Deserialize;

/// Largest `package.json`, in bytes.
pub const MAX_PACKAGE_JSON: usize = 2 << 20;

/// Deepest nesting of JSON objects and arrays.
pub const MAX_JSON_DEPTH: usize = 32;

/// A parsed `package.json`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Package {
    /// Who and what it is.
    pub meta: Meta,
    /// A panel's faces; `None` for a theme.
    pub document: Option<Document>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPackage {
    meta: meta::RawMeta,
    #[serde(default)]
    document: Option<Document>,
}

/// Reads `package.json` for a launcher that can do `can`.
///
/// # Errors
///
/// A [`PackageError`] for the first problem found, in the order described in
/// the crate documentation.
pub fn parse_package(text: &str, can: &Capabilities) -> Result<Package, PackageError> {
    if text.len() > MAX_PACKAGE_JSON {
        return Err(PackageError::new(Code::TooLarge, Position::File));
    }
    json::check(text, MAX_JSON_DEPTH, limits::DOCUMENT_OBJECTS, true)?;
    meta::check_head(text, can)?;
    let raw: RawPackage =
        serde_json::from_str(text).map_err(|e| PackageError::from_typed_json(&e))?;
    let meta = meta::check_meta(raw.meta)?;
    let at_document = Position::field("document");
    match (meta.kind, &raw.document) {
        (Kind::Panel, Some(document)) => document::check(document, &meta)?,
        (Kind::Panel, None) => return Err(PackageError::new(Code::MissingField, at_document)),
        (Kind::Theme, Some(_)) => return Err(PackageError::new(Code::NotForKind, at_document)),
        (Kind::Theme, None) => {}
    }
    Ok(Package {
        meta,
        document: raw.document,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn can() -> Capabilities {
        Capabilities::new(Version::new(0, 5, 0), ["shadow", "rotation"])
    }

    const GOOD: &str = r##"{
      "meta": {
        "format": 2,
        "min_launcher": "0.5.0",
        "requires": ["shadow"],
        "id": "user.kyiv-clock",
        "kind": "panel",
        "version": "1.0.0",
        "default_language": "en",
        "name": { "en": "Kyiv clock", "uk": "Годинник Київ" },
        "description": { "en": "The time in Kyiv." },
        "author": "Somebody",
        "licence": "MIT",
        "homepage": "https://example.org/clock",
        "faces": ["tile", "page"],
        "sources": ["clock"],
        "actions": ["open-page"]
      },
      "document": {
        "variants": [
          { "face": "tile", "root": { "id": "root", "type": "frame", "w": 250, "h": 141, "fill": "#1e2633", "radius": 16,
              "shadows": [{ "y": 4, "blur": 12, "color": "#00000090" }],
              "children": [
                { "id": "time", "type": "text", "x": 16, "y": 16, "w": 218, "h": 40, "text": "12:00", "size": 32, "weight": 700, "color": "#ffffff" }
              ] } },
          { "face": "page", "root": { "id": "root", "type": "frame", "w": 1920, "h": 1080, "fill": "#101418" } }
        ]
      }
    }"##;

    /// `GOOD` with `from` replaced by `to`; fails if `from` isn't there, so a
    /// test can't pass by changing nothing.
    fn edit(from: &str, to: &str) -> String {
        assert!(GOOD.contains(from), "the test edits nothing: {from}");
        GOOD.replacen(from, to, 1)
    }

    fn error(text: &str) -> PackageError {
        parse_package(text, &can()).unwrap_err()
    }

    #[test]
    fn a_good_package_parses() {
        let meta = parse_package(GOOD, &can()).unwrap().meta;
        assert_eq!(meta.id.as_str(), "user.kyiv-clock");
        assert_eq!(meta.kind, Kind::Panel);
        assert_eq!(
            meta.name[&Language::parse("uk").unwrap()].as_str(),
            "Годинник Київ"
        );
        assert_eq!(*meta.faces, [Face::Tile, Face::Page]);
    }

    #[test]
    fn a_panel_needs_a_document_and_a_theme_has_none() {
        let meta_end = GOOD
            .find(r#""document""#)
            .and_then(|at| GOOD[..at].rfind(','));
        let without = GOOD[..meta_end.unwrap()].to_owned() + "}";
        assert_eq!(error(&without).code, Code::MissingField);
        assert_eq!(
            error(&edit(r#""kind": "panel""#, r#""kind": "theme""#)).code,
            Code::NotForKind
        );
    }

    #[test]
    fn a_newer_format_is_too_new_even_with_fields_this_reader_doesnt_know() {
        let found = error(&edit(r#""format": 2"#, r#""format": 3, "holograms": true"#));
        assert_eq!(
            (found.code, found.needs),
            (Code::PackageTooNew, Some(Version::new(0, 5, 0)))
        );
    }

    #[test]
    fn a_newer_min_launcher_is_too_new_and_says_which() {
        let found = error(&edit(
            r#""min_launcher": "0.5.0""#,
            r#""min_launcher": "0.7.1""#,
        ));
        assert_eq!(
            (found.code, found.needs),
            (Code::PackageTooNew, Some(Version::new(0, 7, 1)))
        );
        assert!(found.to_string().ends_with("(needs RustTV 0.7.1)"));
    }

    #[test]
    fn the_version_check_refuses_odd_heads() {
        let cases = [
            (r#""format": 2"#, r#""format": 1"#, Code::FormatUnsupported),
            (r#""format": 2"#, r#""format": 2.0"#, Code::WrongType),
            (r#""format": 2"#, r#""format": "2""#, Code::WrongType),
            (r#""format": 2"#, r#""format": -1"#, Code::WrongType),
            (r#""min_launcher": "0.5.0","#, "", Code::MissingField),
            (
                r#""min_launcher": "0.5.0""#,
                r#""min_launcher": "0.5""#,
                Code::BadVersion,
            ),
            (
                r#""min_launcher": "0.5.0""#,
                r#""min_launcher": 5"#,
                Code::BadVersion,
            ),
            (
                r#""requires": ["shadow"]"#,
                r#""requires": "shadow""#,
                Code::WrongType,
            ),
            (
                r#""requires": ["shadow"]"#,
                r#""requires": [7]"#,
                Code::WrongType,
            ),
        ];
        for (from, to, want) in cases {
            assert_eq!(error(&edit(from, to)).code, want, "{to}");
        }
    }

    #[test]
    fn a_missing_feature_is_named_by_its_index() {
        let found = error(&edit(r#"["shadow"]"#, r#"["shadow","blur"]"#));
        assert_eq!(found.code, Code::RequiresFeature);
        assert_eq!(
            found.at,
            Position::Field {
                path: "meta.requires",
                index: Some(1)
            }
        );
    }

    #[test]
    fn unknown_fields_duplicate_keys_and_other_spellings_are_refused() {
        assert_eq!(
            error(&edit(
                r#""kind": "panel","#,
                r#""kind": "panel", "extra": 1,"#
            ))
            .code,
            Code::UnknownField
        );
        assert_eq!(
            error(&edit(
                r#""kind": "panel","#,
                r#""kind": "panel", "kind": "theme","#
            ))
            .code,
            Code::DuplicateKey
        );
        assert_eq!(error(&format!("[{GOOD}]")).code, Code::WrongType);
        assert_eq!(
            error(r#"{"meta": {"format": 2}, "document": {}}"#).code,
            Code::MissingField
        );
    }

    #[test]
    fn field_checks_report_their_own_codes() {
        let cases = [
            (r#""user.kyiv-clock""#, r#""user.about""#, Code::ReservedId),
            (r#""user.kyiv-clock""#, r#""kyiv""#, Code::BadId),
            (r#""1.0.0""#, r#""1.0""#, Code::BadVersion),
            (
                r#""default_language": "en""#,
                r#""default_language": "klingon""#,
                Code::BadLanguage,
            ),
            (
                r#""uk": "Годинник Київ""#,
                r#""xx": "x""#,
                Code::BadLanguage,
            ),
            (
                r#""Kyiv clock""#,
                r#""a name far longer than twenty-four characters""#,
                Code::TextTooLong,
            ),
            (r#""Kyiv clock""#, "\"\\u202e\\u200d\"", Code::EmptyText),
            (
                r#""https://example.org/clock""#,
                r#""http://example.org""#,
                Code::BadUrl,
            ),
            (
                r#"["tile", "page"]"#,
                r#"["tile", "tile"]"#,
                Code::Duplicate,
            ),
            (
                r#"["tile", "page"]"#,
                r#"["tile", "fridge"]"#,
                Code::BadValue,
            ),
            (r#"["clock"]"#, r#"["Clock"]"#, Code::BadName),
            (r#""kind": "panel""#, r#""kind": 7"#, Code::WrongType),
            (
                r#""author": "Somebody""#,
                r#""author": ["Somebody"]"#,
                Code::WrongType,
            ),
        ];
        for (from, to, want) in cases {
            assert_eq!(error(&edit(from, to)).code, want, "{to}");
        }
        let many = (0..=MAX_LIST)
            .map(|i| format!("\"s{i}\""))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(
            error(&edit(r#"["clock"]"#, &format!("[{many}]"))).code,
            Code::TooMany
        );
    }

    #[test]
    fn the_name_needs_the_default_language() {
        assert_eq!(
            error(&edit(
                r#""default_language": "en""#,
                r#""default_language": "de""#
            ))
            .code,
            Code::NoDefaultName
        );
    }

    #[test]
    fn missing_fields_broken_json_deep_nesting_and_oversized_files_are_refused() {
        assert_eq!(
            error(&edit(r#""kind": "panel","#, "")).code,
            Code::MissingField
        );
        assert_eq!(error(&GOOD[..GOOD.len() - 3]).code, Code::NotJson);
        let deep = format!(
            "{}{}",
            "[".repeat(MAX_JSON_DEPTH + 1),
            "]".repeat(MAX_JSON_DEPTH + 1)
        );
        assert_eq!(
            error(&edit(
                r#""licence": "MIT""#,
                &format!(r#""licence": {deep}"#)
            ))
            .code,
            Code::TooDeep
        );
        let huge = format!(
            "{{\"meta\":{{}},\"pad\":\"{}\"}}",
            "x".repeat(MAX_PACKAGE_JSON)
        );
        assert_eq!(error(&huge).code, Code::TooLarge);
    }

    #[test]
    fn errors_never_quote_the_package() {
        let found = error(&edit(
            r#""kind": "panel","#,
            r#""kind": "panel", "APPROVE-NOW": 1,"#,
        ));
        assert_eq!(found.code, Code::UnknownField);
        assert!(!format!("{found} {found:?}").contains("APPROVE"));
    }
}
