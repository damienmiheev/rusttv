//! Converts a panel template (JSON, PANELS.md § 4.1) into one Slint component.
//!
//! The schema holds only what both surfaces (Slint on the TV, the phone page)
//! can draw: unknown fields, colours, badges and properties are errors, so a
//! template can't say anything the converter would silently drop.

use serde_json::{Map, Value};
use std::fmt::Write as _;

/// Colour tokens: `Theme.<token>` in Slint, a CSS variable on the phone.
const TOKENS: [&str; 5] = ["bg", "surface", "text", "text-dim", "accent"];

/// Names the generated component's own properties already use.
const RESERVED_PROPS: [&str; 16] = [
    "u",
    "width",
    "height",
    "x",
    "y",
    "z",
    "background",
    "border-radius",
    "border-width",
    "border-color",
    "clip",
    "visible",
    "opacity",
    "cache-rendering-hint",
    "absolute-position",
    "accessible-role",
];

/// Names a component must not shadow.
const RESERVED_COMPONENTS: [&str; 8] = [
    "Theme",
    "Dir",
    "Text",
    "Image",
    "Rectangle",
    "Window",
    "VerticalLayout",
    "HorizontalLayout",
];

/// Converts `json` into Slint source. The output imports `Theme` and `Dir`
/// from "theme.slint" and `Badges` from "badges.slint" by name, found through
/// the compiler's include paths, so every file shares the one `Dir.rtl`.
/// `badges` are the names of the pictures in `Badges` (set from Rust);
/// nothing else can be an image.
pub fn convert(json: &str, badges: &[&str]) -> Result<String, String> {
    let doc = parse(json)?;
    let doc = object(&doc, "template")?;
    only(doc, "template", &["component", "props", "root"])?;
    let name = string(field(doc, "template", "component")?, "component")?;
    if !name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        || !name.chars().all(|c| c.is_ascii_alphanumeric())
        || RESERVED_COMPONENTS.contains(&name)
    {
        return Err(format!(
            "component: {name:?} must be an UpperCamelCase name"
        ));
    }
    let mut props = Vec::new();
    for (i, prop) in array(field(doc, "template", "props")?, "props")?
        .iter()
        .enumerate()
    {
        let prop = string(prop, &format!("props[{i}]"))?;
        let shaped = prop.starts_with(|c: char| c.is_ascii_lowercase())
            && prop.ends_with(|c: char| c.is_ascii_lowercase())
            && prop.chars().all(|c| c.is_ascii_lowercase() || c == '-')
            && !prop.contains("--");
        if !shaped {
            return Err(format!(
                "props[{i}]: {prop:?} must be lowercase words joined by '-'"
            ));
        }
        if RESERVED_PROPS.contains(&prop) || props.contains(&prop) {
            return Err(format!(
                "props[{i}]: {prop:?} is taken (a Rectangle property, `u`, or a duplicate)"
            ));
        }
        props.push(prop);
    }
    let cx = Cx {
        props: &props,
        badges,
    };

    let root = field(doc, "template", "root")?;
    let (kind, body) = node(root, "root")?;
    if kind != "column" {
        return Err("root: must be a column".into());
    }
    let mut out = String::from("// Generated from a panel template by build.rs. Do not edit.\n");
    out.push_str("import { Theme, Dir } from \"theme.slint\";\n");
    out.push_str("import { Badges } from \"badges.slint\";\n\n");
    let _ = writeln!(out, "export component {name} inherits Rectangle {{");
    out.push_str("    in property <length> u;\n");
    for prop in &props {
        let _ = writeln!(out, "    in property <string> {prop};");
    }
    let _ = writeln!(
        out,
        "    width: {} * u;",
        number(field(body, "root.column", "width")?, "root.column.width")?
    );
    let _ = writeln!(
        out,
        "    height: {} * u;",
        number(field(body, "root.column", "height")?, "root.column.height")?
    );
    if let Some(radius) = body.get("radius") {
        let _ = writeln!(
            out,
            "    border-radius: {} * u;",
            number(radius, "root.column.radius")?
        );
    }
    if let Some(background) = body.get("background") {
        let _ = writeln!(
            out,
            "    background: {};",
            cx.color(background, "root.column.background")?
        );
    }
    out.push_str("    clip: true;\n");
    // Drawn once into a cached layer (where the renderer supports it), so a
    // swaying tile moves a picture instead of laying out text every frame.
    out.push_str("    cache-rendering-hint: true;\n\n");
    cx.column(body, "root.column", 1, &mut out, true)?;
    out.push_str("}\n");
    Ok(out)
}

