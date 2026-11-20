//! The panel template converter (codegen/convert.rs, run by build.rs):
//! anything outside the schema is a clear error, and no text can break out
//! of its Slint string (PANELS.md § 4.1).

use crate::template_convert::{BLOCKS_SLINT, convert};

const OK: &str = include_str!("../templates/about_tile.json");
const ROW: &str = include_str!("../templates/about_row.json");
const PAGE: &str = include_str!("../templates/about_page.json");
const BADGES: [&str; 1] = ["rusttv-logo"];

/// A one-column page with every block a column may hold, each optional
/// field set.
const ONE_COLUMN: &str = r#"{
  "schema": 1, "component": "TestPage", "face": "page", "props": ["name", "state"],
  "root": { "page": { "layout": "one-column", "start": [
    { "title": { "value": { "tr": "Large" }, "size": "large" } },
    { "title": { "value": { "tr": "Medium" }, "show": "name" } },
    { "title": { "value": { "tr": "Small" }, "size": "small" } },
    { "paragraph": { "value": { "tr": "Hello {}", "args": ["name"] }, "size": "small",
                "tone": "dim", "show": { "select": "state", "cases": { "on": true } } } },
    { "paragraph": { "value": { "tr": "State" } } },
    { "mark": { "name": "slint" } },
    { "hint": { "value": { "tr": "Back returns." }, "show": "state" } }
  ] } }
}"#;

fn run(json: &str) -> Result<String, String> {
    convert(json, &BADGES)
}

fn err(json: &str) -> String {
    run(json).expect_err("should fail")
}

/// No fixed sides: everything goes through `Dir` (blocks.slint, theme.slint).
fn assert_mirrored(out: &str) {
    assert!(
        !out.contains("left") && !out.contains("right"),
        "no fixed sides: {out}"
    );
}

#[test]
fn the_about_templates_convert() {
    let tile = run(OK).expect("tile converts");
    assert!(tile.contains("@tr(\"Update available\")"));
    assert!(tile.contains("source: Badges.rusttv-logo;"));
    assert_mirrored(&tile);

    let row = run(ROW).expect("row converts");
    assert!(row.contains("export global AboutListRow {"));
    assert!(row.contains(
        "public pure function title(version: string) -> string {\n        return @tr(\"RustTV {}\", version);"
    ));
    assert!(row.contains("-> bool {\n        return false;"), "{row}");

    let page = run(PAGE).expect("page converts");
    assert!(page.contains("export component AboutPage inherits Rectangle {"));
    assert!(page.contains("PageColumn {\n        u: root.u;\n        wide: false;"));
    assert!(page.contains("PageSurface {"));
    assert!(page.contains("fills: true;"));
    assert!(page.contains("PageSlintBadge {"));
    assert!(page.contains("in property <float> scroll-fraction: 1;"));
    assert!(page.contains("text: root.scroll-text;"));
    assert!(page.contains("value: @tr(\"Up and Down scroll the notices. Back returns.\");"));
    assert!(page.contains(
        "tone: (root.update == \"available\" ? Tone.accent : root.update == \"needs-permission\" ? Tone.accent : Tone.normal);"
    ), "{page}");
    assert!(page.contains(", root.update-detail)"));
    assert_mirrored(&page);
}

#[test]
fn a_one_column_page_converts_every_block() {
    let page = run(ONE_COLUMN).expect("converts");
    assert!(page.contains("wide: true;"));
    assert!(!page.contains("PageSurface {"));
    assert!(
        !page.contains("scroll-text"),
        "no scroll box, no scroll properties"
    );
    // A title's size is medium unless it says otherwise.
    for size in ["large", "medium", "small"] {
        assert!(page.contains(&format!("size: TitleSize.{size};")), "{size}");
    }
    assert!(page.contains("if root.name != \"\": PageTitle {"));
    assert!(page.contains("size: TextSize.small;\n            tone: Tone.dim;"));
    assert!(page.contains("if (root.state == \"on\" ? true : false): PageText {"));
    // A text's tone is the component's default (normal) unless it says one.
    assert!(page.contains("size: TextSize.normal;\n            text: @tr(\"State\");"));
    assert!(page.contains("if root.state != \"\": PageHint {"));
    assert_mirrored(&page);
}

