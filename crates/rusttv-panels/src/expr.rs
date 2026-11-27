//! Expressions (PANEL_STUDIO.md § 6.4): small JSON trees a binding
//! evaluates, typed and checked when the package is read, total when run.
//!
//! ```json
//! "hello"                                          a literal (text, or a colour where one is wanted)
//! 12.5, true                                       number, yes/no
//! {"source": "network.connected"}                  a field of a source group in meta.sources
//! {"knob": "accent"}                               a knob in meta.knobs
//! {"item": "name"}                                 a field of the list item, inside a repeat
//! {"count": "apps.list"}                           how many items a list has
//! {"msg": "unread", "args": {"count": {...}}}      a message from the package's tables
//! {"if": [condition, then, else]}
//! {"call": "format_time", "args": [{"source": "clock.now"}, "HH:mm"]}
//! ```
//!
//! A string literal is text, or a colour where a colour is wanted (so text
//! that looks like `#fff` can't be a colour by accident: it is checked
//! against the place it is used). Time patterns use `HH H hh h mm ss dd d
//! MM M yyyy a` and `: . / - ,` and space; `'` is reserved for quoting
//! letters later.
//!
//! There are no variables, loops or recursion, so an expression's cost is
//! bounded by its size; [`eval`] still burns fuel per step so the host can
//! share one budget across a frame. Evaluation never panics: a missing
//! source, a division by zero or spent fuel gives `None`, and the binding
//! shows its fallback. With no variables there are no cycles to detect.

use crate::Color;
use crate::error::Code;
use crate::messages::{Arg, MessageId, PlaceholderName};
use crate::meta::{Name, limits};
use crate::text::clean_keeping_ends;
use serde::Deserialize;
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// A value's type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Type {
    /// Yes or no.
    Bool,
    /// A number (counts, bytes, seconds).
    Number,
    /// Text.
    Text,
    /// A colour.
    Color,
    /// A moment with its UTC offset.
    Time,
}

/// A moment: seconds since 1970 (UTC) and the place's offset from UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Moment {
    /// Seconds since 1970-01-01T00:00:00Z.
    pub unix: i64,
    /// Minutes east of UTC.
    pub offset_minutes: i32,
}

/// A value an expression produces or a source provides.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Value {
    /// Yes or no.
    Bool(bool),
    /// A finite number.
    Number(f64),
    /// Text.
    Text(String),
    /// A colour.
    Color(Color),
    /// A moment.
    Time(Moment),
}

/// What [`eval`] reads: live source values and the package's messages.
pub trait Inputs {
    /// The value of `group.field`, or `None` while it is loading, empty or
    /// unavailable.
    fn source(&self, group: &str, field: &str) -> Option<Value>;
    /// The value of knob `name`: [`crate::Knob::resolve`] of the stored
    /// one, so it has the knob's type and rules.
    fn knob(&self, name: &str) -> Option<Value>;
    /// Field `field` of the item a repeat is drawing; `None` outside one.
    fn item(&self, field: &str) -> Option<Value>;
    /// How many items list `group.field` has now (before a repeat's `max`).
    fn count(&self, group: &str, field: &str) -> Option<usize>;
    /// The message `id` in the panel's language with `args` filled in.
    fn message(&self, id: &str, args: &[(&str, Arg<'_>)]) -> Option<String>;
}

/// One source field: its group (listed in `meta.sources`), name, type and
/// privacy tier (§ 6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SourceField {
    /// The group, e.g. `network`.
    pub group: &'static str,
    /// The field, e.g. `connected`.
    pub field: &'static str,
    /// Its type.
    pub kind: Type,
    /// 0 (harmless) to 2 (warned about on review).
    pub tier: u8,
}

const fn field(group: &'static str, field: &'static str, kind: Type, tier: u8) -> SourceField {
    SourceField {
        group,
        field,
        kind,
        tier,
    }
}

/// Every source field a package can bind (§ 6.4). Units: `storage.free`
/// in bytes; `device.uptime` and `history.age` in seconds. Lists are in
/// [`collections`].
const SOURCES: &[SourceField] = &[
    field("clock", "now", Type::Time, 0),
    field("clock", "hour24", Type::Bool, 0),
    field("clock", "zone", Type::Text, 0),
    field("theme", "name", Type::Text, 0),
    field("theme", "mode", Type::Text, 0),
    field("updater", "state", Type::Text, 0),
    field("updater", "version", Type::Text, 0),
    field("device", "model", Type::Text, 1),
    field("device", "uptime", Type::Number, 1),
    field("storage", "free", Type::Number, 1),
    field("network", "connected", Type::Bool, 1),
    field("phones", "count", Type::Number, 2),
    field("history", "age", Type::Number, 2),
    field("history", "waiting", Type::Number, 2),
];

/// A list a `repeat` node draws (PANEL_STUDIO.md § 5.3): its group (listed
/// in `meta.sources`), name, item fields and privacy tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Collection {
    /// The group, e.g. `apps`.
    pub group: &'static str,
    /// The list, e.g. `list`.
    pub field: &'static str,
    /// Each item's fields and their types.
    pub items: &'static [(&'static str, Type)],
    /// 0 (harmless) to 2 (warned about on review).
    pub tier: u8,
}

/// Every list a package can repeat: the visible apps' names and the paired
/// phones' names (shown only on the TV). More item fields (icons, banners,
/// palettes) come with image bindings and step 2f.
const COLLECTIONS: &[Collection] = &[
    Collection {
        group: "apps",
        field: "list",
        items: &[("name", Type::Text)],
        tier: 2,
    },
    Collection {
        group: "phones",
        field: "list",
        items: &[("name", Type::Text)],
        tier: 2,
    },
];

/// Every source field a package can bind.
pub fn sources() -> &'static [SourceField] {
    SOURCES
}

/// Every list a package can repeat.
pub fn collections() -> &'static [Collection] {
    COLLECTIONS
}

/// The list `group.field`, if there is one.
pub fn collection(group: &str, field: &str) -> Option<&'static Collection> {
    COLLECTIONS
        .iter()
        .find(|c| c.group == group && c.field == field)
}

/// Whether `group` is a source group a package may list in `meta.sources`.
pub fn is_source_group(group: &str) -> bool {
    source_tier(group).is_some()
}

/// A source group's privacy tier (every field and list of a group has the
/// same one); the review warns about tier 2.
pub fn source_tier(group: &str) -> Option<u8> {
    SOURCES
        .iter()
        .find(|s| s.group == group)
        .map(|s| s.tier)
        .or_else(|| {
            COLLECTIONS
                .iter()
                .find(|c| c.group == group)
                .map(|c| c.tier)
        })
}