struct Cx<'a> {
    props: &'a [&'a str],
    badges: &'a [&'a str],
}

impl Cx<'_> {
    fn column(
        &self,
        body: &Map<String, Value>,
        at: &str,
        depth: usize,
        out: &mut String,
        root: bool,
    ) -> Result<(), String> {
        let allowed: &[&str] = if root {
            &[
                "width",
                "height",
                "radius",
                "background",
                "padding",
                "gap",
                "align",
                "children",
            ]
        } else {
            &["padding", "gap", "align", "children"]
        };
        only(body, at, allowed)?;
        let pad = "    ".repeat(depth);
        let _ = writeln!(out, "{pad}VerticalLayout {{");
        if let Some(padding) = body.get("padding") {
            let _ = writeln!(
                out,
                "{pad}    padding: {} * u;",
                number(padding, &format!("{at}.padding"))?
            );
        }
        if let Some(gap) = body.get("gap") {
            let _ = writeln!(
                out,
                "{pad}    spacing: {} * u;",
                number(gap, &format!("{at}.gap"))?
            );
        }
        if let Some(align) = body.get("align") {
            let align = string(align, &format!("{at}.align"))?;
            if !["start", "center", "end"].contains(&align) {
                return Err(format!("{at}.align: {align:?} is not start, center or end"));
            }
            let _ = writeln!(out, "{pad}    alignment: {align};");
        }
        out.push('\n');
        for (i, child) in array(field(body, at, "children")?, &format!("{at}.children"))?
            .iter()
            .enumerate()
        {
            let at = format!("{at}.children[{i}]");
            let (kind, child) = node(child, &at)?;
            let at = format!("{at}.{kind}");
            match kind {
                "column" => self.column(child, &at, depth + 1, out, false)?,
                "text" => self.text(child, &at, depth + 1, out)?,
                "image" => self.image(child, &at, depth + 1, out)?,
                other => {
                    return Err(format!(
                        "{at}: unknown node {other:?} (column, text, image)"
                    ));
                }
            }
        }
        let _ = writeln!(out, "{pad}}}");
        Ok(())
    }

    fn text(
        &self,
        body: &Map<String, Value>,
        at: &str,
        depth: usize,
        out: &mut String,
    ) -> Result<(), String> {
        only(body, at, &["value", "size", "weight", "color", "show"])?;
        let pad = "    ".repeat(depth);
        let show = match body.get("show") {
            Some(show) => {
                let prop = self.prop(show, &format!("{at}.show"))?;
                format!("if root.{prop} != \"\": ")
            }
            None => String::new(),
        };
        let _ = writeln!(out, "{pad}{show}Text {{");
        let _ = writeln!(
            out,
            "{pad}    text: {};",
            self.words(field(body, at, "value")?, &format!("{at}.value"))?
        );
        let _ = writeln!(
            out,
            "{pad}    font-size: {} * u;",
            number(field(body, at, "size")?, &format!("{at}.size"))?
        );
        if let Some(weight) = body.get("weight") {
            let weight = number(weight, &format!("{at}.weight"))?;
            if !(100.0..=900.0).contains(&weight) || weight % 100.0 != 0.0 {
                return Err(format!("{at}.weight: {weight} is not 100, 200 … 900"));
            }
            let _ = writeln!(out, "{pad}    font-weight: {weight};");
        }
        let _ = writeln!(
            out,
            "{pad}    color: {};",
            self.color(field(body, at, "color")?, &format!("{at}.color"))?
        );
        // Text starts at the reading side; never left or right.
        let _ = writeln!(out, "{pad}    horizontal-alignment: Dir.start;");
        let _ = writeln!(out, "{pad}    overflow: elide;");
        let _ = writeln!(out, "{pad}}}");
        Ok(())
    }

    fn image(
        &self,
        body: &Map<String, Value>,
        at: &str,
        depth: usize,
        out: &mut String,
    ) -> Result<(), String> {
        only(body, at, &["badge", "height"])?;
        let pad = "    ".repeat(depth);
        let badge = string(field(body, at, "badge")?, &format!("{at}.badge"))?;
        if !self.badges.contains(&badge) {
            return Err(format!("{at}.badge: {badge:?} is not a built-in badge"));
        }
        let _ = writeln!(out, "{pad}Image {{");
        let _ = writeln!(out, "{pad}    source: Badges.{badge};");
        let _ = writeln!(
            out,
            "{pad}    height: {} * u;",
            number(field(body, at, "height")?, &format!("{at}.height"))?
        );
        let _ = writeln!(out, "{pad}    image-fit: contain;");
        let _ = writeln!(out, "{pad}    horizontal-alignment: Dir.image-start;");
        let _ = writeln!(out, "{pad}}}");
        Ok(())
    }

    /// Text: `{"tr": "literal {}", "args": [props]}` or a `select` of those.
    fn words(&self, value: &Value, at: &str) -> Result<String, String> {
        let body = object(value, at)?;
        if body.contains_key("select") {
            return self.select(body, at, |v, at| self.words(v, at), "\"\"".into());
        }
        only(body, at, &["tr", "args"])?;
        let literal = string(field(body, at, "tr")?, &format!("{at}.tr"))?;
        // No control or bidi characters: a text can't hide or reorder words
        // (HISTORY.md § 8.5's rule for names on the TV).
        if literal.chars().any(|c| {
            c.is_control() || matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
        }) {
            return Err(format!("{at}.tr: control or bidi characters are not allowed"));
        }
        let args = match body.get("args") {
            Some(args) => array(args, &format!("{at}.args"))?
                .iter()
                .enumerate()
                .map(|(i, arg)| self.prop(arg, &format!("{at}.args[{i}]")))
                .collect::<Result<Vec<_>, String>>()?,
            None => Vec::new(),
        };
        // `{}` is a placeholder; any other brace would be one too in `@tr`.
        if literal.replace("{}", "").contains(['{', '}']) {
            return Err(format!("{at}.tr: {literal:?} may hold braces only as {{}}"));
        }
        if literal.matches("{}").count() != args.len() {
            return Err(format!("{at}: {literal:?} needs one arg per {{}}"));
        }
        let args: String = args.iter().map(|p| format!(", root.{p}")).collect();
        Ok(format!("@tr({}{args})", quote(literal)))
    }

    /// Colour: a token name or a `select` of token names.
    fn color(&self, value: &Value, at: &str) -> Result<String, String> {
        if let Some(body) = value.as_object() {
            return self.select(body, at, |v, at| self.color(v, at), "Theme.text".into());
        }
        let token = string(value, at)?;
        if !TOKENS.contains(&token) {
            return Err(format!(
                "{at}: {token:?} is not a colour token ({})",
                TOKENS.join(", ")
            ));
        }
        Ok(format!("Theme.{token}"))
    }

    /// `{"select": prop, "cases": {value: x}, "else": x}` as nested `?:`.
    fn select(
        &self,
        body: &Map<String, Value>,
        at: &str,
        each: impl Fn(&Value, &str) -> Result<String, String>,
        fallback: String,
    ) -> Result<String, String> {
        only(body, at, &["select", "cases", "else"])?;
        let prop = self.prop(field(body, at, "select")?, &format!("{at}.select"))?;
        let cases = object(field(body, at, "cases")?, &format!("{at}.cases"))?;
        let mut out = String::new();
        for (case, value) in cases {
            let _ = write!(
                out,
                "root.{prop} == {} ? {} : ",
                quote(case),
                each(value, &format!("{at}.cases.{case}"))?
            );
        }
        out.push_str(&match body.get("else") {
            Some(value) => each(value, &format!("{at}.else"))?,
            None => fallback,
        });
        Ok(format!("({out})"))
    }

    fn prop<'v>(&self, value: &'v Value, at: &str) -> Result<&'v str, String> {
        let prop = string(value, at)?;
        if self.props.contains(&prop) {
            Ok(prop)
        } else {
            Err(format!("{at}: {prop:?} is not in props"))
        }
    }
}