#[test]
fn rejects_what_the_schema_does_not_hold() {
    let cases = [
        (
            OK.replace("\"gap\": 4", "\"gap\": 4, \"left\": 3"),
            "unknown field \"left\"",
        ),
        (
            OK.replace("\"color\": \"text\"", "\"color\": \"#ff0000\""),
            "is not a colour token",
        ),
        (
            OK.replace("rusttv-logo", "https://evil.example/x.png"),
            "is not a built-in badge",
        ),
        (
            OK.replace("\"args\": [\"version\"]", "\"args\": [\"secret\"]"),
            "\"secret\" is not in props",
        ),
        (
            OK.replace("\"args\": [\"version\"]", "\"args\": []"),
            "needs one arg per {}",
        ),
        (
            OK.replace("\"text\": { \"show\"", "\"html\": { \"show\""),
            "unknown node \"html\"",
        ),
        (
            OK.replace("\"weight\": 500", "\"weight\": 450"),
            "is not 100, 200",
        ),
        (
            OK.replace("\"size\": 23", "\"size\": -1"),
            "expected a number",
        ),
        (
            OK.replace("\"AboutTile\"", "\"About Tile; } import\""),
            "UpperCamelCase",
        ),
        (OK.replace("\"AboutTile\"", "\"Theme\""), "UpperCamelCase"),
        (OK.replace("\"AboutTile\"", "\"PageText\""), "UpperCamelCase"),
        (
            OK.replace("[\"version\", \"status\"]", "[\"version\", \"width\"]"),
            "is taken",
        ),
        (
            OK.replace("[\"version\", \"status\"]", "[\"version\", \"scroll-text\"]"),
            "is taken",
        ),
        (
            OK.replace("[\"version\", \"status\"]", "[\"version\", \"root\"]"),
            "is taken",
        ),
        (
            ROW.replace("[\"version\"]", "[\"title\"]"),
            "is taken",
        ),
        (
            OK.replace("[\"version\", \"status\"]", "[\"version\", \"min-width\"]"),
            "is taken",
        ),
        (
            OK.replace("[\"version\", \"status\"]", "[\"version\", \"drop-shadow-blur\"]"),
            "is taken",
        ),
        (
            OK.replace("[\"version\", \"status\"]", "[\"version\", \"version\"]"),
            "is taken",
        ),
        (
            OK.replace("[\"version\", \"status\"]", "[\"version\", \"-x\"]"),
            "lowercase words",
        ),
        (
            OK.replace("\"gap\": 4", "\"gap\": 4, \"gap\": 9"),
            "duplicate key \"gap\"",
        ),
        (
            PAGE.replace("\"checking\":", "\"Checking\":"),
            "must be lowercase words or numbers",
        ),
        (
            OK.replace("\"text\": { \"show\"", "\"paragraph\": { \"show\""),
            "unknown node \"paragraph\"",
        ),
        (
            PAGE.replace("{ \"mark\":", "{ \"text\":"),
            "unknown block \"text\"",
        ),
        // Faces and their roots.
        (OK.replace("\"face\": \"tile\",", ""), "missing \"face\""),
        (OK.replace("\"schema\": 1,", ""), "missing \"schema\""),
        (
            OK.replace("\"schema\": 1", "\"schema\": 2"),
            "reads version 1",
        ),
        (
            OK.replace("\"face\": \"tile\"", "\"face\": \"banner\""),
            "is not tile, row or page",
        ),
        (
            OK.replace("\"face\": \"tile\"", "\"face\": \"page\""),
            "a page's root must be a page",
        ),
        (
            ROW.replace("\"face\": \"row\"", "\"face\": \"tile\""),
            "a tile's root must be a column",
        ),
        // A row.
        (
            ROW.replace("\"title\"", "\"heading\""),
            "unknown field \"heading\"",
        ),
        (
            ROW.replace("\"detail\"", "\"attention\": \"secret\", \"detail\""),
            "\"secret\" is not in props",
        ),
        (
            ROW.replace(
                "\"detail\"",
                "\"attention\": { \"select\": \"version\", \"cases\": { \"1\": \"yes\" } }, \"detail\"",
            ),
            "expected true or false",
        ),
        (
            ROW.replace("\"title\": { \"tr\": \"RustTV {}\", \"args\": [\"version\"] },", ""),
            "missing \"title\"",
        ),
        // A page.
        (
            PAGE.replace("\"two-columns\"", "\"three-columns\""),
            "is not one-column or two-columns",
        ),
        (
            PAGE.replace("\"two-columns\"", "\"one-column\""),
            "a one-column page has only a start",
        ),
        (
            PAGE.replace("\"layout\"", "\"width\": 9, \"layout\""),
            "unknown field \"width\"",
        ),
        (
            PAGE.replace("{ \"hint\":", "{ \"video\":"),
            "unknown block \"video\"",
        ),
        (
            PAGE.replace("\"size\": \"large\"", "\"size\": 76"),
            "expected a string",
        ),
        (
            PAGE.replace("\"size\": \"large\"", "\"size\": \"huge\""),
            "is not large, medium, small",
        ),
        (
            PAGE.replace("\"size\": \"large\"", "\"size\": \"large\", \"tone\": \"dim\""),
            "unknown field \"tone\"",
        ),
        (
            PAGE.replace("\"size\": \"small\", \"tone\": \"dim\"", "\"size\": \"tiny\""),
            "is not normal, small",
        ),
        (
            PAGE.replace("\"tone\": \"dim\" } },", "\"tone\": \"red\" } },"),
            "is not normal, dim",
        ),
        (
            PAGE.replace("\"tone\": { \"select\"", "\"color\": \"accent\", \"tone\": { \"select\""),
            "unknown field \"color\"",
        ),
        (
            PAGE.replace("\"needs-permission\": \"accent\"", "\"needs-permission\": \"red\""),
            "is not normal, dim, accent",
        ),
        (
            PAGE.replace("\"name\": \"slint\"", "\"name\": \"rusttv-logo\""),
            "is not a built-in mark",
        ),
        (
            // Slint's attribution is never conditional.
            PAGE.replace("\"name\": \"slint\"", "\"name\": \"slint\", \"show\": \"version\""),
            "unknown field \"show\"",
        ),
        (
            PAGE.replace("\"source\": \"notices\"", "\"source\": \"notices\", \"show\": \"version\""),
            "unknown field \"show\"",
        ),
        (
            PAGE.replace("\"source\": \"notices\"", "\"source\": \"/etc/passwd\""),
            "is not a text Home loads",
        ),
        (
            PAGE.replace(
                "{ \"mark\": { \"name\": \"slint\" } },",
                "{ \"scroll-box\": { \"source\": \"notices\" } },",
            ),
            "goes in a two-column page's end",
        ),
        (
            PAGE.replace(
                "{ \"scroll-box\": { \"source\": \"notices\" } }",
                "{ \"scroll-box\": { \"source\": \"notices\" } }, { \"scroll-box\": { \"source\": \"notices\" } }",
            ),
            "at most one scroll-box",
        ),
        (
            PAGE.replace("\"source\": \"notices\"", "\"source\": \"notices\", \"height\": 9"),
            "unknown field \"height\"",
        ),
        (
            PAGE.replace("{ \"title\": { \"value\": { \"tr\": \"Third-party notices\" }, \"size\": \"small\" } },", "{ \"title\": { \"size\": \"small\" } },"),
            "missing \"value\"",
        ),
        (
            PAGE.replace("\"end\": [", "\"end\": [ { \"paragraph\": {}, \"hint\": {} },"),
            "a node has exactly one key",
        ),
    ];
    for (json, expected) in cases {
        let changed = json != OK && json != ROW && json != PAGE;
        assert!(changed, "the case must change the template ({expected})");
        let got = err(&json);
        assert!(got.contains(expected), "expected {expected:?}, got {got:?}");
    }
}

