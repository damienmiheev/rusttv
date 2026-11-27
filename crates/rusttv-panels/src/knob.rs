//! Knobs (PANEL_STUDIO.md § 7): typed settings a package offers, which the
//! person sets on the TV, the phone or in the studio, and bindings read.
//!
//! ```json
//! "knobs": [
//!   {"name": "seconds", "type": "toggle", "label": "knob.seconds", "default": false},
//!   {"name": "size", "type": "number", "label": "knob.size", "min": 1, "max": 5, "step": 1, "default": 3},
//!   {"name": "style", "type": "choice", "label": "knob.style", "default": "round",
//!    "options": [{"id": "round", "label": "knob.style.round"}, {"id": "square", "label": "knob.style.square"}]},
//!   {"name": "accent", "type": "color", "label": "knob.accent", "default": "#3478f6",
//!    "group": "knob.look", "visible_if": {"knob": "seconds"}},
//!   {"name": "greeting", "type": "text", "label": "knob.greeting", "max_length": 40, "default": "Hello"}
//! ]
//! ```
//!
//! Labels are message ids, so they are translated like every other package
//! text. `visible_if` is a yes/no expression that reads only knobs declared
//! before it, and no sources, so visibility can't go round in a circle.
//! Texts keep their spaces at the ends, like text literals in expressions.

use crate::Color;
use crate::error::{Code, PackageError, Position};
use crate::expr::{self, Expr, Type, Value};
use crate::messages::MessageId;
use crate::meta::{Name, limits};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

word_enum! {
    /// A knob's type as written.
    KnobType {
        /// Yes or no.
        Toggle => "toggle",
        /// A number in a range.
        Number => "number",
        /// One of a fixed list of options.
        Choice => "choice",
        /// A colour.
        Color => "color",
        /// A short text.
        Text => "text",
    }
}

/// One option of a choice knob.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct KnobOption {
    /// What the knob's value is when this option is picked.
    pub id: Name,
    /// Its label.
    pub label: MessageId,
}

/// What a knob holds, with its rules and default.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum KnobKind {
    /// Yes or no.
    #[non_exhaustive]
    Toggle {
        /// Its default.
        default: bool,
    },
    /// A number from `min` to `max`.
    #[non_exhaustive]
    Number {
        /// The smallest value.
        min: f64,
        /// The largest value.
        max: f64,
        /// How far one press moves it; a hint for the editor.
        step: Option<f64>,
        /// Its default.
        default: f64,
    },
    /// One of `options`; its value is the option's id, as text.
    #[non_exhaustive]
    Choice {
        /// The options, at least two.
        options: Vec<KnobOption>,
        /// The id of the default option.
        default: Name,
    },
    /// A colour.
    #[non_exhaustive]
    Color {
        /// Its default.
        default: Color,
    },
    /// A short text, cleaned like every package text.
    #[non_exhaustive]
    Text {
        /// Its longest length in characters.
        max_length: usize,
        /// Its default (may be empty).
        default: String,
    },
}

/// A knob: a typed setting the person chooses.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Knob {
    /// Its name, which bindings read as `{"knob": name}`.
    pub name: Name,
    /// Its label.
    pub label: MessageId,
    /// The heading it is shown under, if any.
    pub group: Option<MessageId>,
    /// When it is shown; always when `None`.
    pub visible_if: Option<Expr>,
    /// What it holds.
    pub kind: KnobKind,
}

impl Knob {
    /// The type of its value in expressions.
    pub fn value_type(&self) -> Type {
        match self.kind {
            KnobKind::Toggle { .. } => Type::Bool,
            KnobKind::Number { .. } => Type::Number,
            KnobKind::Choice { .. } | KnobKind::Text { .. } => Type::Text,
            KnobKind::Color { .. } => Type::Color,
        }
    }