/// A function an expression can call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Function {
    /// `not(bool)`.
    Not,
    /// `and(bool, bool, …)`.
    And,
    /// `or(bool, bool, …)`.
    Or,
    /// `eq(a, b)`, same type.
    Eq,
    /// `ne(a, b)`, same type.
    Ne,
    /// `lt(a, b)`, numbers or times.
    Lt,
    /// `le(a, b)`.
    Le,
    /// `gt(a, b)`.
    Gt,
    /// `ge(a, b)`.
    Ge,
    /// `add(n, n)`.
    Add,
    /// `sub(n, n)`.
    Sub,
    /// `mul(n, n)`.
    Mul,
    /// `div(n, n)`; dividing by 0 gives no value.
    Div,
    /// `min(n, n)`.
    Min,
    /// `max(n, n)`.
    Max,
    /// `round(n)`.
    Round,
    /// `concat(text, text, …)`.
    Concat,
    /// `text(n)` or `text(n, decimals)`: a number as text, ASCII digits.
    Text,
    /// `format_time(time, "pattern")`.
    FormatTime,
}

const FUNCTIONS: &[(&str, Function)] = &[
    ("not", Function::Not),
    ("and", Function::And),
    ("or", Function::Or),
    ("eq", Function::Eq),
    ("ne", Function::Ne),
    ("lt", Function::Lt),
    ("le", Function::Le),
    ("gt", Function::Gt),
    ("ge", Function::Ge),
    ("add", Function::Add),
    ("sub", Function::Sub),
    ("mul", Function::Mul),
    ("div", Function::Div),
    ("min", Function::Min),
    ("max", Function::Max),
    ("round", Function::Round),
    ("concat", Function::Concat),
    ("text", Function::Text),
    ("format_time", Function::FormatTime),
];

/// An expression.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Expr {
    /// A text literal (cleaned); a colour where a colour is wanted.
    Str(String),
    /// A number literal.
    Number(f64),
    /// A yes/no literal.
    Bool(bool),
    /// `group.field` of a source.
    Source {
        /// The group.
        group: String,
        /// The field.
        field: String,
    },
    /// A knob's value (`{"knob": name}`).
    Knob(Name),
    /// A field of the item a repeat is drawing (`{"item": field}`).
    Item(String),
    /// How many items list `group.field` has (`{"count": "apps.list"}`).
    Count {
        /// The group.
        group: String,
        /// The list.
        field: String,
    },
    /// A message with its placeholders' values.
    Msg {
        /// The message id.
        id: MessageId,
        /// Placeholder values.
        args: Vec<(PlaceholderName, Expr)>,
    },
    /// `if(condition, then, else)`.
    If(Box<[Expr; 3]>),
    /// A function call.
    Call {
        /// The function.
        function: Function,
        /// Its arguments.
        args: Vec<Expr>,
    },
}

/// The raw `args` of a call (a list) or a message (an object).
enum RawArgs {
    List(Vec<Expr>),
    Named(Vec<(PlaceholderName, Expr)>),
}

impl<'de> Deserialize<'de> for RawArgs {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ArgsVisitor;
        impl<'de> Visitor<'de> for ArgsVisitor {
            type Value = RawArgs;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("arguments")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<RawArgs, A::Error> {
                let mut list = Vec::new();
                while let Some(expr) = seq.next_element::<Expr>()? {
                    list.push(expr);
                }
                Ok(RawArgs::List(list))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<RawArgs, A::Error> {
                let mut named = Vec::new();
                while let Some(entry) = map.next_entry::<PlaceholderName, Expr>()? {
                    named.push(entry);
                }
                Ok(RawArgs::Named(named))
            }
        }
        deserializer.deserialize_any(ArgsVisitor)
    }
}

/// The largest whole number a literal may be: every one up to it is exact
/// as a float.
const MAX_EXACT: u64 = 1 << 53;

fn bad<E: de::Error>() -> E {
    E::custom(Code::BadExpression.as_str())
}

impl<'de> Deserialize<'de> for Expr {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ExprVisitor;
        impl<'de> Visitor<'de> for ExprVisitor {
            type Value = Expr;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an expression")
            }
            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Expr, E> {
                Ok(Expr::Bool(value))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Expr, E> {
                if value.unsigned_abs() > MAX_EXACT {
                    return Err(E::custom(Code::OutOfRange.as_str()));
                }
                // Exact: |value| ≤ 2^53.
                Ok(Expr::Number(value as f64))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Expr, E> {
                if value > MAX_EXACT {
                    return Err(E::custom(Code::OutOfRange.as_str()));
                }
                // Exact: value ≤ 2^53.
                Ok(Expr::Number(value as f64))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Expr, E> {
                // The same bound as whole literals (2^53 is exact as a float).
                if value.abs() > (1_u64 << 53) as f64 {
                    return Err(E::custom(Code::OutOfRange.as_str()));
                }
                Ok(Expr::Number(value))
            }
            fn visit_str<E: de::Error>(self, raw: &str) -> Result<Expr, E> {
                if raw.len() > limits::TEXT.saturating_mul(16) {
                    return Err(E::custom(Code::TextTooLong.as_str()));
                }
                let text = clean_keeping_ends(raw);
                if text.chars().count() > limits::TEXT {
                    return Err(E::custom(Code::TextTooLong.as_str()));
                }
                Ok(Expr::Str(text))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Expr, A::Error> {
                let mut head: Option<Expr> = None;
                let mut args: Option<RawArgs> = None;
                while let Some(key) = map.next_key::<String>()? {
                    let expr = match key.as_str() {
                        "args" if args.is_none() => {
                            args = Some(map.next_value()?);
                            continue;
                        }
                        "source" => {
                            let path: String = map.next_value()?;
                            let (group, field) = path.split_once('.').ok_or_else(bad)?;
                            Expr::Source {
                                group: group.to_owned(),
                                field: field.to_owned(),
                            }
                        }
                        "knob" => Expr::Knob(map.next_value()?),
                        "item" => Expr::Item(map.next_value()?),
                        "count" => {
                            let path: String = map.next_value()?;
                            let (group, field) = path.split_once('.').ok_or_else(bad)?;
                            Expr::Count {
                                group: group.to_owned(),
                                field: field.to_owned(),
                            }
                        }
                        "if" => Expr::If(Box::new(map.next_value::<[Expr; 3]>()?)),
                        "msg" => Expr::Msg {
                            id: map.next_value()?,
                            args: Vec::new(),
                        },
                        "call" => {
                            let name: String = map.next_value()?;
                            let function = FUNCTIONS
                                .iter()
                                .find(|(n, _)| *n == name)
                                .map(|(_, f)| *f)
                                .ok_or_else(|| de::Error::custom(Code::UnknownFunction.as_str()))?;
                            Expr::Call {
                                function,
                                args: Vec::new(),
                            }
                        }
                        _ => return Err(bad()),
                    };
                    if head.replace(expr).is_some() {
                        return Err(bad());
                    }
                }
                match (head, args) {
                    (Some(Expr::Msg { id, .. }), Some(RawArgs::Named(args))) => {
                        Ok(Expr::Msg { id, args })
                    }
                    (Some(Expr::Call { function, .. }), Some(RawArgs::List(args))) => {
                        Ok(Expr::Call { function, args })
                    }
                    (Some(Expr::Call { .. }) | None, _) | (Some(_), Some(_)) => Err(bad()),
                    (Some(expr), None) => Ok(expr),
                }
            }
            fn visit_seq<A: SeqAccess<'de>>(self, _: A) -> Result<Expr, A::Error> {
                Err(bad())
            }
            fn visit_unit<E: de::Error>(self) -> Result<Expr, E> {
                Err(bad())
            }
        }
        deserializer.deserialize_any(ExprVisitor)
    }
}

/// A literal's type is decided by where it is used: a string is text, or a
/// colour where a colour is wanted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Checked {
    Is(Type),
    StrLiteral,
}

