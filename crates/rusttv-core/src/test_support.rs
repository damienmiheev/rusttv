//! Strategies shared by the parser property tests.

use proptest::prelude::*;

/// Pieces of real adb output, so random input reaches the parsers' inner
/// branches instead of failing at the first prefix check.
const FRAGMENTS: &[&str] = &[
    "\n",
    "\r\n",
    "\r",
    " ",
    "\t",
    ":",
    "/",
    "=",
    "[",
    "]",
    "]: [",
    "}",
    "%",
    "-",
    ".",
    "0",
    "7",
    "-1",
    "4294967296",
    "18446744073709551615",
    "18446744073709551616",
    "99999999999999999999999",
    "inf",
    "NaN",
    "1e3",
    "é",
    "kB",
    "package:",
    "priority=",
    "cpu ",
    "cpu  ",
    "MemTotal:",
    "MemAvailable:",
    "TOTAL PSS:",
    "TOTAL RSS:",
    "Status: ok",
    "Status",
    "Error:",
    "LaunchState:",
    "TotalTime:",
    "WaitTime:",
    "topResumedActivity=",
    "ActivityRecord{",
    "List of devices attached",
    "device",
    "no permissions",
    "model:",
    "connected to",
    "already connected to",
    "WARNING: linker: ",
    "com.example/.Main",
];

/// Arbitrary text, biased towards adb-like fragments.
pub(crate) fn adb_output() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        4 => prop::sample::select(FRAGMENTS).prop_map(str::to_owned),
        1 => any::<String>(),
    ];
    prop::collection::vec(piece, 0..40).prop_map(|pieces| pieces.concat())
}

/// A line ending seen from adb on some host: `\r\n` from Windows hosts and
/// old pty-mode `adb shell`, `\r\r\n` when both convert.
pub(crate) fn line_ending() -> impl Strategy<Value = &'static str> {
    prop::sample::select(&["\n", "\r\n", "\r\r\n"][..])
}