/// A node is an object with exactly one key, its kind.
fn node<'v>(value: &'v Value, at: &str) -> Result<(&'v str, &'v Map<String, Value>), String> {
    let map = object(value, at)?;
    let mut entries = map.iter();
    match (entries.next(), entries.next()) {
        (Some((kind, body)), None) => Ok((kind, object(body, &format!("{at}.{kind}"))?)),
        _ => Err(format!("{at}: a node has exactly one key, its kind")),
    }
}

fn only(map: &Map<String, Value>, at: &str, allowed: &[&str]) -> Result<(), String> {
    match map.keys().find(|key| !allowed.contains(&key.as_str())) {
        Some(key) => Err(format!(
            "{at}: unknown field {key:?} (allowed: {})",
            allowed.join(", ")
        )),
        None => Ok(()),
    }
}

fn field<'v>(map: &'v Map<String, Value>, at: &str, key: &str) -> Result<&'v Value, String> {
    map.get(key).ok_or_else(|| format!("{at}: missing {key:?}"))
}

fn object<'v>(value: &'v Value, at: &str) -> Result<&'v Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{at}: expected an object"))
}

fn array<'v>(value: &'v Value, at: &str) -> Result<&'v Vec<Value>, String> {
    value
        .as_array()
        .ok_or_else(|| format!("{at}: expected an array"))
}

