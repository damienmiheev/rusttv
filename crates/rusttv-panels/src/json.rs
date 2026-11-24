//! A first pass over the raw JSON: shape, duplicate keys and nesting depth.
//!
//! serde_json keeps the last of two equal keys without a word, so a package
//! could show one value to one reader and another to the next (PANEL_STUDIO.md
//! § 5.1: duplicate keys are errors). Derived structs also accept a JSON array
//! in place of an object, which would give a package a second spelling. This
//! pass walks the whole document once before typed parsing and refuses all
//! three: the file, its `meta` and every struct in `document` (the document,
//! each list entry, `root`, `stroke`) must be objects. It also counts the
//! objects in `document`, so a large file can't build a large tree.

use crate::error::{Code, PackageError};
use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use std::cell::Cell;
use std::collections::HashSet;
use std::fmt;

/// Checks `text`: an object whose `meta` (if present) is an object, no
/// duplicate keys, nesting no deeper than `max_depth`, structs in
/// `document` as objects and no more than `max_objects` of them. With
/// `package` false (a strings table) the root keys `meta` and `document`
/// mean nothing special.
pub(crate) fn check(
    text: &str,
    max_depth: usize,
    max_objects: usize,
    package: bool,
) -> Result<(), PackageError> {
    let mut reader = serde_json::Deserializer::from_str(text);
    let objects = Cell::new(0);
    Node {
        depth: 0,
        max_depth,
        must_be_object: true,
        in_document: false,
        objects: &objects,
        max_objects,
        package,
    }
    .deserialize(&mut reader)
    .and_then(|()| reader.end())
    .map_err(|error| PackageError::from_json(&error))
}

/// One JSON value at a given depth.
#[derive(Clone, Copy)]
struct Node<'a> {
    depth: usize,
    max_depth: usize,
    /// The file itself, its `meta` and the structs in `document`.
    must_be_object: bool,
    /// Inside `document`.
    in_document: bool,
    /// Objects seen inside `document` so far.
    objects: &'a Cell<usize>,
    max_objects: usize,
    /// `package.json`, whose root `meta` and `document` have rules.
    package: bool,
}

impl Node<'_> {
    fn child<E: de::Error>(self, must_be_object: bool, in_document: bool) -> Result<Self, E> {
        if self.depth >= self.max_depth {
            return Err(E::custom(Code::TooDeep.as_str()));
        }
        Ok(Self {
            depth: self.depth + 1,
            must_be_object,
            in_document,
            ..self
        })
    }

    fn not_object<E: de::Error>(self) -> Result<(), E> {
        if self.must_be_object {
            Err(E::custom(Code::WrongType.as_str()))
        } else {
            Ok(())
        }
    }
}

