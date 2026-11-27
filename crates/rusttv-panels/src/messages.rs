//! Message tables (PANEL_STUDIO.md § 5.6): `strings/<lang>.json`, one per
//! language, message id → text with named placeholders and CLDR plural forms.
//!
//! ```json
//! {"messages": {
//!   "title": "Weather in {city}",
//!   "unread": {"plural": "count", "one": "{count} new message", "other": "{count} new messages"}
//! }}
//! ```
//!
//! `{name}` is a placeholder, `{{` and `}}` are braces. A package's tables are
//! its own: RustTV's UI never looks a message up in them. A translation may
//! leave out a message (the lookup falls back; [`Messages::missing`] lists
//! them for a warning) and may leave out a placeholder, but may not add a
//! message or a placeholder the default language doesn't have. A
//! translation may be plural where the default is plain, as long as its
//! count is one of the default's placeholders. The shape is governed by
//! `meta.format`; tables carry no notes (those live in the studio project).

use crate::document::Document;
use crate::error::{Code, PackageError, PackageFile, Position};
use crate::meta::{Meta, limits};
use crate::plural::{PluralCategory, plural_categories, plural_category};
use crate::text::{Language, Text, clean_text};
use crate::{MAX_JSON_DEPTH, json};
use serde::Deserialize;
use serde::de::{self, Deserializer, MapAccess, Visitor};
use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// A message id, `[a-z0-9_.-]{1,64}`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MessageId(String);

impl MessageId {
    /// The id in `text`.
    ///
    /// # Errors
    ///
    /// [`Code::BadMessageId`] unless `text` is `[a-z0-9_.-]{1,64}`.
    pub fn parse(text: &str) -> Result<Self, Code> {
        let ok = (1..=64).contains(&text.len())
            && text.bytes().all(|b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.' | b'-')
            });
        if ok {
            Ok(Self(text.to_owned()))
        } else {
            Err(Code::BadMessageId)
        }
    }

    /// The id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for MessageId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

deserialize_via!(MessageId, MessageId::parse);

/// A placeholder's name, `[a-z0-9_]{1,32}`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlaceholderName(String);

impl PlaceholderName {
    /// The name in `text`.
    ///
    /// # Errors
    ///
    /// [`Code::BadPlaceholder`] unless `text` is `[a-z0-9_]{1,32}`.
    pub fn parse(text: &str) -> Result<Self, Code> {
        let ok = (1..=32).contains(&text.len())
            && text
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if ok {
            Ok(Self(text.to_owned()))
        } else {
            Err(Code::BadPlaceholder)
        }
    }

    /// The name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

deserialize_via!(PlaceholderName, PlaceholderName::parse);

/// A piece of a [`Template`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Piece {
    /// Literal text.
    Text(String),
    /// A placeholder, filled at lookup.
    Arg(PlaceholderName),
}

/// A cleaned text (up to [`limits::TEXT`] characters) split at its
/// placeholders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template(Vec<Piece>);

impl Template {
    /// The template in `raw`, cleaned first.
    ///
    /// # Errors
    ///
    /// The [`Text`] codes, or [`Code::BadPlaceholder`] for a bad name or a
    /// lone brace.
    pub fn parse(raw: &str) -> Result<Self, Code> {
        let text = Text::<{ limits::TEXT }>::new(raw)?;
        let mut pieces = Vec::new();
        let mut literal = String::new();
        let mut placeholders = 0_usize;
        let mut chars = text.as_str().chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '{' | '}' if chars.peek() == Some(&c) => {
                    chars.next();
                    literal.push(c);
                }
                '{' => {
                    let mut name = String::new();
                    loop {
                        match chars.next() {
                            Some('}') => break,
                            Some(c) if name.len() < 32 => name.push(c),
                            _ => return Err(Code::BadPlaceholder),
                        }
                    }
                    if !literal.is_empty() {
                        pieces.push(Piece::Text(std::mem::take(&mut literal)));
                    }
                    pieces.push(Piece::Arg(PlaceholderName::parse(&name)?));
                    placeholders += 1;
                    if placeholders > limits::PLACEHOLDERS {
                        return Err(Code::BadPlaceholder);
                    }
                }
                '}' => return Err(Code::BadPlaceholder),
                c => literal.push(c),
            }
        }
        if !literal.is_empty() {
            pieces.push(Piece::Text(literal));
        }
        Ok(Self(pieces))
    }

    /// Its pieces, in order.
    pub fn pieces(&self) -> &[Piece] {
        &self.0
    }

    fn placeholders(&self) -> impl Iterator<Item = &PlaceholderName> {
        self.0.iter().filter_map(|piece| match piece {
            Piece::Arg(name) => Some(name),
            Piece::Text(_) => None,
        })
    }
}

