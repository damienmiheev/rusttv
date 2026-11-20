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
/// function parameters). Home feeds a page's scroll box through `scroll-*`
/// (it owns the scrolling), its rows through `rows-*` (it owns the focus)
/// and its header's status line through `header-status`.
const RESERVED_PROPS: [&str; 46] = [
    "u",
    "if",
    "else",
    "for",
    "return",
    "in",
    "out",
    "in-out",
    "private",
    "public",
    "pure",
    "property",
    "callback",
    "function",
    "changed",
    "import",
    "export",
    "from",
    "struct",
    "enum",
    "global",
    "component",
    "inherits",
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
    // Rectangle's old name for `background`.
    "color",
    "clip",
    "visible",
    "opacity",
    "col",
    "row",
    "colspan",
    "rowspan",
];

/// Families of element properties (`min-width`, `drop-shadow-blur`, …),
/// refused by prefix so a new one in Slint is covered too, and the page
/// properties Home feeds (`scroll-*`, `rows-*`, `header-*`), refused on
/// every face so a name means the same everywhere.
const RESERVED_PREFIXES: [&str; 15] = [
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
    "rows-",
    "header-",
];

/// Names a component must not shadow: Slint's own, theme.slint's,
/// badges.slint's, pictures.slint's and std-widgets' (blocks.slint's are
/// [`BLOCKS_SLINT`]). A clash with a name of home.slint's own fails the
/// Slint build instead.
const RESERVED_COMPONENTS: [&str; 12] = [
    "Theme",
    "Dir",
    "Hints",
    "Badges",
    "Pictures",
    "Text",
    "Image",
    "Rectangle",
    "Window",
    "VerticalLayout",
    "HorizontalLayout",
    "AboutSlint",
];

/// Everything ui/blocks.slint exports, which a generated page imports.
pub const BLOCKS_SLINT: [&str; 15] = [
    "PageColumn",
    "PageSurface",
    "PageHeader",
    "PageTitle",
    "PageText",
    "PageHint",
    "PageSlintBadge",
    "PagePicture",
    "PageScrollBox",
    "PageRows",
    "MenuRow",
    "SettingsRow",
    "TitleSize",
    "TextSize",
    "Tone",
];

/// A row's function names, which its props (the parameters) can't take.
const ROW_FUNCTIONS: [&str; 3] = ["title", "detail", "attention"];

/// The template format's version: a runtime loader or an editor refuses a
/// file it doesn't know.
const SCHEMA: u64 = 1;

/// Pictures a tile's `image` may show (`badge`): build.rs decodes
/// `assets/badges/<name>.png` and generates the `Badges` global and its
/// Rust setter from this list.
pub const BADGES: [&str; 3] = ["rusttv-logo", "rusttv-remote", "rusttv-history"];

/// A page's blocks and the fields each takes (PANELS.md § 4.1). Slint's
/// attribution (`mark`) is never conditional, and Home's focus relies on
/// its focus blocks (`scroll-box`, `rows`) being there, so they take no
/// `show`.
const PAGE_BLOCKS: [(&str, &[&str]); 7] = [
    ("title", &["value", "size", "show"]),
    ("paragraph", &["value", "size", "tone", "show"]),
    ("hint", &["value", "show"]),
    ("picture", &["name", "show"]),
    ("mark", &["name"]),
    ("scroll-box", &["source"]),
    ("rows", &["source"]),
];

/// Built-in marks a page's `mark` block may show (blocks.slint).
const MARKS: [(&str, &str); 1] = [("slint", "PageSlintBadge")];

/// Long texts Home loads for a page's `scroll-box` (ui.rs).
const SCROLL_SOURCES: [&str; 1] = ["notices"];

/// Lists Home fills for a page's `rows` (ui.rs): the RustTV Remote page's.
const ROWS_SOURCES: [&str; 1] = ["remote"];

/// Pictures Home provides for a page's `picture` block: build.rs makes them
/// the `Pictures` global (`pictures.slint`), which Rust sets.
pub const PICTURES: [&str; 1] = ["remote-banner"];