impl<'de> DeserializeSeed<'de> for Node<'_> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Node<'_> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> Result<(), E> {
        self.not_object()
    }
    fn visit_i64<E: de::Error>(self, _: i64) -> Result<(), E> {
        self.not_object()
    }
    fn visit_u64<E: de::Error>(self, _: u64) -> Result<(), E> {
        self.not_object()
    }
    fn visit_f64<E: de::Error>(self, _: f64) -> Result<(), E> {
        self.not_object()
    }
    fn visit_str<E: de::Error>(self, _: &str) -> Result<(), E> {
        self.not_object()
    }
    fn visit_unit<E: de::Error>(self) -> Result<(), E> {
        // `null` is how an optional struct (`"stroke": null`) says "none";
        // where a struct is required, typed parsing refuses it.
        Ok(())
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        self.not_object()?;
        // Every list in the document is a list of structs.
        let child = self.child(self.in_document, self.in_document)?;
        while seq.next_element_seed(child)?.is_some() {}
        Ok(())
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        if self.in_document {
            self.objects.set(self.objects.get() + 1);
            if self.objects.get() > self.max_objects {
                return Err(de::Error::custom(Code::TooManyNodes.as_str()));
            }
        }
        let mut keys = HashSet::new();
        let at_root = self.package && self.depth == 0;
        while let Some(key) = map.next_key::<String>()? {
            let document = at_root && key == "document";
            let in_document = self.in_document || document;
            let struct_field = self.in_document && (key == "root" || key == "stroke");
            let must_be_object = (at_root && key == "meta") || document || struct_field;
            let child = self.child(must_be_object, in_document)?;
            if !keys.insert(key) {
                return Err(de::Error::custom(Code::DuplicateKey.as_str()));
            }
            map.next_value_seed(child)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Position;

    fn code(text: &str) -> Option<Code> {
        check(text, 4, 3, true).err().map(|e| e.code)
    }

    #[test]
    fn plain_json_passes() {
        assert_eq!(code(r#"{"a":[1,2.5,{"b":null}],"c":"x","d":true}"#), None);
    }

    #[test]
    fn a_duplicate_key_is_refused_with_its_line() {
        let error = check("{\n\"a\": 1,\n\"a\": 2\n}", 4, 3, true).unwrap_err();
        assert_eq!(error.code, Code::DuplicateKey);
        assert!(matches!(error.at, Position::Line { line: 3, .. }));
    }

    #[test]
    fn duplicates_are_found_in_nested_objects_and_through_escapes() {
        assert_eq!(code(r#"{"a":[{"k":1,"k":2}]}"#), Some(Code::DuplicateKey));
        assert_eq!(code(r#"{"a":1,"a":2}"#), Some(Code::DuplicateKey));
    }

    #[test]
    fn equal_keys_in_sibling_objects_are_fine() {
        assert_eq!(code(r#"{"x":[{"k":1},{"k":2}]}"#), None);
    }

    #[test]
    fn the_file_and_its_meta_must_be_objects() {
        assert_eq!(code(r#"[{"meta":{}}]"#), Some(Code::WrongType));
        assert_eq!(code(r#""meta""#), Some(Code::WrongType));
        assert_eq!(code(r#"{"meta":[2,"0.5.0"]}"#), Some(Code::WrongType));
        assert_eq!(code(r#"{"other":[1]}"#), None);
    }

    #[test]
    fn structs_in_the_document_must_be_objects() {
        assert_eq!(code(r#"{"document":[[]]}"#), Some(Code::WrongType));
        assert_eq!(
            code(r#"{"document":{"variants":[["tile"]]}}"#),
            Some(Code::WrongType)
        );
        assert_eq!(
            code(r#"{"document":{"root":["r","frame"]}}"#),
            Some(Code::WrongType)
        );
        assert_eq!(
            code(r#"{"document":{"stroke":[1]}}"#),
            Some(Code::WrongType)
        );
        assert_eq!(code(r#"{"document":{"variants":[{"root":{}}]}}"#), None);
        assert_eq!(code(r#"{"other":{"root":[1]}}"#), None);
        assert_eq!(code(r#"{"document":{"stroke":null}}"#), None);
    }

    #[test]
    fn a_table_has_no_package_keys() {
        assert_eq!(
            check(r#"{"meta":[],"document":{"a":{}}}"#, 4, 0, false),
            Ok(())
        );
    }

    #[test]
    fn objects_in_the_document_are_counted() {
        assert_eq!(code(r#"{"document":{"v":[{},{}]}}"#), None);
        assert_eq!(
            code(r#"{"document":{"v":[{},{},{}]}}"#),
            Some(Code::TooManyNodes)
        );
        assert_eq!(code(r#"{"meta":{"a":{"b":{}}},"x":[{},{},{},{}]}"#), None);
    }

    #[test]
    fn nesting_past_the_limit_is_refused() {
        assert_eq!(code(r#"{"a":[[[1]]]}"#), None);
        assert_eq!(code(r#"{"a":[[[[1]]]]}"#), Some(Code::TooDeep));
    }

    #[test]
    fn broken_json_trailing_garbage_and_huge_numbers_are_not_json() {
        assert_eq!(code(r#"{"a":"#), Some(Code::NotJson));
        assert_eq!(code(r#"{} {}"#), Some(Code::NotJson));
        assert_eq!(code(r#"{"a":1e999}"#), Some(Code::NotJson));
    }

    #[test]
    fn the_error_never_contains_the_packages_text() {
        let error = check(r#"{"secret-key":1,"secret-key":2}"#, 4, 3, true).unwrap_err();
        assert!(!error.to_string().contains("secret"));
    }
}