deserialize_via!(Template, Template::parse);

/// One message: a text, or plural forms chosen by a count.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Message {
    /// One text.
    Plain(Template),
    /// Forms by plural category; `other` is always there.
    Plural {
        /// The placeholder whose count picks the form.
        arg: PlaceholderName,
        /// The forms, by category.
        forms: BTreeMap<PluralCategory, Template>,
    },
}

/// A value for a placeholder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Arg<'a> {
    /// Text, e.g. a city's name; cleaned and isolated when filled in.
    Text(&'a str),
    /// A whole number; picks plural forms.
    Count(u64),
}

/// First Strong Isolate and Pop Directional Isolate: a filled-in text keeps
/// its own direction and can't reorder the message around it.
const FSI: char = '\u{2068}';
const PDI: char = '\u{2069}';

impl Message {
    /// The message in `language` (the language of the table it came from,
    /// for plural rules) with `args` filled in. A text value is cut to
    /// [`limits::ARG`] characters, cleaned and isolated; a count is written
    /// in ASCII digits (locale digits come with expressions). A placeholder
    /// without a value is left empty; a plural without a count uses `other`.
    pub fn format(&self, language: Language, args: &[(&str, Arg<'_>)]) -> String {
        let value = |name: &str| args.iter().find(|(n, _)| *n == name).map(|(_, v)| *v);
        let template = match self {
            Self::Plain(template) => template,
            Self::Plural { arg, forms } => {
                let category = match value(arg.as_str()) {
                    Some(Arg::Count(n)) => plural_category(language, n),
                    _ => PluralCategory::Other,
                };
                match forms
                    .get(&category)
                    .or_else(|| forms.get(&PluralCategory::Other))
                {
                    Some(template) => template,
                    None => return String::new(),
                }
            }
        };
        let mut out = String::new();
        for piece in template.pieces() {
            match piece {
                Piece::Text(text) => out.push_str(text),
                Piece::Arg(name) => match value(name.as_str()) {
                    Some(Arg::Text(text)) => {
                        let cut = text
                            .char_indices()
                            .nth(limits::ARG)
                            .map_or(text, |(at, _)| &text[..at]);
                        out.push(FSI);
                        out.push_str(&clean_text(cut));
                        out.push(PDI);
                    }
                    Some(Arg::Count(n)) => out.push_str(&n.to_string()),
                    None => {}
                },
            }
        }
        out
    }

    /// Every placeholder it uses, with a plural's count.
    fn names(&self) -> BTreeSet<&PlaceholderName> {
        match self {
            Self::Plain(template) => template.placeholders().collect(),
            Self::Plural { arg, forms } => forms
                .values()
                .flat_map(Template::placeholders)
                .chain([arg])
                .collect(),
        }
    }

    fn strings(&self) -> usize {
        match self {
            Self::Plain(_) => 1,
            Self::Plural { forms, .. } => forms.len(),
        }
    }
}

/// The JSON shape of a plural message.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPlural {
    plural: PlaceholderName,
    #[serde(default, deserialize_with = "form")]
    zero: Option<Template>,
    #[serde(default, deserialize_with = "form")]
    one: Option<Template>,
    #[serde(default, deserialize_with = "form")]
    two: Option<Template>,
    #[serde(default, deserialize_with = "form")]
    few: Option<Template>,
    #[serde(default, deserialize_with = "form")]
    many: Option<Template>,
    #[serde(default, deserialize_with = "form")]
    other: Option<Template>,
}

/// A plural form that is there must be a text (`null` is a wrong type).
fn form<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Template>, D::Error> {
    Template::deserialize(deserializer).map(Some)
}

impl<'de> Deserialize<'de> for Message {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct MessageVisitor;
        impl<'de> Visitor<'de> for MessageVisitor {
            type Value = Message;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a text or plural forms")
            }
            fn visit_str<E: de::Error>(self, raw: &str) -> Result<Message, E> {
                Template::parse(raw)
                    .map(Message::Plain)
                    .map_err(|code| E::custom(code.as_str()))
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Message, A::Error> {
                let raw = RawPlural::deserialize(de::value::MapAccessDeserializer::new(map))?;
                let forms: BTreeMap<_, _> = [
                    (PluralCategory::Zero, raw.zero),
                    (PluralCategory::One, raw.one),
                    (PluralCategory::Two, raw.two),
                    (PluralCategory::Few, raw.few),
                    (PluralCategory::Many, raw.many),
                    (PluralCategory::Other, raw.other),
                ]
                .into_iter()
                .filter_map(|(category, form)| form.map(|form| (category, form)))
                .collect();
                if !forms.contains_key(&PluralCategory::Other) {
                    return Err(de::Error::custom(Code::BadPlural.as_str()));
                }
                Ok(Message::Plural {
                    arg: raw.plural,
                    forms,
                })
            }
        }
        deserializer.deserialize_any(MessageVisitor)
    }
}