fn string<'v>(value: &'v Value, at: &str) -> Result<&'v str, String> {
    value
        .as_str()
        .ok_or_else(|| format!("{at}: expected a string"))
}

/// Sizes are plain non-negative numbers in `u`.
fn number(value: &Value, at: &str) -> Result<f64, String> {
    match value.as_f64() {
        Some(n) if (0.0..=4000.0).contains(&n) => Ok(n),
        _ => Err(format!("{at}: expected a number from 0 to 4000")),
    }
}

/// A Slint string literal.
fn quote(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Parses JSON, refusing an object that repeats a key: serde_json would keep
/// the last one silently, and the phone's renderer might keep another.
fn parse(json: &str) -> Result<Value, String> {
    use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};

    struct Strict(Value);

    impl<'de> Deserialize<'de> for Strict {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            deserializer.deserialize_any(StrictVisitor).map(Strict)
        }
    }

    struct StrictVisitor;

    impl<'de> Visitor<'de> for StrictVisitor {
        type Value = Value;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("JSON")
        }
        fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
            Ok(Value::Bool(v))
        }
        fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
            Ok(Value::from(v))
        }
        fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
            Ok(Value::from(v))
        }
        fn visit_f64<E>(self, v: f64) -> Result<Value, E> {
            Ok(Value::from(v))
        }
        fn visit_str<E>(self, v: &str) -> Result<Value, E> {
            Ok(Value::from(v))
        }
        fn visit_string<E>(self, v: String) -> Result<Value, E> {
            Ok(Value::String(v))
        }
        fn visit_unit<E>(self) -> Result<Value, E> {
            Ok(Value::Null)
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
            let mut items = Vec::new();
            while let Some(Strict(item)) = seq.next_element()? {
                items.push(item);
            }
            Ok(Value::Array(items))
        }
        fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Value, A::Error> {
            let mut map = Map::new();
            while let Some(key) = access.next_key::<String>()? {
                if map.contains_key(&key) {
                    return Err(serde::de::Error::custom(format!("duplicate key {key:?}")));
                }
                let Strict(value) = access.next_value()?;
                map.insert(key, value);
            }
            Ok(Value::Object(map))
        }
    }

    let mut deserializer = serde_json::Deserializer::from_str(json);
    let Strict(value) =
        Strict::deserialize(&mut deserializer).map_err(|e| format!("not JSON: {e}"))?;
    deserializer.end().map_err(|e| format!("not JSON: {e}"))?;
    Ok(value)
}