    /// Its default value.
    pub fn default_value(&self) -> Value {
        match &self.kind {
            KnobKind::Toggle { default } => Value::Bool(*default),
            KnobKind::Number { default, .. } => Value::Number(*default),
            KnobKind::Choice { default, .. } => Value::Text(default.as_str().to_owned()),
            KnobKind::Color { default } => Value::Color(*default),
            KnobKind::Text { default, .. } => Value::Text(default.clone()),
        }
    }

    /// Whether `value` is one this knob can hold: the right type, in range,
    /// one of the options, or a text that cleaning leaves as it is and
    /// that has at most `max_length` characters (`char`s). The step is only
    /// a hint for the editor, so any number in range is accepted.
    pub fn accepts(&self, value: &Value) -> bool {
        match (&self.kind, value) {
            (KnobKind::Toggle { .. }, Value::Bool(_))
            | (KnobKind::Color { .. }, Value::Color(_)) => true,
            (KnobKind::Number { min, max, .. }, Value::Number(n)) => (*min..=*max).contains(n),
            (KnobKind::Choice { options, .. }, Value::Text(text)) => {
                options.iter().any(|option| option.id.as_str() == text)
            }
            (KnobKind::Text { max_length, .. }, Value::Text(text)) => {
                text.chars().count() <= *max_length
                    && crate::text::clean_keeping_ends(text) == *text
            }
            _ => false,
        }
    }

    /// The value to use: `stored` if this knob [accepts](Self::accepts)
    /// it, else the default (a value from before an update that changed
    /// the knob, or none yet).
    pub fn resolve(&self, stored: Option<&Value>) -> Value {
        match stored {
            // `+ 0.0` turns -0 into 0, as for literals.
            Some(Value::Number(n)) if self.accepts(&Value::Number(*n)) => Value::Number(n + 0.0),
            Some(value) if self.accepts(value) => value.clone(),
            _ => self.default_value(),
        }
    }

    /// Whether changing it is a one-click look setting (§ 7): it brings in
    /// nothing new. A text knob needs the code on the TV and is never set
    /// from the phone.
    pub fn one_click(&self) -> bool {
        !matches!(self.kind, KnobKind::Text { .. })
    }
}

impl fmt::Display for Knob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name.as_str())
    }
}

/// A knob as written; [`check`] turns it into a [`Knob`].
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawKnob {
    name: Name,
    #[serde(rename = "type")]
    kind: KnobType,
    label: MessageId,
    #[serde(default)]
    group: Option<MessageId>,
    #[serde(default)]
    visible_if: Option<Expr>,
    default: Expr,
    #[serde(default)]
    min: Option<Expr>,
    #[serde(default)]
    max: Option<Expr>,
    #[serde(default)]
    step: Option<Expr>,
    #[serde(default)]
    options: Option<Vec<KnobOption>>,
    #[serde(default)]
    max_length: Option<Expr>,
}

/// A number literal, or [`Code::WrongType`].
fn number(expr: &Expr) -> Result<f64, Code> {
    match expr {
        // `+ 0.0` turns -0 into 0, so there is one spelling of zero.
        Expr::Number(n) => Ok(*n + 0.0),
        _ => Err(Code::WrongType),
    }
}