/// The messages in file order.
struct Entries(Vec<(MessageId, Message)>);

impl<'de> Deserialize<'de> for Entries {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct EntriesVisitor;
        impl<'de> Visitor<'de> for EntriesVisitor {
            type Value = Entries;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("messages")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Entries, A::Error> {
                let mut entries = Vec::new();
                let mut strings = 0_usize;
                while let Some(entry) = map.next_entry::<MessageId, Message>()? {
                    strings += entry.1.strings();
                    if strings > limits::STRINGS {
                        return Err(de::Error::custom(Code::TooMany.as_str()));
                    }
                    entries.push(entry);
                }
                Ok(Entries(entries))
            }
        }
        deserializer.deserialize_map(EntriesVisitor)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTable {
    messages: Entries,
}

/// One language's messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Messages {
    language: Language,
    /// Each message with its place in the file.
    by_id: BTreeMap<MessageId, (usize, Message)>,
}

impl Messages {
    /// The table's language.
    pub const fn language(&self) -> Language {
        self.language
    }

    /// The message `id`, if this table has it.
    pub fn get(&self, id: &str) -> Option<&Message> {
        self.by_id.get(id).map(|(_, message)| message)
    }

    /// The ids `default` has and this table lacks: missing translations,
    /// a warning for user packages (the lookup falls back).
    pub fn missing<'a>(&self, default: &'a Self) -> Vec<&'a MessageId> {
        default
            .by_id
            .keys()
            .filter(|id| !self.by_id.contains_key(id.as_str()))
            .collect()
    }

    /// How many messages it has.
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// Whether it has none.
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

/// Reads `strings/<language>.json`.
///
/// # Errors
///
/// A [`PackageError`] in [`PackageFile::Strings`]: size, the raw JSON pass, ids,
/// templates, at most [`limits::STRINGS`] strings, and plural forms that
/// `language` has ([`Code::BadPlural`]).
pub fn parse_messages(language: Language, text: &str) -> Result<Messages, PackageError> {
    let file = PackageFile::Strings(language);
    let fail = |code, at| PackageError::new(code, at).in_file(file);
    if text.len() > limits::STRINGS_JSON {
        return Err(fail(Code::TooLarge, Position::File));
    }
    json::check(text, MAX_JSON_DEPTH, 0, false).map_err(|e| e.in_file(file))?;
    let raw: RawTable =
        serde_json::from_str(text).map_err(|e| PackageError::from_typed_json(&e).in_file(file))?;
    let has = plural_categories(language);
    let mut by_id = BTreeMap::new();
    for (index, (id, message)) in raw.messages.0.into_iter().enumerate() {
        if let Message::Plural { forms, .. } = &message
            && forms.keys().any(|category| !has.contains(category))
        {
            return Err(fail(Code::BadPlural, Position::Message { index }));
        }
        by_id.insert(id, (index, message));
    }
    Ok(Messages { language, by_id })
}

/// Checks a package's tables against each other and its `meta`: a default
/// language table whenever there are any (or any knobs), translations with
/// no message or placeholder the default language lacks, and the knobs'
/// labels in the default table without placeholders.
///
/// # Errors
///
/// [`Code::NoDefaultStrings`], [`Code::UnknownMessage`] or
/// [`Code::PlaceholderMismatch`].
pub fn check_messages(
    meta: &Meta,
    tables: &BTreeMap<Language, Messages>,
) -> Result<(), PackageError> {
    check_knob_messages(meta, tables.get(&meta.default_language))?;
    if tables.is_empty() {
        return Ok(());
    }
    let Some(default) = tables.get(&meta.default_language) else {
        return Err(PackageError::new(
            Code::NoDefaultStrings,
            Position::field("meta.default_language"),
        ));
    };
    for table in tables.values().filter(|t| t.language != default.language) {
        let file = PackageFile::Strings(table.language);
        let mut in_file_order: Vec<_> = table.by_id.iter().collect();
        in_file_order.sort_by_key(|(_, (index, _))| *index);
        for (id, (index, message)) in in_file_order {
            let at = Position::Message { index: *index };
            let Some(original) = default.get(id.as_str()) else {
                return Err(PackageError::new(Code::UnknownMessage, at).in_file(file));
            };
            if !message.names().is_subset(&original.names()) {
                return Err(PackageError::new(Code::PlaceholderMismatch, at).in_file(file));
            }
        }
    }
    Ok(())
}

