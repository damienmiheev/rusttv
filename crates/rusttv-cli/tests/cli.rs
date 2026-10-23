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

#[test]
fn device_inspect_json_reads_the_only_device() {
    let output = rusttv_with_fake_adb("emulator", &["device", "inspect", "--json"]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = &json_stdout(&output)["data"];
    assert_eq!(data["serial"], "emulator-5554");
    assert_eq!(data["manufacturer"], "Google");
    assert_eq!(data["api_level"], 34);
    assert_eq!(data["abis"], serde_json::json!(["arm64-v8a"]));
    assert_eq!(data["memory"]["total_kib"], 2_026_264);
    assert_eq!(data["data_storage"]["available_kib"], 4_813_952);
}

#[test]
fn device_inspect_text_is_a_readable_summary() {
    let output = rusttv_with_fake_adb("emulator", &["device", "inspect"]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("14 (API 34), security patch 2023-09-05"),
        "{stdout}"
    );
    assert!(stdout.contains("1.9 GiB total"), "{stdout}");
}

#[test]
fn device_inspect_with_several_devices_needs_a_serial() {
    let output = rusttv_with_fake_adb("two-devices", &["device", "inspect", "--json"]);

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(json_stdout(&output)["error"]["code"], "MULTIPLE_DEVICES");
}

#[test]
fn device_inspect_of_an_unauthorized_device_says_so() {
    let output = rusttv_with_fake_adb(
        "two-devices",
        &[
            "device",
            "inspect",
            "--serial",
            "192.168.1.20:5555",
            "--json",
        ],
    );

    assert_eq!(output.status.code(), Some(3));
    let error = &json_stdout(&output)["error"];
    assert_eq!(error["code"], "DEVICE_UNAUTHORIZED");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("accept the debugging prompt")
    );
}

#[test]
fn apps_list_json_marks_kind_and_state() {
    let output = rusttv_with_fake_adb("emulator", &["apps", "list", "--json"]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let apps = json_stdout(&output)["data"].as_array().unwrap().clone();
    assert_eq!(apps.len(), 224);
    let app = |name: &str| {
        apps.iter()
            .find(|app| app["package"] == name)
            .unwrap()
            .clone()
    };
    assert_eq!(app("com.redphx.deviceid")["kind"], "third_party");
    assert_eq!(app("com.android.nfc")["enabled"], false);
    assert_eq!(
        app("android")["apk_path"],
        "/system/framework/framework-res.apk"
    );
}

#[test]
fn apps_list_text_ends_with_a_summary() {
    let output = rusttv_with_fake_adb("emulator", &["apps", "list"]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("PACKAGE"), "{stdout}");
    assert_eq!(
        stdout.lines().last(),
        Some("224 packages: 221 system, 3 third-party; 2 disabled")
    );
}

#[test]
fn apps_list_needs_a_serial_when_several_devices_are_connected() {
    let output = rusttv_with_fake_adb("two-devices", &["apps", "list", "--json"]);

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(json_stdout(&output)["error"]["code"], "MULTIPLE_DEVICES");
}

#[test]
fn performance_snapshot_json_has_launcher_and_top_processes() {
    let output = rusttv_with_fake_adb(
        "emulator",
        &["performance", "snapshot", "--sample-ms", "100", "--json"],
    );

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = &json_stdout(&output)["data"];
    assert_eq!(data["serial"], "emulator-5554");
    // The fake returns the same /proc/stat twice, so no time passes.
    assert_eq!(data["cpu"], Value::Null);
    assert_eq!(
        data["home_app"]["package"],
        "com.google.android.apps.nexuslauncher"
    );
    assert_eq!(data["home_app"]["pss_kib"], 44_081);
    assert_eq!(data["top_processes"].as_array().unwrap().len(), 10);
    assert_eq!(data["top_processes"][0]["name"], "system_server");
    assert_eq!(data["top_processes"][0]["rss_kib"], 243_832);
}

#[test]
fn performance_snapshot_text_names_the_launcher() {
    let output = rusttv_with_fake_adb(
        "emulator",
        &["performance", "snapshot", "--sample-ms", "100"],
    );

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("com.google.android.apps.nexuslauncher (43.0 MiB PSS, 77.0 MiB RSS)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Top processes by resident memory:"),
        "{stdout}"
    );
}