/// One knob's own rules: only its type's fields, a default of its type, a
/// range that holds it, options that are distinct.
fn check_one(raw: RawKnob) -> Result<Knob, Code> {
    use KnobType as T;
    let number_only = raw.min.is_some() || raw.max.is_some() || raw.step.is_some();
    if (number_only && raw.kind != T::Number)
        || (raw.options.is_some() && raw.kind != T::Choice)
        || (raw.max_length.is_some() && raw.kind != T::Text)
    {
        return Err(Code::NotForKind);
    }
    let kind = match raw.kind {
        T::Toggle => match raw.default {
            Expr::Bool(default) => KnobKind::Toggle { default },
            _ => return Err(Code::WrongType),
        },
        T::Number => {
            let (Some(min), Some(max)) = (&raw.min, &raw.max) else {
                return Err(Code::MissingProperty);
            };
            let (min, max, default) = (number(min)?, number(max)?, number(&raw.default)?);
            let step = raw.step.as_ref().map(number).transpose()?;
            let steps = |s: f64| (max - min) / s;
            if min >= max
                || step.is_some_and(|s| {
                    s <= 0.0 || s > max - min || steps(s) > limits::KNOB_STEPS as f64
                })
            {
                return Err(Code::BadKnob);
            }
            if !(min..=max).contains(&default) {
                return Err(Code::OutOfRange);
            }
            KnobKind::Number {
                min,
                max,
                step,
                default,
            }
        }
        T::Choice => {
            let options = raw.options.ok_or(Code::MissingProperty)?;
            if options.len() > limits::KNOB_OPTIONS {
                return Err(Code::TooMany);
            }
            if options.len() < 2 {
                return Err(Code::BadKnob);
            }
            let ids: BTreeSet<&str> = options.iter().map(|o| o.id.as_str()).collect();
            if ids.len() != options.len() {
                return Err(Code::Duplicate);
            }
            let Expr::Str(default) = &raw.default else {
                return Err(Code::WrongType);
            };
            if !ids.contains(default.as_str()) {
                return Err(Code::BadKnob);
            }
            let default = Name::parse(default)?;
            KnobKind::Choice { options, default }
        }
        T::Color => match &raw.default {
            Expr::Str(text) => KnobKind::Color {
                default: Color::parse(text)?,
            },
            _ => return Err(Code::WrongType),
        },
        T::Text => {
            let max_length = match &raw.max_length {
                None => limits::KNOB_TEXT,
                Some(expr) => {
                    let n = number(expr)?;
                    if n.fract() != 0.0 || !(1.0..=limits::KNOB_TEXT as f64).contains(&n) {
                        return Err(Code::OutOfRange);
                    }
                    // Exact: a whole number from 1 to KNOB_TEXT.
                    n as usize
                }
            };
            let Expr::Str(default) = raw.default else {
                return Err(Code::WrongType);
            };
            if default.chars().count() > max_length {
                return Err(Code::TextTooLong);
            }
            KnobKind::Text {
                max_length,
                default,
            }
        }
    };
    Ok(Knob {
        name: raw.name,
        label: raw.label,
        group: raw.group,
        visible_if: raw.visible_if,
        kind,
    })
}