/// Converts `json` into Slint source: a tile is a component, a row a global
/// of pure functions (its words, for Settings' list), a page a component
/// of blocks (blocks.slint). The output imports `Theme`, `Dir`, `Badges`,
/// `Pictures` and the blocks by name, found through the compiler's include
/// paths, so every file shares the one `Dir.rtl`. A tile's images are
/// [`BADGES`] (set from Rust); nothing else can be an image.
pub fn convert(json: &str) -> Result<String, String> {
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
        scope,
    };
    match face {
        "tile" => cx.tile(name, body, &at, &mut out)?,
        "row" => cx.row(name, body, &at, &mut out)?,
        _ => cx.page(name, body, &at, &mut out)?,
    }
    Ok(out)
}

/// What a page's blocks hold so far: its focus block and its marks.
#[derive(Default)]
struct Seen {
    focus: Option<&'static str>,
    marks: Vec<String>,
}

struct Cx<'a> {
    props: &'a [&'a str],
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
        self.declare(out, None);
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
    /// panel beside it (`start` and `end`, mirrored in right-to-left), under
    /// an optional header (a list page's title and status line).
    fn page(
        &self,
        name: &str,
        body: &Map<String, Value>,
        at: &str,
        out: &mut String,
    ) -> Result<(), String> {
        only(body, at, &["layout", "header", "start", "end"])?;
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
        let header = body.contains_key("header");
        if let Some(value) = body.get("header") {
            let at = format!("{at}.header");
            let value = object(value, &at)?;
            only(value, &at, &["title", "hint"])?;
            let title = self.words(field(value, &at, "title")?, &format!("{at}.title"))?;
            let hint = match value.get("hint") {
                Some(hint) => self.words(hint, &format!("{at}.hint"))?,
                None => "\"\"".into(),
            };
            let _ = writeln!(
                blocks,
                "    PageHeader {{\n        u: root.u;\n        title: {title};\n        hint: {hint};\n        status: root.header-status;\n    }}"
            );
        }
        // The page's one focus block (a scroll box or rows), where Home's
        // Up and Down go, and the marks it shows (each once).
        let mut seen = Seen::default();
        let _ = writeln!(
            blocks,
            "    PageColumn {{\n        u: root.u;\n        wide: {};\n        header: {header};",
            !two
        );
        self.blocks(body, at, "start", false, &mut seen, &mut blocks)?;
        // A focus block (rows) in the column takes its height.
        let _ = writeln!(blocks, "        fills: {};\n    }}", seen.focus.is_some());
        if two {
            let _ = writeln!(
                blocks,
                "    PageSurface {{\n        u: root.u;\n        header: {header};"
            );
            let before = seen.focus;
            self.blocks(body, at, "end", true, &mut seen, &mut blocks)?;
            // A scroll box or rows in the panel take its height.
            let _ = writeln!(blocks, "        fills: {};\n    }}", seen.focus != before);
        }

        out.push_str("import { Theme } from \"theme.slint\";\n");
        out.push_str("import { Pictures } from \"pictures.slint\";\n");
        let _ = writeln!(
            out,
            "import {{ {} }} from \"blocks.slint\";\n",
            BLOCKS_SLINT.join(", ")
        );
        let _ = writeln!(out, "export component {name} inherits Rectangle {{");
        self.declare(out, seen.focus);
        if header {
            out.push_str("    in property <string> header-status;\n");
        }
        out.push_str("    background: Theme.bg;\n\n");
        out.push_str(&blocks);
        out.push_str("}\n");
        Ok(())
    }

    /// `in` properties: `u`, the template's own, and those Home feeds a
    /// page's focus block (`scroll-box` or `rows`) through.
    fn declare(&self, out: &mut String, focus: Option<&str>) {
        out.push_str("    in property <length> u;\n");
        for prop in self.props {
            let _ = writeln!(out, "    in property <string> {prop};");
        }
        match focus {
            Some("scroll-box") => {
                out.push_str("    in property <string> scroll-text;\n");
                out.push_str("    in property <float> scroll-position;\n");
                out.push_str("    in property <float> scroll-fraction: 1;\n");
            }
            Some(_) => {
                out.push_str("    in property <[SettingsRow]> rows-model;\n");
                out.push_str("    in property <int> rows-focus;\n");
                out.push_str("    in property <int> rows-first;\n");
                out.push_str(
                    "    // The pointer on a row: its index; 0 moved over it, 1 clicked.\n",
                );
                out.push_str("    callback rows-pointer(int, int);\n");
            }
            None => {}
        }
    }

    /// The blocks of a page's `side` (`start` or `end`). A page has at most
    /// one focus block, where Home's Up and Down go (PANELS.md § 4): a
    /// scroll box, which fills the surface panel's height and so lives
    /// there, or rows, on either side.
    fn blocks(
        &self,
        body: &Map<String, Value>,
        page: &str,
        side: &str,
        surface: bool,
        seen: &mut Seen,
        out: &mut String,
    ) -> Result<(), String> {
        let at = format!("{page}.{side}");
        for (i, block) in array(field(body, page, side)?, &at)?.iter().enumerate() {
            let at = format!("{at}[{i}]");
            let (kind, block) = node(block, &at)?;
            let at = format!("{at}.{kind}");
            let Some(&(kind, allowed)) = PAGE_BLOCKS.iter().find(|(name, _)| *name == kind) else {
                return Err(format!(
                    "{at}: unknown block {kind:?} ({})",
                    PAGE_BLOCKS.map(|(name, _)| name).join(", ")
                ));
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
                "picture" => {
                    let picture = listed(block, &at, "name", &PICTURES, "a picture Home provides")?;
                    fields.push(format!("source: Pictures.{picture}"));
                    "PagePicture"
                }
                "mark" => {
                    let mark = string(field(block, &at, "name")?, &format!("{at}.name"))?;
                    let &(_, component) =
                        MARKS
                            .iter()
                            .find(|(name, _)| *name == mark)
                            .ok_or_else(|| {
                                format!(
                                    "{at}.name: {mark:?} is not a built-in mark ({})",
                                    MARKS.map(|(name, _)| name).join(", ")
                                )
                            })?;
                    if seen.marks.iter().any(|shown| shown == mark) {
                        return Err(format!("{at}: a page shows each mark once"));
                    }
                    seen.marks.push(mark.to_owned());
                    component
                }
                "scroll-box" => {
                    listed(block, &at, "source", &SCROLL_SOURCES, "a text Home loads")?;
                    if !surface {
                        return Err(format!(
                            "{at}: a scroll-box goes in a two-column page's end (the surface panel)"
                        ));
                    }
                    fields.push("text: root.scroll-text".into());
                    fields.push("position: root.scroll-position".into());
                    fields.push("fraction: root.scroll-fraction".into());
                    "PageScrollBox"
                }
                _ => {
                    listed(block, &at, "source", &ROWS_SOURCES, "a list Home fills")?;
                    fields.push("model: root.rows-model".into());
                    fields.push("focused: root.rows-focus".into());
                    fields.push("first: root.rows-first".into());
                    fields.push(format!("outset: {surface}"));
                    "PageRows"
                }
            };
            if matches!(kind, "scroll-box" | "rows") && seen.focus.replace(kind).is_some() {
                return Err(format!(
                    "{at}: a page has at most one focus block (scroll-box or rows)"
                ));
            }
            if matches!(kind, "title" | "paragraph" | "hint") {
                let value = self.words(field(block, &at, "value")?, &format!("{at}.value"))?;
                let text = if kind == "hint" { "value" } else { "text" };
                fields.push(format!("{text}: {value}"));
            }
            let _ = writeln!(out, "        {show}{component} {{\n            u: root.u;");
            for field in fields {
                let _ = writeln!(out, "            {field};");
            }
            if kind == "rows" {
                out.push_str(
                    "            pointer(index, action) => {\n                root.rows-pointer(index, action);\n            }\n",
                );
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
        if !BADGES.contains(&badge) {
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
    /// An arg `{"ltr": prop}` is a left-to-right run isolated from the text
    /// around it, so a version inside Arabic text keeps its order. A text
    /// with a number in it takes gettext plural forms: `"plural"` is the
    /// English plural and `"count"` the property holding the number (a
    /// whole number as text), which both forms may show as `{n}`; each
    /// language then has its own forms ("1 hour" / "2 hours", Slavic
    /// forms), as Slint's `@tr("…" | "…" % n)`.
    fn words(&self, value: &Value, at: &str) -> Result<String, String> {
        let body = object(value, at)?;
        if body.contains_key("select") {
            return self.select(body, at, |v, at| self.words(v, at), "\"\"".into());
        }
        only(body, at, &["tr", "args", "plural", "count"])?;
        let text = literal(field(body, at, "tr")?, &format!("{at}.tr"))?;
        let plural = match (body.get("plural"), body.get("count")) {
            (Some(plural), Some(count)) => Some((
                literal(plural, &format!("{at}.plural"))?,
                self.prop(count, &format!("{at}.count"))?,
            )),
            (None, None) => None,
            _ => return Err(format!("{at}: \"plural\" and \"count\" go together")),
        };
        let args = match body.get("args") {
            Some(args) => array(args, &format!("{at}.args"))?
                .iter()
                .enumerate()
                .map(|(i, arg)| self.arg(arg, &format!("{at}.args[{i}]")))
                .collect::<Result<Vec<_>, String>>()?,
            None => Vec::new(),
        };
        // `{}` is a placeholder, and `{n}` the count in a plural text; any
        // other brace would be one too in `@tr`.
        let allowed = if plural.is_some() { "{} and {n}" } else { "{}" };
        let forms = std::iter::once(("tr", text)).chain(plural.map(|(form, _)| ("plural", form)));
        for (key, form) in forms {
            let Some(holes) = placeholders(form, plural.is_some()) else {
                return Err(format!(
                    "{at}.{key}: {form:?} may hold braces only as {allowed}"
                ));
            };
            if holes != args.len() {
                return Err(format!("{at}.{key}: {form:?} needs one arg per {{}}"));
            }
        }
        let args: String = args
            .iter()
            .map(|&(prop, ltr)| {
                let prop = format!("{}{prop}", self.scope);
                if ltr {
                    // U+2066 LEFT-TO-RIGHT ISOLATE … U+2069 POP DIRECTIONAL
                    // ISOLATE, as Slint escapes.
                    format!(", \"\\u{{2066}}\" + {prop} + \"\\u{{2069}}\"")
                } else {
                    format!(", {prop}")
                }
            })
            .collect();
        let plural = match plural {
            // Slint converts the count to a whole number (`int`).
            Some((form, count)) => format!(" | {} % {}{count}.to-float()", quote(form), self.scope),
            None => String::new(),
        };
        Ok(format!("@tr({}{plural}{args})", quote(text)))
    }

    /// A text's argument: a property's name, or `{"ltr": name}` (that
    /// property as a left-to-right isolate). Returns the name and whether it
    /// is isolated.
    fn arg<'v>(&self, value: &'v Value, at: &str) -> Result<(&'v str, bool), String> {
        match value.as_object() {
            Some(body) => {
                only(body, at, &["ltr"])?;
                let prop = self.prop(field(body, at, "ltr")?, &format!("{at}.ltr"))?;
                Ok((prop, true))
            }
            None => Ok((self.prop(value, at)?, false)),
        }
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

/// How many `{}` a text holds, read left to right: its only braces are
/// `{}` and, in a plural text, `{n}`; `None` for any other brace (`@tr`
/// would read it as another placeholder or an escape).
fn placeholders(text: &str, plural: bool) -> Option<usize> {
    let mut holes = 0;
    let mut rest = text;
    while let Some(at) = rest.find(['{', '}']) {
        rest = &rest[at..];
        if let Some(after) = rest.strip_prefix("{}") {
            holes += 1;
            rest = after;
        } else {
            rest = rest.strip_prefix("{n}").filter(|_| plural)?;
        }
    }
    Some(holes)
}

/// A text's literal: a string without control or bidi characters, so a text
/// can't hide or reorder words (HISTORY.md § 8.5's rule for names on the
/// TV).
fn literal<'v>(value: &'v Value, at: &str) -> Result<&'v str, String> {
    let text = string(value, at)?;
    if text.chars().any(|c| {
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
        return Err(format!("{at}: control or bidi characters are not allowed"));
    }
    Ok(text)
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

/// `key` of `map`: a name from `names`, a fixed list Home knows (`what`
/// says what it holds).
fn listed<'v>(
    map: &'v Map<String, Value>,
    at: &str,
    key: &str,
    names: &[&str],
    what: &str,
) -> Result<&'v str, String> {
    let name = string(field(map, at, key)?, &format!("{at}.{key}"))?;
    if names.contains(&name) {
        Ok(name)
    } else {
        Err(format!(
            "{at}.{key}: {name:?} is not {what} ({})",
            names.join(", ")
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
