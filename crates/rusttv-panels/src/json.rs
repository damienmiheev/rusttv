//! A first pass over the raw JSON: shape, duplicate keys and nesting depth.
//!
//! serde_json keeps the last of two equal keys without a word, so a package
//! could show one value to one reader and another to the next (PANEL_STUDIO.md
//! § 5.1: duplicate keys are errors). Derived structs also accept a JSON array
//! in place of an object, which would give a package a second spelling. This
//! pass walks the whole document once before typed parsing and refuses all
//! three: the file and its `meta` must be objects.

use crate::error::{Code, PackageError};
use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use std::collections::HashSet;
use std::fmt;

/// Checks `text`: an object whose `meta` (if present) is an object, no
/// duplicate keys, nesting no deeper than `max_depth`.
pub(crate) fn check(text: &str, max_depth: usize) -> Result<(), PackageError> {
    let mut reader = serde_json::Deserializer::from_str(text);
    Node {
        depth: 0,
        max_depth,
        must_be_object: true,
    }
    .deserialize(&mut reader)
    .and_then(|()| reader.end())
    .map_err(|error| PackageError::from_json(&error))
}

/// One JSON value at a given depth.
#[derive(Clone, Copy)]
struct Node {
    depth: usize,
    max_depth: usize,
    /// The file itself and its `meta`.
    must_be_object: bool,
}

impl Node {
    fn child<E: de::Error>(self, must_be_object: bool) -> Result<Self, E> {
        if self.depth >= self.max_depth {
            return Err(E::custom(Code::TooDeep.as_str()));
        }
        Ok(Self {
            depth: self.depth + 1,
            max_depth: self.max_depth,
            must_be_object,
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

impl<'de> DeserializeSeed<'de> for Node {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Node {
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
        self.not_object()
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        self.not_object()?;
        let child = self.child(false)?;
        while seq.next_element_seed(child)?.is_some() {}
        Ok(())
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let mut keys = HashSet::new();
        let at_root = self.depth == 0;
        while let Some(key) = map.next_key::<String>()? {
            let child = self.child(at_root && key == "meta")?;
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
        check(text, 4).err().map(|e| e.code)
    }

    #[test]
    fn plain_json_passes() {
        assert_eq!(code(r#"{"a":[1,2.5,{"b":null}],"c":"x","d":true}"#), None);
    }

    #[test]
    fn a_duplicate_key_is_refused_with_its_line() {
        let error = check("{\n\"a\": 1,\n\"a\": 2\n}", 4).unwrap_err();
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
        let error = check(r#"{"secret-key":1,"secret-key":2}"#, 4).unwrap_err();
        assert!(!error.to_string().contains("secret"));
    }
}