/// Checks the messages `meta.knobs` uses: every label, group heading and
/// option label must be in the default table and fill no placeholder (a
/// label has nothing to fill it with), and messages in `visible_if` must
/// exist with the placeholders they fill.
fn check_knob_messages(meta: &Meta, default: Option<&Messages>) -> Result<(), PackageError> {
    for (index, knob) in meta.knobs.iter().enumerate() {
        let at = Position::Field {
            path: "meta.knobs",
            index: Some(index),
        };
        let fail = |code| Err(PackageError::new(code, at));
        let Some(default) = default else {
            return fail(Code::NoDefaultStrings);
        };
        let options = match &knob.kind {
            crate::KnobKind::Choice { options, .. } => options.as_slice(),
            _ => &[],
        };
        let labels = [Some(&knob.label), knob.group.as_ref()]
            .into_iter()
            .flatten()
            .chain(options.iter().map(|o| &o.label));
        for id in labels {
            match default.get(id.as_str()) {
                None => return fail(Code::UnknownMessage),
                Some(message) if !message.names().is_empty() => {
                    return fail(Code::PlaceholderMismatch);
                }
                Some(_) => {}
            }
        }
        let mut found = Ok(());
        if let Some(condition) = &knob.visible_if {
            crate::expr::each_message(condition, &mut |id, args| {
                if found.is_ok() {
                    found = match default.get(id.as_str()) {
                        None => Err(Code::UnknownMessage),
                        Some(m) if args.iter().all(|(name, _)| m.names().contains(name)) => Ok(()),
                        Some(_) => Err(Code::PlaceholderMismatch),
                    };
                }
            });
        }
        if let Err(code) = found {
            return fail(code);
        }
    }
    Ok(())
}

/// Checks the messages a panel's bindings use against its default
/// language table: each must exist there, and each placeholder a binding
/// fills must be one the message has.
///
/// # Errors
///
/// [`Code::NoDefaultStrings`] if bindings use messages and there is no
/// default table, [`Code::UnknownMessage`] or [`Code::PlaceholderMismatch`]
/// at the binding.
pub fn check_document_messages(
    meta: &Meta,
    document: &Document,
    tables: &BTreeMap<Language, Messages>,
) -> Result<(), PackageError> {
    let default = tables.get(&meta.default_language);
    crate::document::each_binding(document, &mut |at, expr| {
        let mut found = Ok(());
        crate::expr::each_message(expr, &mut |id, args| {
            if found.is_err() {
                return;
            }
            let Some(default) = default else {
                found = Err(Code::NoDefaultStrings);
                return;
            };
            found = match default.get(id.as_str()) {
                None => Err(Code::UnknownMessage),
                Some(message) => {
                    let names = message.names();
                    if args.iter().all(|(name, _)| names.contains(name)) {
                        Ok(())
                    } else {
                        Err(Code::PlaceholderMismatch)
                    }
                }
            };
        });
        found.map_err(|code| PackageError::new(code, at))
    })
}

/// The languages to try, in order: the TV's (or the panel's own), the
/// package's default, then English. The caller maps the TV's locale to one
/// of RustTV's languages first (as the launcher does for its own UI). None
/// of RustTV's languages is another's base language, so there is no
/// base-language step yet.
pub fn fallback_chain(wanted: Language, default: Language) -> Vec<Language> {
    let mut chain = Vec::with_capacity(3);
    let english = Language::english();
    for language in [wanted, default, english] {
        if !chain.contains(&language) {
            chain.push(language);
        }
    }
    chain
}

