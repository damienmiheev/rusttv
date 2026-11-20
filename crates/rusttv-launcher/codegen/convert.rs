//! Converts a panel template (JSON, PANELS.md § 4.1) into one Slint component.
//!
//! The schema holds only what both surfaces (Slint on the TV, the phone page)
//! can draw: unknown fields, colours, badges and properties are errors, so a
//! template can't say anything the converter would silently drop.

use serde_json::{Map, Value};
use std::fmt::Write as _;

/// Colour tokens: `Theme.<token>` in Slint, a CSS variable on the phone.
const TOKENS: [&str; 5] = ["bg", "surface", "text", "text-dim", "accent"];

/// Names a generated component's own properties already use (a
/// `Rectangle`'s and every element's), and Slint words (a row's props are
/// function parameters). A page's scroll box is fed through `scroll-*`
/// (Home owns the scrolling).
const RESERVED_PROPS: [&str; 26] = [
    "u",
    "if",
    "for",
    "return",
    "root",
    "self",
    "parent",
    "true",
    "false",
    "init",
    "animate",
    "states",
    "transitions",
    "width",
    "height",
    "x",
    "y",
    "z",
    "background",
    "clip",
    "visible",
    "opacity",
    "col",
    "row",
    "colspan",
    "rowspan",
];

/// Families of element properties (`min-width`, `drop-shadow-blur`, …),
/// refused by prefix so a new one in Slint is covered too.
const RESERVED_PREFIXES: [&str; 13] = [
    "min-",
    "max-",
    "preferred-",
    "horizontal-",
    "vertical-",
    "border-",
    "drop-shadow-",
    "accessible-",
    "rotation-",
    "transform-",
    "cache-",
    "absolute-",
    "scroll-",
];

/// Names a component must not shadow: Slint's own, theme.slint's,
/// badges.slint's and std-widgets' (blocks.slint's are [`BLOCKS_SLINT`]).
/// A clash with a name of home.slint's own fails the Slint build instead.
const RESERVED_COMPONENTS: [&str; 11] = [
    "Theme",
    "Dir",
    "Hints",
    "Badges",
    "Text",
    "Image",
    "Rectangle",
    "Window",
    "VerticalLayout",
    "HorizontalLayout",
    "AboutSlint",
];

/// Everything ui/blocks.slint exports, which a generated page imports.
pub const BLOCKS_SLINT: [&str; 10] = [
    "PageColumn",
    "PageSurface",
    "PageTitle",
    "PageText",
    "PageHint",
    "PageSlintBadge",
    "PageScrollBox",
    "TitleSize",
    "TextSize",
    "Tone",
];

/// A row's function names, which its props (the parameters) can't take.
const ROW_FUNCTIONS: [&str; 3] = ["title", "detail", "attention"];

/// The template format's version: a runtime loader or an editor refuses a
/// file it doesn't know.
const SCHEMA: u64 = 1;

/// Built-in marks a page's `mark` block may show (blocks.slint).
const MARKS: [(&str, &str); 1] = [("slint", "PageSlintBadge")];

/// Long texts Home loads for a page's `scroll-box` (ui.rs).
const SCROLL_SOURCES: [&str; 1] = ["notices"];