/// The common type of two values that must agree (`if` branches, `eq`
/// sides): a string literal agrees only with text or another literal.
fn unify(a: Checked, b: Checked) -> Result<Checked, Code> {
    match (a, b) {
        (Checked::StrLiteral, Checked::StrLiteral) => Ok(Checked::StrLiteral),
        (Checked::StrLiteral, Checked::Is(Type::Text))
        | (Checked::Is(Type::Text), Checked::StrLiteral) => Ok(Checked::Is(Type::Text)),
        (Checked::Is(x), Checked::Is(y)) if x == y => Ok(a),
        _ => Err(Code::TypeMismatch),
    }
}

/// What a check counts.
struct Budget {
    nodes: usize,
}

/// Checks `expr` where a `want` is expected, reading only what `scope`
/// allows.
///
/// # Errors
///
/// [`Code::TypeMismatch`], [`Code::UnknownSource`],
/// [`Code::UndeclaredSource`], [`Code::UnknownKnob`], [`Code::NotInRepeat`],
/// [`Code::BadExpression`] (wrong argument count,
/// a non-literal time pattern), [`Code::BadColor`] or
/// [`Code::ExpressionTooLarge`].
pub fn check(expr: &Expr, want: Type, scope: &Scope<'_>) -> Result<(), Code> {
    let mut budget = Budget { nodes: 0 };
    check_at(expr, Some(want), scope, &mut budget, 1).map(|_| ())
}

/// What an expression may read: the source groups in `meta.sources`, the
/// knobs (names and value types) and, inside a repeat, its list's items.
#[derive(Debug, Clone, Copy)]
pub struct Scope<'a> {
    sources: &'a BTreeSet<&'a str>,
    knobs: &'a BTreeMap<&'a str, Type>,
    item: Option<&'static Collection>,
}

impl<'a> Scope<'a> {
    /// Reading `sources` and `knobs`, outside any repeat.
    pub fn new(sources: &'a BTreeSet<&'a str>, knobs: &'a BTreeMap<&'a str, Type>) -> Self {
        Self {
            sources,
            knobs,
            item: None,
        }
    }

    /// Whether `group` is listed in `meta.sources`.
    pub fn contains_source(&self, group: &str) -> bool {
        self.sources.contains(group)
    }

    /// Whether this is inside a repeat.
    pub fn in_a_repeat(&self) -> bool {
        self.item.is_some()
    }

    /// The same, inside a repeat over `list`.
    #[must_use]
    pub fn in_repeat(self, list: &'static Collection) -> Self {
        Self {
            item: Some(list),
            ..self
        }
    }
}

fn check_at(
    expr: &Expr,
    want: Option<Type>,
    scope: &Scope<'_>,
    budget: &mut Budget,
    depth: usize,
) -> Result<Checked, Code> {
    budget.nodes += 1;
    if budget.nodes > limits::EXPR_NODES || depth > limits::EXPR_DEPTH {
        return Err(Code::ExpressionTooLarge);
    }
    let got = match expr {
        Expr::Str(text) => {
            if want == Some(Type::Color) {
                Color::parse(text)?;
                Checked::Is(Type::Color)
            } else {
                Checked::StrLiteral
            }
        }
        Expr::Number(_) => Checked::Is(Type::Number),
        Expr::Bool(_) => Checked::Is(Type::Bool),
        Expr::Source { group, field } => {
            let known = SOURCES
                .iter()
                .find(|s| s.group == group && s.field == field)
                .ok_or(Code::UnknownSource)?;
            if !scope.sources.contains(known.group) {
                return Err(Code::UndeclaredSource);
            }
            Checked::Is(known.kind)
        }
        Expr::Knob(name) => Checked::Is(*scope.knobs.get(name.as_str()).ok_or(Code::UnknownKnob)?),
        Expr::Item(field) => {
            let list = scope.item.ok_or(Code::NotInRepeat)?;
            let (_, kind) = list
                .items
                .iter()
                .find(|(name, _)| name == field)
                .ok_or(Code::UnknownSource)?;
            Checked::Is(*kind)
        }
        Expr::Count { group, field } => {
            let list = collection(group, field).ok_or(Code::UnknownSource)?;
            if !scope.sources.contains(list.group) {
                return Err(Code::UndeclaredSource);
            }
            Checked::Is(Type::Number)
        }
        Expr::Msg { args, .. } => {
            for (_, arg) in args {
                let got = check_at(arg, None, scope, budget, depth + 1)?;
                if !matches!(
                    got,
                    Checked::StrLiteral | Checked::Is(Type::Text | Type::Number)
                ) {
                    return Err(Code::TypeMismatch);
                }
            }
            Checked::Is(Type::Text)
        }
        Expr::If(parts) => {
            let [condition, then, otherwise] = &**parts;
            check_at(condition, Some(Type::Bool), scope, budget, depth + 1)?;
            let first = check_at(then, want, scope, budget, depth + 1)?;
            let second_want = match first {
                Checked::Is(t) => Some(t),
                Checked::StrLiteral => want,
            };
            let second = check_at(otherwise, second_want, scope, budget, depth + 1)?;
            unify(first, second)?
        }
        Expr::Call { function, args } => check_call(*function, args, scope, budget, depth)?,
    };
    match (got, want) {
        (_, None) | (Checked::StrLiteral, Some(Type::Text)) => Ok(got),
        (Checked::Is(t), Some(w)) if t == w => Ok(got),
        _ => Err(Code::TypeMismatch),
    }
}

