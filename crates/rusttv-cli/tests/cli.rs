//! End-to-end tests that run the built `rusttv` binary.
//!
//! Device commands run against `tests/fixtures/fake-adb` (via `RUSTTV_ADB`),
//! so these tests need neither a real `adb` nor a device.

#![expect(
    clippy::unwrap_used,
    reason = "test helpers should panic on unexpected failures; clippy only exempts #[test] fns"
)]

use std::process::{Command, Output};

use serde_json::Value;

fn rusttv() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rusttv"))
}

/// Runs `rusttv <args>` against the fake adb in the given scenario.
fn rusttv_with_fake_adb(scenario: &str, args: &[&str]) -> Output {
    let fake_adb = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fake-adb");
    rusttv()
        .args(args)
        .env("RUSTTV_ADB", fake_adb)
        .env("FAKE_ADB_SCENARIO", scenario)
        .output()
        .unwrap()
}

fn json_stdout(output: &Output) -> Value {
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schema"], "rusttv.v1");
    value
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

#[test]
fn device_list_json_reports_each_device_and_its_state() {
    let output = rusttv_with_fake_adb("two-devices", &["device", "list", "--json"]);

    assert_eq!(output.status.code(), Some(0));
    let json = json_stdout(&output);
    assert_eq!(json["ok"], true);
    let devices = json["data"].as_array().unwrap();
    assert_eq!(devices.len(), 2);
    assert_eq!(devices[0]["serial"], "emulator-5554");
    assert_eq!(devices[0]["state"], "device");
    assert_eq!(devices[0]["model"], "Android_TV_on_x86");
    assert_eq!(devices[1]["serial"], "192.168.1.20:5555");
    assert_eq!(devices[1]["state"], "unauthorized");
    assert_eq!(devices[1]["model"], Value::Null);
}

#[test]
fn device_list_text_shows_a_table() {
    let output = rusttv_with_fake_adb("two-devices", &["device", "list"]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(lines[0].starts_with("SERIAL"), "{stdout}");
    assert!(lines[1].starts_with("emulator-5554") && lines[1].contains("device"));
    assert!(lines[2].starts_with("192.168.1.20:5555") && lines[2].contains("unauthorized"));
}

#[test]
fn device_list_without_devices_explains_how_to_connect() {
    let output = rusttv_with_fake_adb("no-devices", &["device", "list"]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("rusttv device connect"), "{stdout}");
}

#[test]
fn device_connect_adds_the_default_port_and_reports_state() {
    let output = rusttv_with_fake_adb(
        "connect-ok",
        &["device", "connect", "192.168.1.20", "--json"],
    );

    assert_eq!(output.status.code(), Some(0));
    let json = json_stdout(&output);
    assert_eq!(json["data"]["address"], "192.168.1.20:5555");
    assert_eq!(json["data"]["outcome"], "connected");
    assert_eq!(json["data"]["state"], "unauthorized");
}

#[test]
fn device_connect_text_tells_the_user_to_accept_the_prompt() {
    let output = rusttv_with_fake_adb("connect-ok", &["device", "connect", "192.168.1.20"]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("Connected to 192.168.1.20:5555"),
        "{stdout}"
    );
    assert!(stdout.contains("Accept the prompt on the TV"), "{stdout}");
}

#[test]
fn refused_connection_is_an_unreachable_device_even_though_adb_exits_0() {
    let output = rusttv_with_fake_adb(
        "connect-refused",
        &["device", "connect", "192.168.1.20", "--json"],
    );

    assert_eq!(output.status.code(), Some(3));
    let json = json_stdout(&output);
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "CONNECT_FAILED");
    let message = json["error"]["message"].as_str().unwrap();
    assert!(message.contains("Connection refused"), "{message}");
}

#[test]
fn missing_adb_is_reported_with_a_stable_code() {
    let output = rusttv()
        .args(["device", "list", "--json"])
        .env("RUSTTV_ADB", "/nonexistent/rusttv-test/adb")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(json_stdout(&output)["error"]["code"], "ADB_NOT_FOUND");
}

#[test]
fn missing_adb_text_error_goes_to_stderr() {
    let output = rusttv()
        .args(["device", "list"])
        .env("RUSTTV_ADB", "/nonexistent/rusttv-test/adb")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.starts_with("error: `adb` was not found"), "{stderr}");
}

#[test]
fn invalid_address_is_a_usage_error() {
    let output = rusttv_with_fake_adb("connect-ok", &["device", "connect", "tv;reboot"]);

    assert_eq!(output.status.code(), Some(2));
}