/// Converts `json` into Slint source: a tile is a component, a row a global
/// of pure functions (its words, for Settings' list), a page a component
/// of blocks (blocks.slint). The output imports `Theme`, `Dir`, `Badges`
/// and the blocks by name, found through the compiler's include paths, so
/// every file shares the one `Dir.rtl`. `badges` are the names of the
/// pictures in `Badges` (set from Rust); nothing else can be an image.
pub fn convert(json: &str, badges: &[&str]) -> Result<String, String> {
    let doc = parse(json)?;
    let doc = object(&doc, "template")?;
    only(
        doc,
        "template",
        &["schema", "component", "face", "props", "root"],
    )?;
    if field(doc, "template", "schema")?.as_u64() != Some(SCHEMA) {
        return Err(format!("schema: this converter reads version {SCHEMA}"));
    }
    let name = string(field(doc, "template", "component")?, "component")?;
    if !name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        || !name.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return Err(format!(
            "component: {name:?} must be an UpperCamelCase name"
        ));
    }
    if RESERVED_COMPONENTS.contains(&name) || BLOCKS_SLINT.contains(&name) {
        return Err(format!(
            "component: {name:?} is taken (Slint's, the theme's or a block's)"
        ));
    }
    let face = string(field(doc, "template", "face")?, "face")?;
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
        if RESERVED_PROPS.contains(&prop)
            || RESERVED_PREFIXES
                .iter()
                .any(|prefix| prop.starts_with(prefix))
            || (face == "row" && ROW_FUNCTIONS.contains(&prop))
            || props.contains(&prop)
        {
            return Err(format!(
                "props[{i}]: {prop:?} is taken (a Slint word, an element property, `u`, `scroll-*`, a row's function, or a duplicate)"
            ));
        }
        props.push(prop);
    }

    let (kind, body) = node(field(doc, "template", "root")?, "root")?;
    let root = match face {
        "tile" => "column",
        "row" => "list-row",
        "page" => "page",
        other => return Err(format!("face: {other:?} is not tile, row or page")),
    };
    if kind != root {
        return Err(format!("root: a {face}'s root must be a {root}"));
    }
    let at = format!("root.{root}");
    let mut out = String::from("// Generated from a panel template by build.rs. Do not edit.\n");
    // A row's words are functions of its properties; elsewhere they are
    // the component's own.
    let scope = if face == "row" { "" } else { "root." };
    let cx = Cx {
        props: &props,
        badges,
        scope,
    };
    match face {
        "tile" => cx.tile(name, body, &at, &mut out)?,
        "row" => cx.row(name, body, &at, &mut out)?,
        _ => cx.page(name, body, &at, &mut out)?,
    }
    Ok(out)
}

struct Cx<'a> {
    props: &'a [&'a str],
    badges: &'a [&'a str],
    /// What a property is read through: `root.` in a component, nothing in
    /// a function (its parameters).
    scope: &'a str,
}