fn check_call(
    function: Function,
    args: &[Expr],
    scope: &Scope<'_>,
    budget: &mut Budget,
    depth: usize,
) -> Result<Checked, Code> {
    use Function as F;
    let each = |want: Type, budget: &mut Budget| -> Result<(), Code> {
        for arg in args {
            check_at(arg, Some(want), scope, budget, depth + 1)?;
        }
        Ok(())
    };
    let count = |ok: bool| if ok { Ok(()) } else { Err(Code::BadExpression) };
    let n = args.len();
    match function {
        F::Not => {
            count(n == 1)?;
            each(Type::Bool, budget)?;
            Ok(Checked::Is(Type::Bool))
        }
        F::And | F::Or => {
            count(n >= 2)?;
            each(Type::Bool, budget)?;
            Ok(Checked::Is(Type::Bool))
        }
        F::Add | F::Sub | F::Mul | F::Div | F::Min | F::Max => {
            count(n == 2)?;
            each(Type::Number, budget)?;
            Ok(Checked::Is(Type::Number))
        }
        F::Round => {
            count(n == 1)?;
            each(Type::Number, budget)?;
            Ok(Checked::Is(Type::Number))
        }
        F::Concat => {
            count(n >= 1)?;
            each(Type::Text, budget)?;
            Ok(Checked::Is(Type::Text))
        }
        F::Text => {
            count(n == 1 || n == 2)?;
            each(Type::Number, budget)?;
            if let Some(decimals) = args.get(1) {
                match decimals {
                    Expr::Number(d) if (0.0..=6.0).contains(d) && d.fract() == 0.0 => {}
                    _ => return Err(Code::BadExpression),
                }
            }
            Ok(Checked::Is(Type::Text))
        }
        F::FormatTime => {
            count(n == 2)?;
            check_at(&args[0], Some(Type::Time), scope, budget, depth + 1)?;
            match &args[1] {
                Expr::Str(pattern) if time_pattern(pattern).is_some() => {}
                _ => return Err(Code::BadExpression),
            }
            budget.nodes += 1;
            Ok(Checked::Is(Type::Text))
        }
        F::Eq | F::Ne | F::Lt | F::Le | F::Gt | F::Ge => {
            count(n == 2)?;
            let first = check_at(&args[0], None, scope, budget, depth + 1)?;
            let second_want = match first {
                Checked::Is(t) => Some(t),
                Checked::StrLiteral => None,
            };
            let second = check_at(&args[1], second_want, scope, budget, depth + 1)?;
            let kind = match unify(first, second)? {
                Checked::Is(t) => t,
                Checked::StrLiteral => Type::Text,
            };
            let ordered = matches!(kind, Type::Number | Type::Time);
            if matches!(function, F::Lt | F::Le | F::Gt | F::Ge) && !ordered {
                return Err(Code::TypeMismatch);
            }
            Ok(Checked::Is(Type::Bool))
        }
    }
}

/// Calls `f` with every message an expression uses and the placeholders
/// it fills.
pub(crate) fn each_message<'a>(
    expr: &'a Expr,
    f: &mut dyn FnMut(&'a MessageId, &'a [(PlaceholderName, Expr)]),
) {
    match expr {
        Expr::Msg { id, args } => {
            f(id, args);
            for (_, arg) in args {
                each_message(arg, f);
            }
        }
        Expr::If(parts) => parts.iter().for_each(|part| each_message(part, f)),
        Expr::Call { args, .. } => args.iter().for_each(|arg| each_message(arg, f)),
        Expr::Str(_)
        | Expr::Number(_)
        | Expr::Bool(_)
        | Expr::Source { .. }
        | Expr::Knob(_)
        | Expr::Item(_)
        | Expr::Count { .. } => {}
    }
}

/// One piece of a time pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimePiece {
    Hour24 { pad: bool },
    Hour12 { pad: bool },
    Minute,
    Second,
    Day { pad: bool },
    Month { pad: bool },
    Year,
    AmPm,
    Literal(char),
}

/// Reads a time pattern: `HH H hh h mm ss dd d MM M yyyy a` and the
/// separators `: . / - ,` and space. Names of days and months need locale
/// data and come with locale formatting.
fn time_pattern(pattern: &str) -> Option<Vec<TimePiece>> {
    const TOKENS: &[(&str, TimePiece)] = &[
        ("yyyy", TimePiece::Year),
        ("HH", TimePiece::Hour24 { pad: true }),
        ("hh", TimePiece::Hour12 { pad: true }),
        ("mm", TimePiece::Minute),
        ("ss", TimePiece::Second),
        ("dd", TimePiece::Day { pad: true }),
        ("MM", TimePiece::Month { pad: true }),
        ("H", TimePiece::Hour24 { pad: false }),
        ("h", TimePiece::Hour12 { pad: false }),
        ("d", TimePiece::Day { pad: false }),
        ("M", TimePiece::Month { pad: false }),
        ("a", TimePiece::AmPm),
    ];
    if pattern.is_empty() || pattern.len() > 32 {
        return None;
    }
    let mut pieces = Vec::new();
    let mut rest = pattern;
    while let Some(c) = rest.chars().next() {
        if let Some((token, piece)) = TOKENS.iter().find(|(t, _)| rest.starts_with(t)) {
            pieces.push(*piece);
            rest = &rest[token.len()..];
        } else if matches!(c, ':' | '.' | '/' | '-' | ',' | ' ') {
            pieces.push(TimePiece::Literal(c));
            rest = &rest[c.len_utf8()..];
        } else {
            return None;
        }
    }
    Some(pieces)
}

/// Days since 1970-01-01 → (year, month, day), proleptic Gregorian
/// (Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    // day is 1..=31 and month 1..=12 by construction.
    (
        year,
        u32::try_from(month).unwrap_or(1),
        u32::try_from(day).unwrap_or(1),
    )
}

fn format_time(time: Moment, pattern: &[TimePiece]) -> Option<String> {
    use std::fmt::Write as _;
    let local = time
        .unix
        .checked_add(i64::from(time.offset_minutes).checked_mul(60)?)?;
    let (days, seconds) = (local.div_euclid(86_400), local.rem_euclid(86_400));
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    let hour12 = if hour % 12 == 0 { 12 } else { hour % 12 };
    let mut out = String::new();
    for piece in pattern {
        // Writing to a String can't fail.
        let _ = match *piece {
            TimePiece::Hour24 { pad: true } => write!(out, "{hour:02}"),
            TimePiece::Hour24 { pad: false } => write!(out, "{hour}"),
            TimePiece::Hour12 { pad: true } => write!(out, "{hour12:02}"),
            TimePiece::Hour12 { pad: false } => write!(out, "{hour12}"),
            TimePiece::Minute => write!(out, "{minute:02}"),
            TimePiece::Second => write!(out, "{second:02}"),
            TimePiece::Day { pad: true } => write!(out, "{day:02}"),
            TimePiece::Day { pad: false } => write!(out, "{day}"),
            TimePiece::Month { pad: true } => write!(out, "{month:02}"),
            TimePiece::Month { pad: false } => write!(out, "{month}"),
            TimePiece::Year => write!(out, "{year:04}"),
            TimePiece::AmPm => write!(out, "{}", if hour < 12 { "AM" } else { "PM" }),
            TimePiece::Literal(c) => write!(out, "{c}"),
        };
    }
    Some(out)
}

