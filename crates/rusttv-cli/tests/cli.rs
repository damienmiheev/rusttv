//! End-to-end tests that run the built `rusttv` binary.

use std::process::Command;

fn rusttv() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rusttv"))
}

#[test]
fn version_flag_prints_name_and_version() {
    let output = rusttv().arg("--version").output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout.trim(),
        format!("rusttv {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn unknown_argument_is_a_usage_error() {
    let output = rusttv().arg("--no-such-flag").output().unwrap();

    // clap exits with 2 on usage errors, matching ARCHITECTURE.md exit codes.
    assert_eq!(output.status.code(), Some(2));
}
