//! Pass bar 4: anything outside the schema is a clear error.
#[path = "../convert.rs"]
#[allow(dead_code)]
mod convert;

const OK: &str = include_str!("../templates/about_tile.json");

fn err(json: &str) -> String {
    convert::convert(json, "theme.slint", &[("rusttv-logo", "logo.png")]).expect_err("should fail")
}

#[test]
fn the_real_template_converts() {
    let out =
        convert::convert(OK, "theme.slint", &[("rusttv-logo", "logo.png")]).expect("converts");
    assert!(out.contains("@tr(\"Update available\")"));
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
    ];
    for (json, expected) in cases {
        let got = err(&json);
        assert!(got.contains(expected), "expected {expected:?}, got {got:?}");
    }
}

#[test]
fn quotes_cannot_break_out_of_a_string() {
    let convert =
        |json: &str| convert::convert(json, "theme.slint", &[("rusttv-logo", "logo.png")]);
    // A quote and a backslash (Slint's `\{…}` interpolation) stay text.
    let out = convert(&OK.replace("Update available", r#"x\"); \\(evil)"#)).expect("converts");
    assert!(out.contains(r#"@tr("x\"); \\(evil)")"#), "{out}");
    // Braces other than `{}` are refused (they would be `@tr` placeholders).
    assert!(err(&OK.replace("Update available", "a {0} b")).contains("braces only as {}"));
    // Control and bidi characters are refused (JSON escapes \n and \u202E).
    assert!(err(&OK.replace("Update available", r"a\nb")).contains("control or bidi"));
    assert!(err(&OK.replace("Update available", r"a\u202Eb")).contains("control or bidi"));
}