#[test]
fn texts_cannot_break_out_of_their_string() {
    // A quote and a backslash (Slint's `\{…}` interpolation) stay text.
    let out = run(&OK.replace("Update available", r#"x\"); \\(evil)"#)).expect("converts");
    assert!(out.contains(r#"@tr("x\"); \\(evil)")"#), "{out}");
    // Braces other than `{}` are refused (they would be `@tr` placeholders).
    assert!(err(&OK.replace("Update available", "a {0} b")).contains("braces only as {}"));
    // Control and bidi characters are refused (JSON escapes \n and \u202E).
    assert!(err(&OK.replace("Update available", r"a\nb")).contains("control or bidi"));
    assert!(err(&OK.replace("Update available", r"a\u202Eb")).contains("control or bidi"));
    // The same in a row's function and a page's block.
    let out = run(&ROW.replace("RustTV {}", r#"x\"); {}"#)).expect("converts");
    assert!(out.contains(r#"@tr("x\"); {}", version)"#), "{out}");
    let out = run(&PAGE.replace("Third-party notices", r#"x\"; \\(e)"#)).expect("converts");
    assert!(out.contains(r#"text: @tr("x\"; \\(e)");"#), "{out}");
    assert!(err(&PAGE.replace("Third-party notices", r"a\u2066b")).contains("control or bidi"));
}

/// A generated page imports exactly what ui/blocks.slint exports, and no
/// template may name a component after one of them.
#[test]
fn pages_import_every_block_component() {
    let blocks = include_str!("../ui/blocks.slint");
    let mut exported: Vec<&str> = blocks
        .lines()
        .filter_map(|line| {
            let rest = line
                .strip_prefix("export component ")
                .or_else(|| line.strip_prefix("export enum "))?;
            rest.split_whitespace().next()
        })
        .collect();
    exported.sort_unstable();
    let mut listed = BLOCKS_SLINT.to_vec();
    listed.sort_unstable();
    assert_eq!(exported, listed);
    for name in BLOCKS_SLINT {
        let json = PAGE.replace("\"AboutPage\"", &format!("\"{name}\""));
        assert!(err(&json).contains("UpperCamelCase"), "{name}");
    }
}
