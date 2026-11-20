//! The panel template converter (codegen/convert.rs, run by build.rs):
//! anything outside the schema is a clear error, and no text can break out
//! of its Slint string (PANELS.md § 4.1).

use crate::template_convert::convert;

const OK: &str = include_str!("../templates/about_tile.json");
const BADGES: [&str; 1] = ["rusttv-logo"];

fn run(json: &str) -> Result<String, String> {
    convert(json, &BADGES)
}

fn err(json: &str) -> String {
    run(json).expect_err("should fail")
}

#[test]
fn the_about_template_converts() {
    let out = run(OK).expect("converts");
    assert!(out.contains("@tr(\"Update available\")"));
    assert!(out.contains("source: Badges.rusttv-logo;"));
    assert!(
        !out.contains("left") && !out.contains("right"),
        "no fixed sides"
    );
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
        (
            OK.replace("[\"version\", \"status\"]", "[\"version\", \"width\"]"),
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
    ];
    for (json, expected) in cases {
        assert_ne!(json, OK, "the case must change the template ({expected})");
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
}