/// Cuts `text` to [`limits::TEXT`] characters and cleans it again: every
/// text an expression makes is capped, and joining or cutting cleaned
/// pieces can leave an isolate unpaired or a run of combining marks long.
fn capped(text: &str) -> String {
    let cut = text
        .char_indices()
        .nth(limits::TEXT)
        .map_or(text, |(at, _)| &text[..at]);
    clean_keeping_ends(cut)
}

/// `n` with `decimals` decimals, rounded half away from zero like
/// `round`, ASCII digits, never "-0".
fn number_text(n: f64, decimals: usize) -> Option<String> {
    if !n.is_finite() || n.abs() > 1e15 {
        return None;
    }
    let scale = [1.0, 10.0, 100.0, 1e3, 1e4, 1e5, 1e6]
        .get(decimals)
        .copied()?;
    let rounded = (n * scale).round() / scale;
    let text = format!("{rounded:.decimals$}");
    let negative_zero = text
        .strip_prefix('-')
        .is_some_and(|rest| rest.bytes().all(|b| b == b'0' || b == b'.'));
    Some(if negative_zero {
        text[1..].to_owned()
    } else {
        text
    })
}

/// Evaluates a checked expression. `None` means no value (a source without
/// one, a division by zero, a number out of range or spent `fuel`); the
/// binding then shows its fallback. A string literal evaluates to text;
/// where the binding wants a colour, [`Value::as_color`] turns it into one.
///
/// One evaluation of a checked expression spends at most about twice
/// [`limits::EXPR_NODES`]; the host gives each frame one budget and passes
/// the same `fuel` to every binding it evaluates in that frame.
pub fn eval(expr: &Expr, inputs: &dyn Inputs, fuel: &mut u32) -> Option<Value> {
    *fuel = fuel.checked_sub(1)?;
    let finite = |n: f64| n.is_finite().then_some(Value::Number(n));
    match expr {
        Expr::Str(text) => Some(Value::Text(text.clone())),
        Expr::Number(n) => finite(*n),
        Expr::Bool(b) => Some(Value::Bool(*b)),
        Expr::Source { group, field } => inputs.source(group, field),
        Expr::Knob(name) => inputs.knob(name.as_str()),
        Expr::Item(field) => inputs.item(field),
        Expr::Count { group, field } => {
            // Exact below 2^53; a larger count gives no value. (u64, so the
            // bound means the same on 32-bit targets.)
            let n = u64::try_from(inputs.count(group, field)?).ok()?;
            (n < 1 << 53).then_some(Value::Number(n as f64))
        }
        Expr::Msg { id, args } => {
            // A whole, non-negative number is a count (it picks plural
            // forms); any other value is text.
            enum Owned {
                Count(u64),
                Text(String),
            }
            let mut values = Vec::with_capacity(args.len());
            for (name, arg) in args {
                let owned = match eval(arg, inputs, fuel)? {
                    Value::Number(n) if n >= 0.0 && n.fract() == 0.0 && n < 1e15 => {
                        Owned::Count(n as u64)
                    }
                    Value::Number(n) => Owned::Text(number_text(n, 2)?),
                    Value::Text(text) => Owned::Text(text),
                    _ => return None,
                };
                values.push((name.as_str(), owned));
            }
            let args: Vec<(&str, Arg<'_>)> = values
                .iter()
                .map(|(name, value)| match value {
                    Owned::Count(n) => (*name, Arg::Count(*n)),
                    Owned::Text(text) => (*name, Arg::Text(text)),
                })
                .collect();
            inputs
                .message(id.as_str(), &args)
                .map(|text| Value::Text(capped(&text)))
        }
        Expr::If(parts) => {
            let [condition, then, otherwise] = &**parts;
            match eval(condition, inputs, fuel)? {
                Value::Bool(true) => eval(then, inputs, fuel),
                Value::Bool(false) => eval(otherwise, inputs, fuel),
                _ => None,
            }
        }
        Expr::Call { function, args } => call(*function, args, inputs, fuel),
    }
}

fn call(function: Function, args: &[Expr], inputs: &dyn Inputs, fuel: &mut u32) -> Option<Value> {
    use Function as F;
    let mut values = Vec::with_capacity(args.len());
    // `and` and `or` stop early, like everywhere else.
    for arg in args {
        let value = eval(arg, inputs, fuel)?;
        let stop = matches!(
            (function, &value),
            (F::And, Value::Bool(false)) | (F::Or, Value::Bool(true))
        );
        values.push(value);
        if stop {
            break;
        }
    }
    let number = |i: usize| match values.get(i) {
        Some(Value::Number(n)) => Some(*n),
        _ => None,
    };
    let finite = |n: f64| n.is_finite().then_some(Value::Number(n));
    match function {
        F::Not => match values.first()? {
            Value::Bool(b) => Some(Value::Bool(!b)),
            _ => None,
        },
        F::And => Some(Value::Bool(values.iter().all(|v| *v == Value::Bool(true)))),
        F::Or => Some(Value::Bool(values.contains(&Value::Bool(true)))),
        F::Eq | F::Ne => {
            let same = match (values.first()?, values.get(1)?) {
                // The same moment, whatever offset each is shown in.
                (Value::Time(a), Value::Time(b)) => a.unix == b.unix,
                // A colour literal evaluates to text; compare it as a colour.
                (a @ Value::Color(_), b) | (b, a @ Value::Color(_)) => {
                    a.as_color().is_some() && a.as_color() == b.as_color()
                }
                (a, b) => a == b,
            };
            Some(Value::Bool(same == (function == F::Eq)))
        }
        F::Lt | F::Le | F::Gt | F::Ge => {
            let ordering = match (values.first()?, values.get(1)?) {
                (Value::Number(a), Value::Number(b)) => a.partial_cmp(b)?,
                (Value::Time(a), Value::Time(b)) => a.unix.cmp(&b.unix),
                _ => return None,
            };
            let yes = match function {
                F::Lt => ordering.is_lt(),
                F::Le => ordering.is_le(),
                F::Gt => ordering.is_gt(),
                _ => ordering.is_ge(),
            };
            Some(Value::Bool(yes))
        }
        F::Add => finite(number(0)? + number(1)?),
        F::Sub => finite(number(0)? - number(1)?),
        F::Mul => finite(number(0)? * number(1)?),
        F::Div => finite(number(0)? / number(1)?),
        F::Min => finite(number(0)?.min(number(1)?)),
        F::Max => finite(number(0)?.max(number(1)?)),
        F::Round => finite(number(0)?.round()),
        F::Concat => {
            let mut out = String::new();
            for value in &values {
                match value {
                    Value::Text(text) => out.push_str(text),
                    _ => return None,
                }
                if out.len() > limits::TEXT * 4 {
                    break;
                }
            }
            Some(Value::Text(capped(&out)))
        }
        F::Text => {
            let decimals = number(1).map_or(0, |d| d.clamp(0.0, 6.0) as usize);
            number_text(number(0)?, decimals).map(Value::Text)
        }
        F::FormatTime => {
            let (Value::Time(time), Some(Expr::Str(pattern))) = (values.first()?, args.get(1))
            else {
                return None;
            };
            format_time(*time, &time_pattern(pattern)?).map(Value::Text)
        }
    }
}

impl Value {
    /// The colour a binding's value stands for, if it is one: a colour, or
    /// a checked colour literal.
    pub fn as_color(&self) -> Option<Color> {
        match self {
            Self::Color(color) => Some(*color),
            Self::Text(text) => Color::parse(text).ok(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expr(json: &str) -> Result<Expr, Code> {
        serde_json::from_str(json).map_err(|e| crate::PackageError::from_typed_json(&e).code)
    }

    fn checked(json: &str, want: Type) -> Result<Expr, Code> {
        let expr = expr(json)?;
        let sources = BTreeSet::from(["clock", "network", "history"]);
        let knobs = BTreeMap::from([("size", Type::Number), ("accent", Type::Color)]);
        check(&expr, want, &Scope::new(&sources, &knobs)).map(|()| expr)
    }

    struct Fake;
    impl Inputs for Fake {
        fn source(&self, group: &str, field: &str) -> Option<Value> {
            match (group, field) {
                ("network", "connected") => Some(Value::Bool(false)),
                ("history", "waiting") => Some(Value::Number(3.0)),
                ("clock", "now") => Some(Value::Time(Moment {
                    // 2026-11-24T21:05:09Z
                    unix: 1_795_554_309,
                    offset_minutes: 120,
                })),
                _ => None,
            }
        }
        fn knob(&self, name: &str) -> Option<Value> {
            (name == "size").then_some(Value::Number(4.0))
        }
        fn item(&self, field: &str) -> Option<Value> {
            (field == "name").then(|| Value::Text("YouTube".into()))
        }
        fn count(&self, group: &str, _: &str) -> Option<usize> {
            (group == "apps").then_some(7)
        }
        fn message(&self, id: &str, args: &[(&str, Arg<'_>)]) -> Option<String> {
            match (id, args) {
                ("unread", [("count", Arg::Count(n))]) => Some(format!("{n} waiting")),
                _ => None,
            }
        }
    }

    fn run(json: &str, want: Type) -> Option<Value> {
        let expr = checked(json, want).unwrap();
        eval(&expr, &Fake, &mut 1000)
    }

    fn text(s: &str) -> Option<Value> {
        Some(Value::Text(s.to_owned()))
    }

    #[test]
    fn literals_sources_and_ifs_evaluate() {
        assert_eq!(run(r#""hi""#, Type::Text), text("hi"));
        assert_eq!(
            run(r#"{"source": "history.waiting"}"#, Type::Number),
            Some(Value::Number(3.0))
        );
        let colour = r##"{"if": [{"source": "network.connected"}, "#ffffff", "#888888"]}"##;
        let value = run(colour, Type::Color).unwrap();
        assert_eq!(value.as_color(), Color::parse("#888888").ok());
        assert_eq!(run(r#"{"source": "clock.zone"}"#, Type::Text), None);
    }

    #[test]
    fn functions_evaluate() {
        let call = |f: &str, args: &str| format!(r#"{{"call": "{f}", "args": [{args}]}}"#);
        assert_eq!(
            run(&call("not", "false"), Type::Bool),
            Some(Value::Bool(true))
        );
        assert_eq!(
            run(&call("and", "true, false"), Type::Bool),
            Some(Value::Bool(false))
        );
        assert_eq!(
            run(
                &call("or", r#"false, {"source": "network.connected"}"#),
                Type::Bool
            ),
            Some(Value::Bool(false))
        );
        assert_eq!(
            run(
                &call("gt", r#"{"source": "history.waiting"}, 2"#),
                Type::Bool
            ),
            Some(Value::Bool(true))
        );
        assert_eq!(
            run(&call("eq", r#""a", "a""#), Type::Bool),
            Some(Value::Bool(true))
        );
        assert_eq!(run(&call("div", "1, 0"), Type::Number), None);
        assert_eq!(
            run(&call("add", "1, 2"), Type::Number),
            Some(Value::Number(3.0))
        );
        assert_eq!(
            run(&call("round", "2.5"), Type::Number),
            Some(Value::Number(3.0))
        );
        assert_eq!(run(&call("text", "2.345, 1"), Type::Text), text("2.3"));
        assert_eq!(run(&call("text", "-0.0"), Type::Text), text("0"));
        assert_eq!(
            run(&call("concat", r#""a", " · ", "b""#), Type::Text),
            text("a · b")
        );
        let time = |pattern: &str| {
            run(
                &call(
                    "format_time",
                    &format!(r#"{{"source": "clock.now"}}, "{pattern}""#),
                ),
                Type::Text,
            )
        };
        assert_eq!(time("HH:mm"), text("23:05"));
        assert_eq!(time("h:mm a"), text("11:05 PM"));
        assert_eq!(time("dd.MM.yyyy"), text("24.11.2026"));
        assert_eq!(time("d/M ss"), text("24/11 09"));
    }

    #[test]
    fn messages_get_counts_and_texts() {
        let msg = r#"{"msg": "unread", "args": {"count": {"source": "history.waiting"}}}"#;
        assert_eq!(run(msg, Type::Text), text("3 waiting"));
        assert_eq!(run(r#"{"msg": "other"}"#, Type::Text), None);
    }

    #[test]
    fn types_are_checked() {
        let mismatch = |json, want| checked(json, want).err();
        assert_eq!(
            mismatch(r#"{"source": "network.connected"}"#, Type::Text),
            Some(Code::TypeMismatch)
        );
        assert_eq!(mismatch(r#""hello""#, Type::Color), Some(Code::BadColor));
        assert_eq!(
            mismatch(r##"{"if": [true, "#ffffff", 3]}"##, Type::Color),
            Some(Code::TypeMismatch)
        );
        assert_eq!(
            mismatch(r#"{"if": [1, "a", "b"]}"#, Type::Text),
            Some(Code::TypeMismatch)
        );
        assert_eq!(
            mismatch(r#"{"call": "lt", "args": ["a", "b"]}"#, Type::Bool),
            Some(Code::TypeMismatch)
        );
        assert_eq!(
            mismatch(r#"{"call": "eq", "args": [1, "a"]}"#, Type::Bool),
            Some(Code::TypeMismatch)
        );
        assert_eq!(
            mismatch(r#"{"call": "concat", "args": [1]}"#, Type::Text),
            Some(Code::TypeMismatch)
        );
        assert_eq!(
            mismatch(r#"{"call": "not", "args": [true, false]}"#, Type::Bool),
            Some(Code::BadExpression)
        );
        assert_eq!(
            mismatch(r#"{"call": "text", "args": [1, 9]}"#, Type::Text),
            Some(Code::BadExpression)
        );
        assert_eq!(
            mismatch(
                r#"{"call": "format_time", "args": [{"source": "clock.now"}, "EEEE"]}"#,
                Type::Text
            ),
            Some(Code::BadExpression)
        );
        assert_eq!(
            mismatch(r#"{"msg": "a", "args": {"x": true}}"#, Type::Text),
            Some(Code::TypeMismatch)
        );
    }

    #[test]
    fn colours_compare_as_colours() {
        struct Accent;
        impl Inputs for Accent {
            fn source(&self, _: &str, _: &str) -> Option<Value> {
                None
            }
            fn knob(&self, _: &str) -> Option<Value> {
                Some(Value::Color(Color::parse("#3478f6").ok()?))
            }
            fn item(&self, _: &str) -> Option<Value> {
                None
            }
            fn count(&self, _: &str, _: &str) -> Option<usize> {
                None
            }
            fn message(&self, _: &str, _: &[(&str, Arg<'_>)]) -> Option<String> {
                None
            }
        }
        let same = |literal: &str| {
            let json = format!(r#"{{"call": "eq", "args": [{{"knob": "accent"}}, "{literal}"]}}"#);
            eval(&checked(&json, Type::Bool).unwrap(), &Accent, &mut 10)
        };
        assert_eq!(same("#3478f6"), Some(Value::Bool(true)));
        assert_eq!(same("#3478F6ff"), Some(Value::Bool(true)));
        assert_eq!(same("#000000"), Some(Value::Bool(false)));
    }

    #[test]
    fn items_are_read_only_inside_a_repeat_and_counts_need_their_group() {
        let sources = BTreeSet::from(["apps", "clock"]);
        let knobs = BTreeMap::new();
        let outside = Scope::new(&sources, &knobs);
        let inside = outside.in_repeat(collection("apps", "list").unwrap());
        let check_in =
            |json: &str, want, scope: &Scope<'_>| check(&expr(json).unwrap(), want, scope);
        assert_eq!(check_in(r#"{"item": "name"}"#, Type::Text, &inside), Ok(()));
        assert_eq!(
            check_in(r#"{"item": "name"}"#, Type::Text, &outside),
            Err(Code::NotInRepeat)
        );
        assert_eq!(
            check_in(r#"{"item": "icon"}"#, Type::Text, &inside),
            Err(Code::UnknownSource)
        );
        assert_eq!(
            check_in(r#"{"item": "name"}"#, Type::Number, &inside),
            Err(Code::TypeMismatch)
        );
        assert_eq!(
            check_in(r#"{"count": "apps.list"}"#, Type::Number, &outside),
            Ok(())
        );
        assert_eq!(
            check_in(r#"{"count": "phones.list"}"#, Type::Number, &outside),
            Err(Code::UndeclaredSource)
        );
        assert_eq!(
            check_in(r#"{"count": "apps.grid"}"#, Type::Number, &outside),
            Err(Code::UnknownSource)
        );
        assert_eq!(expr(r#"{"count": "apps"}"#), Err(Code::BadExpression));
        assert_eq!(expr(r#"{"item": 5}"#), Err(Code::WrongType));
        assert_eq!(
            check_in(r#"{"count": "foo.list"}"#, Type::Number, &outside),
            Err(Code::UnknownSource)
        );
        let phones = BTreeSet::from(["phones"]);
        let with_phones = Scope::new(&phones, &knobs);
        assert_eq!(
            check_in(r#"{"count": "phones.list"}"#, Type::Number, &with_phones),
            Ok(())
        );
        assert!(!outside.in_a_repeat() && inside.in_a_repeat());
        assert!(outside.contains_source("apps") && !outside.contains_source("phones"));
        assert_eq!(source_tier("phones"), Some(2));
        assert!(collection("apps", "grid").is_none());
        // A host with no count gives no value: the fallback shows.
        let no_count = Pair([Value::Bool(true), Value::Bool(true)]);
        assert_eq!(
            eval(
                &expr(r#"{"count": "apps.list"}"#).unwrap(),
                &no_count,
                &mut 10
            ),
            None
        );
        let none_waiting = eval(
            &expr(r#"{"call": "eq", "args": [{"count": "apps.list"}, 0]}"#).unwrap(),
            &Fake,
            &mut 10,
        );
        assert_eq!(none_waiting, Some(Value::Bool(false)));
        assert_eq!(
            eval(&expr(r#"{"item": "name"}"#).unwrap(), &Fake, &mut 10),
            text("YouTube")
        );
    }

    #[test]
    fn knobs_are_read_by_name_and_type() {
        assert_eq!(
            run(
                r#"{"call": "mul", "args": [{"knob": "size"}, 2]}"#,
                Type::Number
            ),
            Some(Value::Number(8.0))
        );
        assert!(checked(r#"{"knob": "accent"}"#, Type::Color).is_ok());
        let cases = [
            (r#"{"knob": "nope"}"#, Type::Number, Code::UnknownKnob),
            (r#"{"knob": "accent"}"#, Type::Text, Code::TypeMismatch),
            (r#"{"knob": "Size"}"#, Type::Number, Code::BadName),
            (
                r#"{"knob": "size", "args": []}"#,
                Type::Number,
                Code::BadExpression,
            ),
        ];
        for (json, want, code) in cases {
            assert_eq!(checked(json, want).err(), Some(code), "{json}");
        }
        assert_eq!(
            run(
                r#"{"if": [{"call": "gt", "args": [{"knob": "size"}, 3]}, {"msg": "unread", "args": {"count": {"knob": "size"}}}, "no"]}"#,
                Type::Text
            ),
            text("4 waiting")
        );
        // A knob with no value (the host has none) shows the fallback.
        assert_eq!(
            eval(
                &checked(r#"{"knob": "accent"}"#, Type::Color).unwrap(),
                &Fake,
                &mut 10
            ),
            None
        );
    }

    #[test]
    fn sources_must_exist_and_be_declared() {
        assert_eq!(
            checked(r#"{"source": "network.ssid"}"#, Type::Text).err(),
            Some(Code::UnknownSource)
        );
        assert_eq!(
            checked(r#"{"source": "storage.free"}"#, Type::Number).err(),
            Some(Code::UndeclaredSource)
        );
        assert_eq!(
            checked(r#"{"source": "network"}"#, Type::Bool).err(),
            Some(Code::BadExpression)
        );
        assert!(sources().iter().all(|s| is_source_group(s.group)));
        assert_eq!(source_tier("history"), Some(2));
        assert_eq!(source_tier("apps"), Some(2));
        assert!(is_source_group("apps") && !is_source_group("weather"));
        assert!(collections().iter().all(|c| is_source_group(c.group)));
    }

    #[test]
    fn bad_shapes_are_refused() {
        for json in [
            "null",
            "[1, 2]",
            "{}",
            r#"{"source": "clock.now", "extra": 1}"#,
            r#"{"if": [true, "a"]}"#,
            r#"{"call": "not"}"#,
            r#"{"call": "not", "args": {"a": true}}"#,
            r#"{"msg": "a", "args": [1]}"#,
            r#"{"weird": 1}"#,
        ] {
            assert_eq!(
                expr(json)
                    .err()
                    .map(|c| c == Code::BadExpression || c == Code::WrongType),
                Some(true),
                "{json}"
            );
        }
        assert_eq!(
            expr(r#"{"call": "exec", "args": []}"#).err(),
            Some(Code::UnknownFunction)
        );
    }

    #[test]
    fn size_depth_and_fuel_are_bounded() {
        let mut deep = "true".to_owned();
        for _ in 0..limits::EXPR_DEPTH {
            deep = format!(r#"{{"call": "not", "args": [{deep}]}}"#);
        }
        assert_eq!(
            checked(&deep, Type::Bool).err(),
            Some(Code::ExpressionTooLarge)
        );
        let wide = format!(
            r#"{{"call": "and", "args": [{}]}}"#,
            vec!["true"; limits::EXPR_NODES].join(",")
        );
        assert_eq!(
            checked(&wide, Type::Bool).err(),
            Some(Code::ExpressionTooLarge)
        );
        let small = checked(r#"{"call": "and", "args": [true, true, true]}"#, Type::Bool).unwrap();
        assert_eq!(eval(&small, &Fake, &mut 3), None);
        assert_eq!(eval(&small, &Fake, &mut 4), Some(Value::Bool(true)));
    }

    #[test]
    fn texts_are_capped_and_numbers_bounded() {
        let long = format!(
            r#"{{"call": "concat", "args": [{}]}}"#,
            vec![format!(r#""{}""#, "a".repeat(400)); 4].join(",")
        );
        let Some(Value::Text(out)) = run(&long, Type::Text) else {
            panic!()
        };
        assert_eq!(out.chars().count(), limits::TEXT);
        assert_eq!(
            run(
                r#"{"call": "text", "args": [{"call": "mul", "args": [1e8, 1e8]}]}"#,
                Type::Text
            ),
            None
        );
        assert_eq!(expr("1e300").err(), Some(Code::OutOfRange));
    }

    #[test]
    fn literals_agree_only_with_text() {
        let mismatch = |json| checked(json, Type::Bool).err();
        assert_eq!(
            mismatch(r#"{"call": "eq", "args": ["a", 5]}"#),
            Some(Code::TypeMismatch)
        );
        assert_eq!(
            mismatch(r#"{"call": "lt", "args": ["a", 5]}"#),
            Some(Code::TypeMismatch)
        );
        assert_eq!(
            mismatch(r#"{"call": "eq", "args": [{"if": [true, "a", 5]}, 5]}"#),
            Some(Code::TypeMismatch)
        );
        assert_eq!(
            mismatch(r#"{"call": "eq", "args": ["a", {"source": "clock.zone"}]}"#),
            None
        );
        assert_eq!(
            expr(&format!("{}", (1u64 << 53) + 1)).err(),
            Some(Code::OutOfRange)
        );
        assert_eq!(expr("18446744073709551616").err(), Some(Code::OutOfRange));
        assert_eq!(expr("-1e20").err(), Some(Code::OutOfRange));
        assert_eq!(expr("0.5").map(|_| ()), Ok(()));
        assert_eq!(
            expr(r#"{"args": [true], "call": "not"}"#).map(|_| ()),
            Ok(())
        );
    }

    #[test]
    fn numbers_round_half_away_and_never_show_minus_zero() {
        assert_eq!(number_text(2.5, 0).as_deref(), Some("3"));
        assert_eq!(number_text(-0.0, 2).as_deref(), Some("0.00"));
        assert_eq!(number_text(-0.04, 1).as_deref(), Some("0.0"));
        assert_eq!(number_text(-1.25, 1).as_deref(), Some("-1.3"));
    }

    #[test]
    fn made_texts_are_cleaned_again() {
        let marks = format!(
            r#"{{"call": "concat", "args": [{}]}}"#,
            [r#""́́́́""#; 4].join(",")
        );
        let Some(Value::Text(out)) = run(
            &format!(r#"{{"call": "concat", "args": ["a", {marks}]}}"#),
            Type::Text,
        ) else {
            panic!()
        };
        assert_eq!(out.chars().filter(|c| *c == '\u{0301}').count(), 4);
        assert_eq!(
            capped(&format!(
                "{}\u{2068}x\u{2069}",
                "a".repeat(limits::TEXT - 1)
            ))
            .chars()
            .count(),
            limits::TEXT - 1
        );
    }

    #[test]
    fn times_compare_by_moment_and_format_at_midnight() {
        let at = |unix, offset_minutes| {
            Value::Time(Moment {
                unix,
                offset_minutes,
            })
        };
        let source = |field: &str| Expr::Source {
            group: "t".to_owned(),
            field: field.to_owned(),
        };
        let same = call(
            Function::Eq,
            &[source("a"), source("b")],
            &Pair([at(0, 0), at(0, 120)]),
            &mut 10,
        );
        assert_eq!(same, Some(Value::Bool(true)));
        let midnight = Moment {
            unix: 0,
            offset_minutes: 0,
        };
        let pattern = time_pattern("h:mm a hh").unwrap();
        assert_eq!(
            format_time(midnight, &pattern).as_deref(),
            Some("12:00 AM 12")
        );
        let before = Moment {
            unix: -1,
            offset_minutes: 0,
        };
        assert_eq!(
            format_time(before, &time_pattern("yyyy-MM-dd HH:mm:ss").unwrap()).as_deref(),
            Some("1969-12-31 23:59:59")
        );
        let crossing = Moment {
            unix: 23 * 3600,
            offset_minutes: 90,
        };
        assert_eq!(
            format_time(crossing, &time_pattern("d HH:mm").unwrap()).as_deref(),
            Some("2 00:30")
        );
    }

    /// Two time sources, for comparing them.
    struct Pair([Value; 2]);
    impl Inputs for Pair {
        fn source(&self, _: &str, field: &str) -> Option<Value> {
            self.0.get(usize::from(field == "b")).cloned()
        }
        fn knob(&self, _: &str) -> Option<Value> {
            None
        }
        fn item(&self, _: &str) -> Option<Value> {
            None
        }
        fn count(&self, _: &str, _: &str) -> Option<usize> {
            None
        }
        fn message(&self, _: &str, _: &[(&str, Arg<'_>)]) -> Option<String> {
            None
        }
    }

    #[test]
    fn dates_are_right_far_from_1970() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(20_781), (2026, 11, 24));
        let far = Moment {
            unix: i64::MAX,
            offset_minutes: 60,
        };
        assert_eq!(format_time(far, &[TimePiece::Year]), None);
    }
}