impl Cx<'_> {
    /// A tile: one sized, rounded column of text and pictures.
    fn tile(
        &self,
        name: &str,
        body: &Map<String, Value>,
        at: &str,
        out: &mut String,
    ) -> Result<(), String> {
        out.push_str("import { Theme, Dir } from \"theme.slint\";\n");
        out.push_str("import { Badges } from \"badges.slint\";\n\n");
        let _ = writeln!(out, "export component {name} inherits Rectangle {{");
        self.declare(out, false);
        let _ = writeln!(
            out,
            "    width: {} * u;",
            number(field(body, at, "width")?, &format!("{at}.width"))?
        );
        let _ = writeln!(
            out,
            "    height: {} * u;",
            number(field(body, at, "height")?, &format!("{at}.height"))?
        );
        if let Some(radius) = body.get("radius") {
            let _ = writeln!(
                out,
                "    border-radius: {} * u;",
                number(radius, &format!("{at}.radius"))?
            );
        }
        if let Some(background) = body.get("background") {
            let _ = writeln!(
                out,
                "    background: {};",
                self.color(background, &format!("{at}.background"))?
            );
        }
        out.push_str("    clip: true;\n");
        // Drawn once into a cached layer (where the renderer supports it), so a
        // swaying tile moves a picture instead of laying out text every frame.
        out.push_str("    cache-rendering-hint: true;\n\n");
        self.column(body, at, 1, out, true)?;
        out.push_str("}\n");
        Ok(())
    }

    /// A row: its title, dim detail and attention (Settings' accent dot),
    /// as functions of the properties, since Settings draws every row of
    /// its list with one component (quick.slint `ControlRow`).
    fn row(
        &self,
        name: &str,
        body: &Map<String, Value>,
        at: &str,
        out: &mut String,
    ) -> Result<(), String> {
        only(body, at, &["title", "detail", "attention"])?;
        let params = self
            .props
            .iter()
            .map(|prop| format!("{prop}: string"))
            .collect::<Vec<_>>()
            .join(", ");
        let title = self.words(field(body, at, "title")?, &format!("{at}.title"))?;
        let detail = match body.get("detail") {
            Some(detail) => self.words(detail, &format!("{at}.detail"))?,
            None => "\"\"".into(),
        };
        let attention = match body.get("attention") {
            Some(attention) => self.flag(attention, &format!("{at}.attention"))?,
            None => "false".into(),
        };
        let _ = writeln!(out, "\nexport global {name} {{");
        for (function, kind, value) in [
            ("title", "string", title),
            ("detail", "string", detail),
            ("attention", "bool", attention),
        ] {
            let _ = writeln!(
                out,
                "    public pure function {function}({params}) -> {kind} {{\n        return {value};\n    }}"
            );
        }
        out.push_str("}\n");
        Ok(())
    }

    /// A page: blocks in one column, or in a narrow column and the surface
    /// panel beside it (`start` and `end`, mirrored in right-to-left).
    fn page(
        &self,
        name: &str,
        body: &Map<String, Value>,
        at: &str,
        out: &mut String,
    ) -> Result<(), String> {
        only(body, at, &["layout", "start", "end"])?;
        let layout = string(field(body, at, "layout")?, &format!("{at}.layout"))?;
        let two = match layout {
            "one-column" => false,
            "two-columns" => true,
            other => {
                return Err(format!(
                    "{at}.layout: {other:?} is not one-column or two-columns"
                ));
            }
        };
        if !two && body.contains_key("end") {
            return Err(format!("{at}.end: a one-column page has only a start"));
        }
        let mut blocks = String::new();
        let mut scroll = false;
        let _ = writeln!(
            blocks,
            "    PageColumn {{\n        u: root.u;\n        wide: {};",
            !two
        );
        self.blocks(body, at, "start", false, &mut scroll, &mut blocks)?;
        blocks.push_str("    }\n");
        if two {
            blocks.push_str("    PageSurface {\n        u: root.u;\n");
            self.blocks(body, at, "end", true, &mut scroll, &mut blocks)?;
            let _ = writeln!(blocks, "        fills: {scroll};\n    }}");
        }

        out.push_str("import { Theme } from \"theme.slint\";\n");
        let _ = writeln!(
            out,
            "import {{ {} }} from \"blocks.slint\";\n",
            BLOCKS_SLINT.join(", ")
        );
        let _ = writeln!(out, "export component {name} inherits Rectangle {{");
        self.declare(out, scroll);
        out.push_str("    background: Theme.bg;\n\n");
        out.push_str(&blocks);
        out.push_str("}\n");
        Ok(())
    }

    /// `in` properties: `u`, the template's own, and the scroll box's.
    fn declare(&self, out: &mut String, scroll: bool) {
        out.push_str("    in property <length> u;\n");
        for prop in self.props {
            let _ = writeln!(out, "    in property <string> {prop};");
        }
        if scroll {
            out.push_str("    in property <string> scroll-text;\n");
            out.push_str("    in property <float> scroll-position;\n");
            out.push_str("    in property <float> scroll-fraction: 1;\n");
        }
    }

    /// The blocks of a page's `side` (`start` or `end`). A scroll box fills
    /// the surface panel's height, so it lives there, once per page: Up and
    /// Down scroll it (Home's keys, PANELS.md § 4).
    fn blocks(
        &self,
        body: &Map<String, Value>,
        page: &str,
        side: &str,
        surface: bool,
        scroll: &mut bool,
        out: &mut String,
    ) -> Result<(), String> {
        let at = format!("{page}.{side}");
        for (i, block) in array(field(body, page, side)?, &at)?.iter().enumerate() {
            let at = format!("{at}[{i}]");
            let (kind, block) = node(block, &at)?;
            let at = format!("{at}.{kind}");
            let allowed: &[&str] = match kind {
                "title" => &["value", "size", "show"],
                "paragraph" => &["value", "size", "tone", "show"],
                "hint" => &["value", "show"],
                // Slint's attribution is never conditional.
                "mark" => &["name"],
                "scroll-box" => &["source"],
                other => {
                    return Err(format!(
                        "{at}: unknown block {other:?} (title, paragraph, hint, mark, scroll-box)"
                    ));
                }
            };
            only(block, &at, allowed)?;
            let show = match block.get("show") {
                Some(show) => format!("if {}: ", self.flag(show, &format!("{at}.show"))?),
                None => String::new(),
            };
            let mut fields = Vec::new();
            let component = match kind {
                "title" => {
                    let size = choice(block, &at, "size", &["large", "medium", "small"], "medium")?;
                    fields.push(format!("size: TitleSize.{size}"));
                    "PageTitle"
                }
                "paragraph" => {
                    let size = choice(block, &at, "size", &["normal", "small"], "normal")?;
                    fields.push(format!("size: TextSize.{size}"));
                    if let Some(tone) = block.get("tone") {
                        fields.push(format!("tone: {}", self.tone(tone, &format!("{at}.tone"))?));
                    }
                    "PageText"
                }
                "hint" => "PageHint",
                "mark" => {
                    let mark = string(field(block, &at, "name")?, &format!("{at}.name"))?;
                    MARKS
                        .iter()
                        .find(|(name, _)| *name == mark)
                        .map(|(_, component)| *component)
                        .ok_or_else(|| {
                            format!(
                                "{at}.name: {mark:?} is not a built-in mark ({})",
                                MARKS.map(|(name, _)| name).join(", ")
                            )
                        })?
                }
                _ => {
                    let source = string(field(block, &at, "source")?, &format!("{at}.source"))?;
                    if !SCROLL_SOURCES.contains(&source) {
                        return Err(format!(
                            "{at}.source: {source:?} is not a text Home loads ({})",
                            SCROLL_SOURCES.join(", ")
                        ));
                    }
                    if !surface {
                        return Err(format!(
                            "{at}: a scroll-box goes in a two-column page's end (the surface panel)"
                        ));
                    }
                    if std::mem::replace(scroll, true) {
                        return Err(format!("{at}: a page has at most one scroll-box"));
                    }
                    fields.push("text: root.scroll-text".into());
                    fields.push("position: root.scroll-position".into());
                    fields.push("fraction: root.scroll-fraction".into());
                    "PageScrollBox"
                }
            };
            if matches!(kind, "title" | "paragraph" | "hint") {
                let value = self.words(field(block, &at, "value")?, &format!("{at}.value"))?;
                let text = if kind == "hint" { "value" } else { "text" };
                fields.push(format!("{text}: {value}"));
            }
            let _ = writeln!(out, "        {show}{component} {{\n            u: root.u;");
            for field in fields {
                let _ = writeln!(out, "            {field};");
            }
            out.push_str("        }\n");
        }
        Ok(())
    }

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
            Some(show) => format!("if {}: ", self.flag(show, &format!("{at}.show"))?),
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
            c.is_control()
                || matches!(
                    c,
                    '\u{061C}'
                        | '\u{200E}'
                        | '\u{200F}'
                        | '\u{2028}'
                        | '\u{2029}'
                        | '\u{202A}'..='\u{202E}'
                        | '\u{2066}'..='\u{2069}'
                )
        }) {
            return Err(format!(
                "{at}.tr: control or bidi characters are not allowed"
            ));
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
        let args: String = args
            .iter()
            .map(|p| format!(", {}{p}", self.scope))
            .collect();
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

    /// A text's tone: `normal`, `dim` or `accent`, or a `select` of them
    /// (else `normal`).
    fn tone(&self, value: &Value, at: &str) -> Result<String, String> {
        if let Some(body) = value.as_object() {
            return self.select(body, at, |v, at| self.tone(v, at), "Tone.normal".into());
        }
        let tone = string(value, at)?;
        if ["normal", "dim", "accent"].contains(&tone) {
            Ok(format!("Tone.{tone}"))
        } else {
            Err(format!("{at}: {tone:?} is not normal, dim, accent"))
        }
    }

    /// A condition: a property's name (true when it isn't empty) or a
    /// `select` of `true` and `false` (else `false`).
    fn flag(&self, value: &Value, at: &str) -> Result<String, String> {
        if let Some(body) = value.as_object() {
            return self.select(
                body,
                at,
                |v, at| {
                    v.as_bool()
                        .map(|b| b.to_string())
                        .ok_or_else(|| format!("{at}: expected true or false"))
                },
                "false".into(),
            );
        }
        let prop = self.prop(value, at)?;
        Ok(format!("{}{prop} != \"\"", self.scope))
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
        // serde_json's map (no `preserve_order`) yields cases sorted by name,
        // so the generated file doesn't depend on the JSON's order.
        for (case, value) in cases {
            if !is_name(case) {
                return Err(format!(
                    "{at}.cases: {case:?} must be lowercase words or numbers joined by '-'"
                ));
            }
            let _ = write!(
                out,
                "{}{prop} == {} ? {} : ",
                self.scope,
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

/// The optional `key` of `map`: one of `options`, `default` when absent.
fn choice<'v>(
    map: &'v Map<String, Value>,
    at: &str,
    key: &str,
    options: &[&str],
    default: &'v str,
) -> Result<&'v str, String> {
    let value = match map.get(key) {
        Some(value) => string(value, &format!("{at}.{key}"))?,
        None => default,
    };
    if options.contains(&value) {
        Ok(value)
    } else {
        Err(format!(
            "{at}.{key}: {value:?} is not {}",
            options.join(", ")
        ))
    }
}

/// Lowercase words or numbers joined by single '-': props' and cases' shape.
fn is_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_alphanumeric())
        && name.ends_with(|c: char| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.contains("--")
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

/// A Slint string literal. Texts and case values arrive without control
/// characters (checked above); dropping them here is a second guard.
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