/// The first table along `chain` that has `id`, and the message.
pub fn lookup<'a>(
    tables: &'a BTreeMap<Language, Messages>,
    chain: &[Language],
    id: &str,
) -> Option<(Language, &'a Message)> {
    chain.iter().find_map(|language| {
        tables
            .get(language)
            .and_then(|table| table.get(id))
            .map(|message| (*language, message))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lang(code: &str) -> Language {
        Language::parse(code).unwrap()
    }

    fn table(code: &str, messages: &str) -> Result<Messages, PackageError> {
        parse_messages(lang(code), &format!(r#"{{"messages": {{{messages}}}}}"#))
    }

    fn code_of(code: &str, messages: &str) -> Option<Code> {
        table(code, messages).err().map(|e| e.code)
    }

    #[test]
    fn plain_and_plural_messages_parse_and_format() {
        let en = table(
            "en",
            r#""title": "Weather in {city}", "unread": {"plural": "count", "one": "{count} new message", "other": "{count} new messages"}, "braces": "{{x}}""#,
        )
        .unwrap();
        let title = en.get("title").unwrap();
        assert_eq!(
            title.format(lang("en"), &[("city", Arg::Text("Kyiv"))]),
            "Weather in \u{2068}Kyiv\u{2069}"
        );
        let unread = en.get("unread").unwrap();
        assert_eq!(
            unread.format(lang("en"), &[("count", Arg::Count(1))]),
            "1 new message"
        );
        assert_eq!(
            unread.format(lang("en"), &[("count", Arg::Count(5))]),
            "5 new messages"
        );
        assert_eq!(unread.format(lang("en"), &[]), " new messages");
        assert_eq!(en.get("braces").unwrap().format(lang("en"), &[]), "{x}");
        assert_eq!(en.len(), 3);
    }

    #[test]
    fn plural_forms_follow_the_tables_language() {
        let uk = table(
            "uk",
            r#""n": {"plural": "n", "one": "{n} лист", "few": "{n} листи", "many": "{n} листів", "other": "{n} листа"}"#,
        )
        .unwrap();
        let n = uk.get("n").unwrap();
        let say = |count| n.format(lang("uk"), &[("n", Arg::Count(count))]);
        assert_eq!(
            (say(21), say(3), say(11)),
            ("21 лист".into(), "3 листи".into(), "11 листів".into())
        );
    }

    #[test]
    fn a_filled_in_text_is_cleaned_and_isolated() {
        let en = table("en", r#""hi": "Hi {name}!""#).unwrap();
        let out = en
            .get("hi")
            .unwrap()
            .format(lang("en"), &[("name", Arg::Text("\u{202E}evil\u{2069}"))]);
        assert_eq!(out, "Hi \u{2068}evil\u{2069}!");
    }

    #[test]
    fn bad_ids_placeholders_and_plurals_are_refused() {
        assert_eq!(code_of("en", r#""Title": "x""#), Some(Code::BadMessageId));
        assert_eq!(
            code_of("en", r#""t": "{City}""#),
            Some(Code::BadPlaceholder)
        );
        assert_eq!(code_of("en", r#""t": "{city""#), Some(Code::BadPlaceholder));
        assert_eq!(code_of("en", r#""t": "a } b""#), Some(Code::BadPlaceholder));
        assert_eq!(code_of("en", r#""t": "{}""#), Some(Code::BadPlaceholder));
        assert_eq!(
            code_of("en", &format!(r#""t": "{{{}}}""#, "a".repeat(33))),
            Some(Code::BadPlaceholder)
        );
        assert_eq!(code_of("en", r#""t": "   ""#), Some(Code::EmptyText));
        assert_eq!(
            code_of("en", r#""t": {"plural": "n", "one": "x"}"#),
            Some(Code::BadPlural)
        );
        assert_eq!(
            code_of(
                "en",
                r#""t": {"plural": "n", "other": "x", "several": "y"}"#
            ),
            Some(Code::UnknownField)
        );
        assert_eq!(
            code_of("en", r#""t": {"one": "x", "other": "y"}"#),
            Some(Code::MissingField)
        );
        assert_eq!(code_of("en", r#""t": 7"#), Some(Code::WrongType));
        assert_eq!(
            code_of("en", r#""t": "x", "t": "y""#),
            Some(Code::DuplicateKey)
        );
    }

    #[test]
    fn a_plural_form_the_language_lacks_is_refused_where_it_is() {
        let error = table(
            "en",
            r#""a": "x", "b": {"plural": "n", "few": "x", "other": "y"}"#,
        )
        .unwrap_err();
        assert_eq!(
            (error.code, error.file, error.at),
            (
                Code::BadPlural,
                PackageFile::Strings(lang("en")),
                Position::Message { index: 1 }
            )
        );
        assert_eq!(error.to_string(), "BAD_PLURAL at strings/en.json message 1");
        assert_eq!(
            code_of("ja", r#""b": {"plural": "n", "one": "x", "other": "y"}"#),
            Some(Code::BadPlural)
        );
        assert_eq!(
            code_of(
                "ar",
                r#""b": {"plural": "n", "zero": "x", "two": "y", "other": "z"}"#
            ),
            None
        );
    }

    #[test]
    fn strings_and_size_are_capped() {
        let many: Vec<String> = (0..=limits::STRINGS)
            .map(|i| format!(r#""m{i}": "x""#))
            .collect();
        assert_eq!(code_of("en", &many.join(",")), Some(Code::TooMany));
        assert_eq!(code_of("en", &many[..limits::STRINGS].join(",")), None);
        let huge = format!(
            r#"{{"messages": {{"t": "{}"}}}}"#,
            "a".repeat(limits::STRINGS_JSON)
        );
        assert_eq!(
            parse_messages(lang("en"), &huge).unwrap_err().code,
            Code::TooLarge
        );
        assert_eq!(
            code_of("en", &format!(r#""t": "{}""#, "a".repeat(501))),
            Some(Code::TextTooLong)
        );
    }

    #[test]
    fn braces_and_placeholders_parse_exactly() {
        let pieces = |raw: &str| Template::parse(raw).map(|t| t.pieces().to_vec());
        let arg = |name| Piece::Arg(PlaceholderName::parse(name).unwrap());
        let text = |t: &str| Piece::Text(t.to_owned());
        assert_eq!(pieces("{{{a}}}"), Ok(vec![text("{"), arg("a"), text("}")]));
        assert_eq!(pieces("}}{"), Err(Code::BadPlaceholder));
        assert_eq!(pieces("{a}}"), Err(Code::BadPlaceholder));
        assert_eq!(
            pieces(&"{a}".repeat(limits::PLACEHOLDERS)).map(|p| p.len()),
            Ok(limits::PLACEHOLDERS)
        );
        assert_eq!(
            pieces(&"{a}".repeat(limits::PLACEHOLDERS + 1)),
            Err(Code::BadPlaceholder)
        );
    }

    #[test]
    fn plural_edge_cases_fall_back_to_other() {
        let uk = table(
            "uk",
            r#""n": {"plural": "n", "one": "один", "other": "{n}"}"#,
        )
        .unwrap();
        let n = uk.get("n").unwrap();
        assert_eq!(n.format(lang("uk"), &[("n", Arg::Count(3))]), "3");
        assert_eq!(n.format(lang("uk"), &[("n", Arg::Count(21))]), "один");
        assert_eq!(
            n.format(lang("uk"), &[("n", Arg::Text("1"))]),
            "\u{2068}1\u{2069}"
        );
        assert_eq!(
            code_of("en", r#""t": {"plural": "n", "one": null, "other": "x"}"#),
            Some(Code::WrongType)
        );
    }

    #[test]
    fn a_filled_in_text_is_cut() {
        let en = table("en", r#""t": "{a}""#).unwrap();
        let long = "я".repeat(limits::ARG + 10);
        let out = en
            .get("t")
            .unwrap()
            .format(lang("en"), &[("a", Arg::Text(&long))]);
        assert_eq!(out.chars().count(), limits::ARG + 2);
    }

    #[test]
    fn package_keys_mean_nothing_in_a_table() {
        let unknown = |text: &str| parse_messages(lang("en"), text).unwrap_err().code;
        assert_eq!(
            unknown(r#"{"messages": {"a": "x"}, "document": {}}"#),
            Code::UnknownField
        );
        assert_eq!(
            unknown(r#"{"messages": {}, "meta": []}"#),
            Code::UnknownField
        );
    }

    #[test]
    fn missing_translations_are_listed() {
        let all = tables(&[("en", r#""a": "A", "b": "B""#), ("uk", r#""a": "А""#)]);
        let missing = all[&lang("uk")].missing(&all[&lang("en")]);
        assert_eq!(
            missing.iter().map(|id| id.as_str()).collect::<Vec<_>>(),
            ["b"]
        );
    }

    #[test]
    fn errors_never_contain_the_tables_text() {
        let error = table("en", r#""secret": "{secret""#).unwrap_err();
        assert!(!error.to_string().contains("secret"), "{error}");
    }

    fn meta(default: &str) -> Meta {
        let text = format!(
            r#"{{"meta": {{"format": 2, "min_launcher": "0.5.0", "id": "user.test", "kind": "theme",
               "version": "1.0.0", "default_language": "{default}", "name": {{"{default}": "Test"}}}}}}"#
        );
        crate::parse_package(
            &text,
            &crate::Capabilities::new(crate::Version::new(0, 5, 0), [""; 0]),
        )
        .unwrap()
        .meta
    }

    fn tables(list: &[(&str, &str)]) -> BTreeMap<Language, Messages> {
        list.iter()
            .map(|(code, messages)| (lang(code), table(code, messages).unwrap()))
            .collect()
    }

    fn knob_meta(knobs: &str) -> Meta {
        let text = format!(
            r#"{{"meta": {{"format": 2, "min_launcher": "0.5.0", "id": "user.test", "kind": "theme",
               "version": "1.0.0", "default_language": "en", "name": {{"en": "Test"}}, "knobs": [{knobs}]}}}}"#
        );
        crate::parse_package(
            &text,
            &crate::Capabilities::new(crate::Version::new(0, 5, 0), [""; 0]),
        )
        .unwrap()
        .meta
    }

    #[test]
    fn knob_labels_are_plain_messages_in_the_default_table() {
        let choice = r#"{"name": "style", "type": "choice", "label": "style", "group": "look", "default": "a",
            "options": [{"id": "a", "label": "round"}, {"id": "b", "label": "square"}]}"#;
        let shown = r#"{"name": "on", "type": "toggle", "label": "style", "default": true,
            "visible_if": {"call": "eq", "args": [{"msg": "round"}, "x"]}}"#;
        let en = r#""style": "Style", "look": "Look", "round": "Round", "square": "Square""#;
        let check = |knobs: &str, list: &[(&str, &str)]| {
            check_messages(&knob_meta(knobs), &tables(list))
                .err()
                .map(|e| (e.code, e.at))
        };
        let at = |index| Position::Field {
            path: "meta.knobs",
            index: Some(index),
        };
        assert_eq!(check(&format!("{choice},{shown}"), &[("en", en)]), None);
        assert_eq!(check(choice, &[]), Some((Code::NoDefaultStrings, at(0))));
        for missing in ["style", "look", "square"] {
            let without = en.replace(&format!(r#""{missing}":"#), r#""other":"#);
            assert_eq!(
                check(choice, &[("en", &without)]),
                Some((Code::UnknownMessage, at(0))),
                "{missing}"
            );
        }
        let with_placeholder = en.replace(r#""Look""#, r#""Look {x}""#);
        assert_eq!(
            check(choice, &[("en", &with_placeholder)]),
            Some((Code::PlaceholderMismatch, at(0)))
        );
        for (from, to) in [
            (r#""Style""#, r#""Style {x}""#),
            (r#""Square""#, r#""Square {x}""#),
        ] {
            assert_eq!(
                check(choice, &[("en", &en.replace(from, to))]),
                Some((Code::PlaceholderMismatch, at(0))),
                "{to}"
            );
        }
        // A plural message needs its count, which a label doesn't have.
        let plural = en.replace(
            r#""Look""#,
            r#"{"plural": "n", "one": "Look", "other": "Looks"}"#,
        );
        assert_eq!(
            check(choice, &[("en", &plural)]),
            Some((Code::PlaceholderMismatch, at(0)))
        );
        let second = format!(
            r#"{choice}, {}"#,
            choice
                .replace(r#""name": "style""#, r#""name": "two""#)
                .replace(r#""label": "style""#, r#""label": "gone""#)
        );
        assert_eq!(
            check(&second, &[("en", en)]),
            Some((Code::UnknownMessage, at(1)))
        );
        assert_eq!(
            check(choice, &[("uk", en)]),
            Some((Code::NoDefaultStrings, at(0)))
        );
        let with_arg = shown.replace(
            r#"{"msg": "round"}"#,
            r#"{"msg": "round", "args": {"x": "y"}}"#,
        );
        assert_eq!(
            check(&with_arg, &[("en", en)]),
            Some((Code::PlaceholderMismatch, at(0)))
        );
        let round_x = en.replace(r#""Round""#, r#""Round {x}""#);
        assert_eq!(check(&with_arg, &[("en", &round_x)]), None);
        let no_round = en.replace(r#""round":"#, r#""other":"#);
        let toggle_first = shown.replace(r#""label": "style""#, r#""label": "look""#);
        assert_eq!(
            check(&toggle_first, &[("en", &no_round)]),
            Some((Code::UnknownMessage, at(0)))
        );
    }

    #[test]
    fn translations_are_checked_against_the_default_language() {
        let en = r#""t": "Weather in {city}", "u": {"plural": "count", "one": "one", "other": "{count}"}"#;
        let check = |list: &[(&str, &str)]| {
            check_messages(&meta("en"), &tables(list))
                .err()
                .map(|e| (e.code, e.file))
        };
        let uk = PackageFile::Strings(lang("uk"));
        assert_eq!(check(&[]), None);
        assert_eq!(check(&[("en", en)]), None);
        assert_eq!(check(&[("en", en), ("uk", r#""t": "Погода""#)]), None);
        assert_eq!(
            check(&[
                ("en", en),
                (
                    "uk",
                    r#""u": {"plural": "count", "one": "{count} раз", "few": "{count} рази", "many": "{count} разів", "other": "{count}"}"#
                )
            ]),
            None
        );
        assert_eq!(
            check(&[("en", en), ("uk", r#""x": "Новий""#)]),
            Some((Code::UnknownMessage, uk))
        );
        assert_eq!(
            check(&[("en", en), ("uk", r#""t": "{town}""#)]),
            Some((Code::PlaceholderMismatch, uk))
        );
        assert_eq!(
            check(&[
                ("en", en),
                ("uk", r#""t": {"plural": "n", "other": "{city}"}"#)
            ]),
            Some((Code::PlaceholderMismatch, uk))
        );
        assert_eq!(
            check(&[("uk", r#""t": "x""#)]),
            Some((Code::NoDefaultStrings, PackageFile::PackageJson))
        );
    }

    #[test]
    fn the_first_bad_message_in_file_order_is_reported() {
        let all = tables(&[
            ("en", r#""a": "A", "z": "Z""#),
            ("uk", r#""z": "{x}", "a": "{y}""#),
        ]);
        let error = check_messages(&meta("en"), &all).unwrap_err();
        assert_eq!(error.at, Position::Message { index: 0 });
        let all = tables(&[
            ("en", r#""t": {"plural": "count", "other": "{count}"}"#),
            ("uk", r#""t": {"plural": "n", "other": "x"}"#),
        ]);
        assert_eq!(
            check_messages(&meta("en"), &all).unwrap_err().code,
            Code::PlaceholderMismatch
        );
    }

    #[test]
    fn bindings_use_messages_the_default_table_has() {
        let package = |args: &str| {
            format!(
                r#"{{"meta": {{"format": 2, "min_launcher": "0.5.0", "id": "user.test", "kind": "panel",
                   "version": "1.0.0", "default_language": "en", "name": {{"en": "Test"}}, "faces": ["tile"],
                   "sources": ["history"]}},
                  "document": {{"variants": [{{"face": "tile", "root": {{"id": "root", "type": "frame", "w": 250, "h": 141,
                   "children": [{{"id": "t", "type": "text", "w": 10, "h": 10, "text": "…", "size": 10,
                   "bindings": {{"text": {{"msg": "unread", "args": {args}}}}}}}]}}}}]}}}}"#
            )
        };
        let can = crate::Capabilities::new(crate::Version::new(0, 5, 0), [""; 0]);
        let read = |args: &str| crate::parse_package(&package(args), &can).unwrap();
        let en = r#""unread": {"plural": "count", "one": "one", "other": "{count}"}"#;
        let check = |args: &str, list: &[(&str, &str)]| {
            let p = read(args);
            check_document_messages(&p.meta, p.document.as_ref().unwrap(), &tables(list))
                .err()
                .map(|e| (e.code, e.at))
        };
        let at = Position::Binding {
            variant: 0,
            node: 1,
            property: "text",
        };
        let count = r#"{"count": {"source": "history.waiting"}}"#;
        assert_eq!(check(count, &[("en", en)]), None);
        assert_eq!(
            check(r#"{"other": 1}"#, &[("en", en)]),
            Some((Code::PlaceholderMismatch, at))
        );
        assert_eq!(
            check(count, &[("en", r#""read": "x""#)]),
            Some((Code::UnknownMessage, at))
        );
        assert_eq!(check(count, &[]), Some((Code::NoDefaultStrings, at)));
    }

    #[test]
    fn lookup_falls_back_to_the_default_then_english() {
        assert_eq!(
            fallback_chain(lang("uk"), lang("de")),
            [lang("uk"), lang("de"), lang("en")]
        );
        assert_eq!(fallback_chain(lang("en"), lang("en")), [lang("en")]);
        let all = tables(&[
            ("de", r#""a": "A-de", "b": "B-de""#),
            ("uk", r#""a": "A-uk""#),
        ]);
        let chain = fallback_chain(lang("uk"), lang("de"));
        let say = |id| lookup(&all, &chain, id).map(|(language, m)| m.format(language, &[]));
        assert_eq!(say("a"), Some("A-uk".to_owned()));
        assert_eq!(say("b"), Some("B-de".to_owned()));
        assert_eq!(say("c"), None);
    }
}