#[test]
fn performance_snapshot_rejects_tiny_sample_intervals() {
    let output = rusttv_with_fake_adb("emulator", &["performance", "snapshot", "--sample-ms", "5"]);

    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn every_error_code_is_documented_in_cli_md() {
    let docs = include_str!("../../../CLI.md");
    let cli_only = ["INCOMPLETE_PROFILE", "INTERNAL"];
    for code in rusttv_core::ERROR_CODES.iter().chain(&cli_only) {
        assert!(
            docs.contains(&format!("| `{code}` |")),
            "{code} is missing from the error code table in CLI.md"
        );
    }
}

#[test]
fn compat_profile_prints_toml_and_suggests_a_path() {
    let output = rusttv_with_fake_adb("emulator", &["compat", "profile"]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let committed = include_str!("../../../compat/devices/google-emu64a.toml");
    // Same profile as the committed one, apart from the hand-written notes.
    let without_notes = |text: &str| -> String {
        text.lines()
            .filter(|line| !line.starts_with("notes ="))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(without_notes(&stdout), without_notes(committed));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("compat/devices/google-emu64a.toml"),
        "{stderr}"
    );
}

#[test]
fn compat_profile_json_includes_the_suggested_path() {
    let output = rusttv_with_fake_adb("emulator", &["compat", "profile", "--json"]);

    assert_eq!(output.status.code(), Some(0));
    let data = &json_stdout(&output)["data"];
    assert_eq!(data["path"], "compat/devices/google-emu64a.toml");
    assert_eq!(data["profile"]["device"]["kind"], "emulator");
    assert_eq!(data["profile"]["firmware"]["api_level"], 34);
}

/// Fast settings for benchmark tests: no waiting.
const QUICK_BENCHMARK: [&str; 10] = [
    "--runs",
    "3",
    "--warmup",
    "1",
    "--pause-ms",
    "0",
    "--settle-ms",
    "0",
    "--cpu-window-ms",
    "0",
];

#[test]
fn performance_benchmark_json_reports_cold_starts_and_raw_runs() {
    let mut args = vec![
        "performance",
        "benchmark",
        "--app",
        "ai.rusttv.spike/.LauncherActivity",
        "--idle-samples",
        "1",
        "--json",
    ];
    args.extend(QUICK_BENCHMARK);
    let output = rusttv_with_fake_adb("emulator", &args);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = &json_stdout(&output)["data"];
    let app = &data["apps"][0];
    assert_eq!(app["component"], "ai.rusttv.spike/.LauncherActivity");
    assert_eq!(app["cold_start_ms"]["n"], 3);
    assert_eq!(app["cold_start_ms"]["median"], 281.0);
    assert_eq!(
        app["resume_ms"],
        Value::Null,
        "one app has no resume measurement"
    );
    assert_eq!(app["idle_pss_kib"]["median"], 44_081.0);
    assert_eq!(
        data["runs"].as_array().unwrap().len(),
        4,
        "warm-up + 3 runs"
    );
    assert_eq!(data["device"]["manufacturer"], "Google");
    let progress = String::from_utf8(output.stderr).unwrap();
    assert!(progress.contains("cold 4/4"), "{progress}");
}

#[test]
fn performance_benchmark_of_a_missing_activity_fails_clearly() {
    let mut args = vec![
        "performance",
        "benchmark",
        "--app",
        "ai.rusttv.spike/.Missing",
        "--json",
    ];
    args.extend(QUICK_BENCHMARK);
    let output = rusttv_with_fake_adb("emulator", &args);

    assert_eq!(output.status.code(), Some(1));
    let error = &json_stdout(&output)["error"];
    assert_eq!(error["code"], "LAUNCH_FAILED");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("does not exist")
    );
}

#[test]
fn performance_benchmark_rejects_unsafe_components() {
    let output = rusttv_with_fake_adb(
        "emulator",
        &[
            "performance",
            "benchmark",
            "--app",
            "ai.rusttv.spike/.Main;reboot",
        ],
    );

    assert_eq!(output.status.code(), Some(2));
}
