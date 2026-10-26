//! Result and error printing shared by all commands.
//!
//! With `--json`, everything goes to stdout as one JSON envelope:
//! `{"schema":"rusttv.v10","ok":true,"data":...}` or
//! `{"schema":"rusttv.v10","ok":false,"error":{"code":...,"message":...}}`.
//! Changing a field or an error code requires a new schema version.

use std::error::Error;

use serde::Serialize;

/// Version of the JSON output contract.
pub const SCHEMA: &str = "rusttv.v10";

#[derive(Debug, Serialize)]
struct Success<'a, T> {
    schema: &'static str,
    ok: bool,
    data: &'a T,
}

#[derive(Debug, Serialize)]
struct Failure<'a> {
    schema: &'static str,
    ok: bool,
    error: ErrorBody<'a>,
}

#[derive(Debug, Serialize)]
struct ErrorBody<'a> {
    code: &'a str,
    message: String,
}

/// Prints `data` as a successful JSON envelope.
pub fn print_json<T: Serialize>(data: &T) {
    print_envelope(&Success {
        schema: SCHEMA,
        ok: true,
        data,
    });
}

/// Prints an error: a JSON envelope on stdout, or a message on stderr.
pub fn print_error(json: bool, code: &str, error: &dyn Error) {
    let message = message_with_causes(error);
    if json {
        print_envelope(&Failure {
            schema: SCHEMA,
            ok: false,
            error: ErrorBody { code, message },
        });
    } else {
        print_to_stderr(&format!("error: {message}"));
    }
}

/// Prints a hint for humans on stderr, so stdout stays pipeable.
pub fn print_note(message: &str) {
    print_to_stderr(message);
}

fn print_envelope<T: Serialize>(envelope: &T) {
    // Our envelopes only contain strings, numbers, options and lists, which
    // always serialize; the fallback exists so output is never empty.
    let text = serde_json::to_string(envelope).unwrap_or_else(|_| {
        format!(
            r#"{{"schema":"{SCHEMA}","ok":false,"error":{{"code":"INTERNAL","message":"failed to serialize output"}}}}"#
        )
    });
    println!("{text}");
}

#[expect(
    clippy::print_stderr,
    reason = "human-readable errors and notes belong on stderr; this is the single place that writes them"
)]
fn print_to_stderr(message: &str) {
    eprintln!("{message}");
}

/// Joins an error with its chain of causes: "outer: inner: root".
pub(crate) fn message_with_causes(error: &dyn Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}

/// Formats KiB as GiB with one decimal, e.g. `1.9 GiB`.
pub fn gib(kib: u64) -> String {
    // Display only; f64 is exact for any realistic device size.
    format!("{:.1} GiB", kib as f64 / (1024.0 * 1024.0))
}

/// Formats KiB as MiB with one decimal, e.g. `43.0 MiB`.
pub fn mib(kib: u64) -> String {
    format!("{:.1} MiB", kib as f64 / 1024.0)
}