/// Checks `meta.knobs`: at most [`limits::KNOBS`], distinct names, each
/// knob's own rules, and each `visible_if` a yes/no expression over knobs
/// declared before it.
pub(crate) fn check(raw: Vec<RawKnob>) -> Result<Vec<Knob>, PackageError> {
    let at = |index| Position::Field {
        path: "meta.knobs",
        index: Some(index),
    };
    if raw.len() > limits::KNOBS {
        return Err(PackageError::new(
            Code::TooMany,
            Position::field("meta.knobs"),
        ));
    }
    let mut knobs: Vec<Knob> = Vec::with_capacity(raw.len());
    let mut earlier: BTreeMap<&str, Type> = BTreeMap::new();
    let no_sources = BTreeSet::new();
    for (index, raw) in raw.into_iter().enumerate() {
        let knob = check_one(raw).map_err(|code| PackageError::new(code, at(index)))?;
        if knobs.iter().any(|k| k.name == knob.name) {
            return Err(PackageError::new(Code::Duplicate, at(index)));
        }
        if let Some(condition) = &knob.visible_if {
            expr::check(condition, Type::Bool, &no_sources, &earlier)
                .map_err(|code| PackageError::new(code, at(index)))?;
        }
        knobs.push(knob);
        // Re-borrowed each round, so a knob sees only the ones before it.
        earlier = knobs
            .iter()
            .map(|k| (k.name.as_str(), k.value_type()))
            .collect();
    }
    Ok(knobs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Result<Vec<Knob>, Code> {
        let raw: Vec<RawKnob> =
            serde_json::from_str(json).map_err(|e| PackageError::from_typed_json(&e).code)?;
        check(raw).map_err(|e| e.code)
    }

    fn one(fields: &str) -> Result<Knob, Code> {
        parse(&format!(r#"[{{"name": "k", "label": "l", {fields}}}]"#)).map(|mut k| k.remove(0))
    }

    #[test]
    fn each_type_reads_with_its_default() {
        let cases = [
            (r#""type": "toggle", "default": true"#, Value::Bool(true)),
            (
                r#""type": "number", "min": 0, "max": 10, "step": 0.5, "default": 2.5"#,
                Value::Number(2.5),
            ),
            (
                r#""type": "choice", "default": "b", "options": [{"id": "a", "label": "x"}, {"id": "b", "label": "y"}]"#,
                Value::Text("b".into()),
            ),
            (
                r##""type": "color", "default": "#102030""##,
                Value::Color(Color::parse("#102030").unwrap()),
            ),
            (
                r#""type": "text", "max_length": 5, "default": "hi""#,
                Value::Text("hi".into()),
            ),
            (
                r#""type": "text", "default": """#,
                Value::Text(String::new()),
            ),
        ];
        for (fields, want) in cases {
            let knob = one(fields).unwrap();
            assert_eq!(knob.default_value(), want, "{fields}");
            assert!(knob.accepts(&want), "{fields}");
        }
    }

    #[test]
    fn bad_knobs_get_their_codes() {
        let cases = [
            (r#""type": "dial", "default": 1"#, Code::BadValue),
            (r#""type": "toggle", "default": 1"#, Code::WrongType),
            (
                r#""type": "toggle", "default": true, "min": 0"#,
                Code::NotForKind,
            ),
            (
                r#""type": "number", "max": 1, "default": 0"#,
                Code::MissingProperty,
            ),
            (
                r#""type": "number", "min": 1, "max": 1, "default": 1"#,
                Code::BadKnob,
            ),
            (
                r#""type": "number", "min": 0, "max": 1, "step": 0, "default": 0"#,
                Code::BadKnob,
            ),
            (
                r#""type": "number", "min": 0, "max": 1, "step": 2, "default": 0"#,
                Code::BadKnob,
            ),
            (
                r#""type": "number", "min": 0, "max": 1, "default": 2"#,
                Code::OutOfRange,
            ),
            (
                r#""type": "number", "min": "0", "max": 1, "default": 0"#,
                Code::WrongType,
            ),
            (r#""type": "choice", "default": "a""#, Code::MissingProperty),
            (
                r#""type": "choice", "default": "a", "options": [{"id": "a", "label": "x"}]"#,
                Code::BadKnob,
            ),
            (
                r#""type": "choice", "default": "c", "options": [{"id": "a", "label": "x"}, {"id": "b", "label": "y"}]"#,
                Code::BadKnob,
            ),
            (
                r#""type": "choice", "default": "a", "options": [{"id": "a", "label": "x"}, {"id": "a", "label": "y"}]"#,
                Code::Duplicate,
            ),
            (
                r#""type": "choice", "default": "a", "options": [{"id": "A", "label": "x"}, {"id": "b", "label": "y"}]"#,
                Code::BadName,
            ),
            (r#""type": "color", "default": "red""#, Code::BadColor),
            (
                r#""type": "text", "max_length": 2, "default": "abc""#,
                Code::TextTooLong,
            ),
            (
                r#""type": "text", "max_length": 201, "default": """#,
                Code::OutOfRange,
            ),
            (
                r#""type": "text", "max_length": 1.5, "default": """#,
                Code::OutOfRange,
            ),
            (r#""type": "text", "default": false"#, Code::WrongType),
            (
                r#""type": "toggle", "default": true, "colour": 1"#,
                Code::UnknownField,
            ),
        ];
        for (fields, want) in cases {
            assert_eq!(one(fields).err(), Some(want), "{fields}");
        }
        assert_eq!(
            parse(r#"[{"name": "K", "type": "toggle", "label": "l", "default": true}]"#).err(),
            Some(Code::BadName)
        );
        assert_eq!(
            parse(r#"[{"name": "k", "type": "toggle", "label": "L", "default": true}]"#).err(),
            Some(Code::BadMessageId)
        );
    }

    #[test]
    fn names_are_distinct_and_the_list_is_capped() {
        let knob = |name: &str| {
            format!(r#"{{"name": "{name}", "type": "toggle", "label": "l", "default": true}}"#)
        };
        assert_eq!(
            parse(&format!("[{},{}]", knob("a"), knob("a"))).err(),
            Some(Code::Duplicate)
        );
        let many: Vec<String> = (0..=limits::KNOBS)
            .map(|i| knob(&format!("k{i}")))
            .collect();
        assert_eq!(
            parse(&format!("[{}]", many.join(","))).err(),
            Some(Code::TooMany)
        );
        let options: Vec<String> = (0..=limits::KNOB_OPTIONS)
            .map(|i| format!(r#"{{"id": "o{i}", "label": "l"}}"#))
            .collect();
        assert_eq!(
            one(&format!(
                r#""type": "choice", "default": "o0", "options": [{}]"#,
                options.join(",")
            ))
            .err(),
            Some(Code::TooMany)
        );
    }

    #[test]
    fn visible_if_reads_only_earlier_knobs_as_yes_or_no() {
        let toggle = r#"{"name": "on", "type": "toggle", "label": "l", "default": true}"#;
        let shown = |condition: &str| {
            format!(
                r#"{{"name": "size", "type": "number", "label": "l", "min": 0, "max": 9, "default": 1, "visible_if": {condition}}}"#
            )
        };
        assert!(parse(&format!(r#"[{toggle},{}]"#, shown(r#"{"knob": "on"}"#))).is_ok());
        let cases = [
            (
                format!(r#"[{},{toggle}]"#, shown(r#"{"knob": "on"}"#)),
                Code::UnknownKnob,
            ),
            (
                format!(r#"[{}]"#, shown(r#"{"knob": "size"}"#)),
                Code::UnknownKnob,
            ),
            (
                format!(r#"[{toggle},{}]"#, shown(r#"{"knob": "nope"}"#)),
                Code::UnknownKnob,
            ),
            (
                format!(r#"[{toggle},{}]"#, shown(r#"{"source": "clock.hour24"}"#)),
                Code::UndeclaredSource,
            ),
            (
                format!(r#"[{toggle},{}]"#, shown(r#""yes""#)),
                Code::TypeMismatch,
            ),
        ];
        for (json, want) in cases {
            assert_eq!(parse(&json).err(), Some(want), "{json}");
        }
    }

    #[test]
    fn edges_of_each_rule() {
        let ok = [
            r#""type": "number", "min": 1, "max": 5, "default": 1"#,
            r#""type": "number", "min": 1, "max": 5, "default": 5"#,
            r#""type": "number", "min": 0, "max": 1, "step": 1, "default": 0"#,
            r#""type": "number", "min": 0, "max": 10000, "step": 1, "default": 0"#,
            r#""type": "text", "max_length": 1, "default": "a""#,
            r#""type": "text", "max_length": 200, "default": """#,
            r#""type": "text", "max_length": 3, "default": "ééé""#,
            r##""type": "color", "default": "#10203040""##,
            r#""type": "toggle", "default": false, "visible_if": true"#,
        ];
        for fields in ok {
            assert!(one(fields).is_ok(), "{fields}");
        }
        let long = "a".repeat(limits::KNOB_TEXT + 1);
        let bad = [
            (r#""type": "number", "min": 1, "max": 5, "default": 0"#.to_owned(), Code::OutOfRange),
            (r#""type": "number", "min": 2, "max": 1, "default": 1"#.to_owned(), Code::BadKnob),
            (r#""type": "number", "min": 0, "max": 1, "step": -1, "default": 0"#.to_owned(), Code::BadKnob),
            (r#""type": "number", "min": 0, "max": 1, "step": "1", "default": 0"#.to_owned(), Code::WrongType),
            (r#""type": "number", "min": 0, "max": 10001, "step": 1, "default": 0"#.to_owned(), Code::BadKnob),
            (r#""type": "number", "min": 0, "max": 1, "step": 5e-324, "default": 0"#.to_owned(), Code::BadKnob),
            (r#""type": "number", "min": 0, "default": 0"#.to_owned(), Code::MissingProperty),
            (r#""type": "toggle", "default": true, "options": []"#.to_owned(), Code::NotForKind),
            (r#""type": "number", "min": 0, "max": 1, "default": 0, "max_length": 3"#.to_owned(), Code::NotForKind),
            (r#""type": "text", "default": "", "step": 1"#.to_owned(), Code::NotForKind),
            (r#""type": "choice", "default": "a", "max": 1, "options": []"#.to_owned(), Code::NotForKind),
            (r#""type": "choice", "default": 1, "options": [{"id": "a", "label": "x"}, {"id": "b", "label": "y"}]"#.to_owned(), Code::WrongType),
            (r#""type": "text", "max_length": 0, "default": """#.to_owned(), Code::OutOfRange),
            (r#""type": "text", "max_length": "5", "default": """#.to_owned(), Code::WrongType),
            (format!(r#""type": "text", "default": "{long}""#), Code::TextTooLong),
            (r#""type": "color", "default": 5"#.to_owned(), Code::WrongType),
            (r##""type": "color", "default": "#abc""##.to_owned(), Code::BadColor),
            (r#""type": "toggle""#.to_owned(), Code::MissingField),
        ];
        for (fields, want) in bad {
            assert_eq!(one(&fields).err(), Some(want), "{fields}");
        }
        assert_eq!(
            parse(r#"[{"name": "k", "label": "l", "default": true}]"#).err(),
            Some(Code::MissingField)
        );
        let zero = one(r#""type": "number", "min": -1, "max": 1, "default": -0.0"#).unwrap();
        assert!(
            matches!(zero.kind, KnobKind::Number { default, .. } if default.is_sign_positive())
        );
        let cleaned = one(r#""type": "text", "default": " a\u202eb ""#).unwrap();
        assert_eq!(cleaned.default_value(), Value::Text(" ab ".into()));
    }

    #[test]
    fn up_to_the_limits_is_fine_and_errors_name_their_knob() {
        let knob = |name: &str| {
            format!(r#"{{"name": "{name}", "type": "toggle", "label": "l", "default": true}}"#)
        };
        let all: Vec<String> = (0..limits::KNOBS).map(|i| knob(&format!("k{i}"))).collect();
        assert!(parse(&format!("[{}]", all.join(","))).is_ok());
        let options: Vec<String> = (0..limits::KNOB_OPTIONS)
            .map(|i| format!(r#"{{"id": "o{i}", "label": "l"}}"#))
            .collect();
        assert!(
            one(&format!(
                r#""type": "choice", "default": "o0", "options": [{}]"#,
                options.join(",")
            ))
            .is_ok()
        );
        let at = |json: &str| {
            let raw: Vec<RawKnob> = serde_json::from_str(json).unwrap();
            check(raw).unwrap_err()
        };
        let first = knob("a");
        let range =
            r#"{"name": "n", "type": "number", "label": "l", "min": 1, "max": 1, "default": 1}"#;
        let later = r#"{"name": "s", "type": "toggle", "label": "l", "default": true, "visible_if": {"knob": "z"}}"#;
        for (json, code) in [
            (format!("[{first},{}]", knob("a")), Code::Duplicate),
            (format!("[{first},{range}]"), Code::BadKnob),
            (format!("[{first},{later}]"), Code::UnknownKnob),
        ] {
            let error = at(&json);
            assert_eq!(
                (error.code, error.at),
                (
                    code,
                    Position::Field {
                        path: "meta.knobs",
                        index: Some(1)
                    }
                ),
                "{json}"
            );
        }
        let many: Vec<String> = (0..=limits::KNOBS)
            .map(|i| knob(&format!("k{i}")))
            .collect();
        assert_eq!(
            at(&format!("[{}]", many.join(","))).at,
            Position::field("meta.knobs")
        );
    }

    #[test]
    fn visible_if_sees_the_types_of_earlier_knobs() {
        let knobs = r#"{"name": "on", "type": "toggle", "label": "l", "default": true},
            {"name": "size", "type": "number", "label": "l", "min": 0, "max": 9, "default": 1},
            {"name": "style", "type": "choice", "label": "l", "default": "a",
             "options": [{"id": "a", "label": "l"}, {"id": "b", "label": "l"}]}"#;
        let with = |condition: &str| {
            parse(&format!(
                r#"[{knobs}, {{"name": "x", "type": "toggle", "label": "l", "default": true, "visible_if": {condition}}}]"#
            ))
            .err()
        };
        assert_eq!(
            with(
                r#"{"call": "and", "args": [{"knob": "on"}, {"call": "eq", "args": [{"knob": "style"}, "b"]}]}"#
            ),
            None
        );
        assert_eq!(
            with(r#"{"call": "gt", "args": [{"knob": "size"}, 3]}"#),
            None
        );
        assert_eq!(with(r#"{"knob": "size"}"#), Some(Code::TypeMismatch));
    }

    #[test]
    fn accepts_checks_range_options_and_length() {
        let number = one(r#""type": "number", "min": 0, "max": 10, "default": 1"#).unwrap();
        assert!(!number.accepts(&Value::Number(11.0)));
        assert!(!number.accepts(&Value::Bool(true)));
        let choice = one(
            r#""type": "choice", "default": "a", "options": [{"id": "a", "label": "x"}, {"id": "b", "label": "y"}]"#,
        )
        .unwrap();
        assert!(choice.accepts(&Value::Text("b".into())));
        assert!(!choice.accepts(&Value::Text("c".into())));
        let text = one(r#""type": "text", "max_length": 3, "default": "", "group": "g""#).unwrap();
        assert!(text.accepts(&Value::Text("abc".into())));
        assert!(!text.accepts(&Value::Text("abcd".into())));
        assert!(!text.accepts(&Value::Text("a\u{202e}b".into())));
        assert!(text.accepts(&Value::Text("ééé".into())));
        assert!(!text.accepts(&Value::Bool(true)));
        assert!(!text.one_click());
        assert!(choice.one_click() && number.one_click());
        assert!(number.accepts(&Value::Number(0.0)) && number.accepts(&Value::Number(10.0)));
        assert!(!number.accepts(&Value::Number(-1.0)) && !number.accepts(&Value::Number(f64::NAN)));
        assert!(!choice.accepts(&Value::Text("B".into())) && !choice.accepts(&Value::Number(1.0)));
        let toggle = one(r#""type": "toggle", "default": true"#).unwrap();
        let color = one(r##""type": "color", "default": "#000000""##).unwrap();
        assert!(toggle.one_click() && color.one_click());
        assert!(
            !toggle.accepts(&Value::Number(1.0)) && !color.accepts(&Value::Text("#000000".into()))
        );
        assert_eq!(
            number.resolve(Some(&Value::Number(7.0))),
            Value::Number(7.0)
        );
        assert_eq!(
            number.resolve(Some(&Value::Number(70.0))),
            Value::Number(1.0)
        );
        assert_eq!(number.resolve(None), Value::Number(1.0));
        let zero = one(r#""type": "number", "min": -1, "max": 1, "default": 1"#).unwrap();
        assert!(
            matches!(zero.resolve(Some(&Value::Number(-0.0))), Value::Number(n) if n.is_sign_positive())
        );
    }
}
