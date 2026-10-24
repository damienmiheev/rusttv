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

/// The binary, with a config directory that has no choices file and a state
/// directory without journals, so the tests never read the real
/// `privacy.toml` or journals (tests that need journals set their own).
fn rusttv() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rusttv"));
    command.env(
        "RUSTTV_CONFIG_DIR",
        std::env::temp_dir().join("rusttv-cli-tests-no-config"),
    );
    command.env(
        "RUSTTV_STATE_DIR",
        std::env::temp_dir().join("rusttv-cli-tests-no-state"),
    );
    command
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
    assert_eq!(value["schema"], "rusttv.v7");
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
fn launcher_status_json_shows_home_and_candidates() {
    let output = rusttv_with_fake_adb("tv", &["launcher", "status", "--json"]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = json_stdout(&output)["data"].clone();
    assert_eq!(
        data["home"],
        "com.google.android.apps.tv.launcherx/.home.HomeActivity"
    );
    assert_eq!(data["rusttv_installed"], true);
    assert_eq!(data["rusttv_is_home"], false);
    assert_eq!(data["candidates"][0]["priority"], 2);
    assert_eq!(
        data["candidates"][1]["component"],
        "ai.rusttv.launcher/.LauncherActivity"
    );
}

#[test]
fn launcher_status_text_explains_a_higher_priority_launcher() {
    let output = rusttv_with_fake_adb("tv", &["launcher", "status"]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.starts_with(
            "Home button opens: com.google.android.apps.tv.launcherx/.home.HomeActivity\n\
         RustTV launcher:   installed, not Home\n"
        ),
        "{stdout}"
    );
    assert!(stdout.contains("       2  com.google.android.apps.tv.launcherx/.home.HomeActivity"));
    assert!(
        stdout.ends_with("always wins the Home button on this device.\n"),
        "{stdout}"
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
fn apps_list_fails_when_pm_prints_an_error_instead_of_packages() {
    let output = rusttv_with_fake_adb("pm-fails", &["apps", "list", "--json"]);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let error = json_stdout(&output)["error"].clone();
    assert_eq!(error["code"], "PACKAGE_LIST_FAILED");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("cmd: Can't find service: package"),
        "{error}"
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
    // Same profile as the committed one, apart from the hand-written notes and
    // the RustTV version it was tested with, which stays as recorded.
    let without_notes = |text: &str| -> String {
        text.lines()
            .filter(|line| !line.starts_with("notes =") && !line.starts_with("rusttv_version ="))
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
const QUICK_BENCHMARK: [&str; 12] = [
    "--runs",
    "3",
    "--warmup",
    "1",
    "--pause-ms",
    "0",
    "--settle-ms",
    "0",
    "--fresh-settle-ms",
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
    assert_eq!(app["idle_pss_kib_fresh"]["median"], 44_081.0);
    assert_eq!(app["idle_pss_kib_after_resumes"]["median"], 44_081.0);
    assert_eq!(app["idle_pss_growth_kib"], 0.0);
    assert_eq!(data["options"]["fresh_settle_ms"], 0);
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
fn performance_benchmark_text_shows_fresh_and_after_resumes_memory() {
    let mut args = vec![
        "performance",
        "benchmark",
        "--app",
        "ai.rusttv.spike/.LauncherActivity",
        "--idle-samples",
        "1",
    ];
    args.extend(QUICK_BENCHMARK);
    let output = rusttv_with_fake_adb("emulator", &args);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains("idle PSS fresh         median 43.0 MiB"),
        "{text}"
    );
    assert!(
        text.contains("idle PSS after resumes median 43.0 MiB"),
        "{text}"
    );
    assert!(text.contains("idle PSS growth        +0.0 MiB"), "{text}");
    let progress = String::from_utf8(output.stderr).unwrap();
    assert!(progress.contains("fresh idle 1/1"), "{progress}");
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

/// A fresh, empty state directory for one test (`RUSTTV_STATE_DIR`).
fn state_dir(test: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rusttv-cli-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("journals")).unwrap();
    dir
}

fn rusttv_with_state(scenario: &str, state: &std::path::Path, args: &[&str]) -> Output {
    let fake_adb = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fake-adb");
    rusttv()
        .args(args)
        .env("RUSTTV_ADB", fake_adb)
        .env("FAKE_ADB_SCENARIO", scenario)
        .env("RUSTTV_STATE_DIR", state)
        .output()
        .unwrap()
}

#[test]
fn launcher_takeover_is_refused_on_a_device_without_a_tested_profile() {
    // No bundled profile records a tested takeover, so even a dry run stops
    // here. The plan itself is tested in rusttv-core with a test profile.
    let state = state_dir("takeover-untested");
    let output = rusttv_with_state(
        "tv",
        &state,
        &["launcher", "takeover", "--dry-run", "--json"],
    );

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let json = json_stdout(&output);
    assert_eq!(json["error"]["code"], "TAKEOVER_NOT_TESTED");
    let message = json["error"]["message"].as_str().unwrap();
    assert!(message.contains("sdk_google_atv64_arm64"), "{message}");
    assert_eq!(
        std::fs::read_dir(state.join("journals")).unwrap().count(),
        0
    );
    std::fs::remove_dir_all(state).unwrap();
}

#[test]
fn launcher_takeover_without_a_terminal_is_refused_before_touching_a_device() {
    // Test processes get no terminal on stdin, like an agent or a script.
    // `no-devices` would fail with NO_DEVICE if the device were looked up.
    let output = rusttv_with_fake_adb("no-devices", &["launcher", "takeover", "--json"]);

    assert_eq!(output.status.code(), Some(1));
    let json = json_stdout(&output);
    assert_eq!(json["error"]["code"], "NOT_APPROVED");
    let message = json["error"]["message"].as_str().unwrap();
    assert!(message.contains("interactive terminal"), "{message}");

    let output = rusttv_with_fake_adb("no-devices", &["launcher", "restore", "--json"]);
    assert_eq!(json_stdout(&output)["error"]["code"], "NOT_APPROVED");
}

/// A takeover journal for the synthetic getprop fixture's fingerprint.
const TAKEOVER_JOURNAL: &str = r#"schema_version = 1
plan = "home-takeover"
serial = "emulator-5554"
fingerprint = "google/sdk_google_atv64_arm64/emu64a:14/UTT3.240625.001.K5/12345678:userdebug/dev-keys"
started_at_unix_ms = 1761220000000
status = "applied"

[[steps]]
adb = "adb -s emulator-5554 shell pm disable-user --user 0 com.google.android.apps.tv.launcherx"
command = { kind = "disable_package", arg = "com.google.android.apps.tv.launcherx" }
original_state = { kind = "package_enabled", arg = "com.google.android.apps.tv.launcherx" }
status = "done"
"#;

#[test]
fn launcher_restore_dry_run_finds_the_takeover_journal() {
    let state = state_dir("restore-dry-run");
    std::fs::write(
        state.join("journals/1761220000000-home-takeover.toml"),
        TAKEOVER_JOURNAL,
    )
    .unwrap();

    let output = rusttv_with_state(
        "tv",
        &state,
        &["launcher", "restore", "--dry-run", "--json"],
    );

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let json = json_stdout(&output);
    assert_eq!(json["data"]["plan"], "home-takeover-restore");
    assert_eq!(json["data"]["confirmation"], "restore");
    let steps = json["data"]["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(
        steps[0]["command"],
        "adb -s emulator-5554 shell pm enable --user 0 com.google.android.apps.tv.launcherx"
    );
    std::fs::remove_dir_all(state).unwrap();
}

#[test]
fn launcher_restore_without_a_journal_says_so() {
    let state = state_dir("restore-none");
    let output = rusttv_with_state(
        "tv",
        &state,
        &["launcher", "restore", "--dry-run", "--json"],
    );

    assert_eq!(output.status.code(), Some(1));
    let json = json_stdout(&output);
    assert_eq!(json["error"]["code"], "NO_JOURNAL");
    std::fs::remove_dir_all(state).unwrap();
}

#[test]
fn launcher_restore_warns_about_unreadable_journals_on_stderr() {
    let state = state_dir("restore-unreadable");
    std::fs::write(
        state.join("journals/1761220000000-home-takeover.toml"),
        TAKEOVER_JOURNAL,
    )
    .unwrap();
    let corrupt = state.join("journals/1761220000001-home-takeover.toml");
    std::fs::write(
        &corrupt,
        TAKEOVER_JOURNAL.replace("disable_package", "retired_command"),
    )
    .unwrap();
    let corrupt = corrupt.display().to_string();

    // The readable journal is still restored; stdout is the usual JSON.
    let output = rusttv_with_state(
        "tv",
        &state,
        &["launcher", "restore", "--dry-run", "--json"],
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        json_stdout(&output)["data"]["plan"],
        "home-takeover-restore"
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("warning:"), "{stderr}");
    assert!(stderr.contains(&format!("--journal {corrupt}")), "{stderr}");
    assert!(stderr.contains("--from-profile"), "{stderr}");

    // Only the unreadable one left: NO_JOURNAL, still with the warning.
    std::fs::remove_file(state.join("journals/1761220000000-home-takeover.toml")).unwrap();
    let output = rusttv_with_state("tv", &state, &["launcher", "restore", "--dry-run"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains(&corrupt), "{stderr}");
    assert!(stderr.contains("no home-takeover journal"), "{stderr}");
    std::fs::remove_dir_all(state).unwrap();
}

/// Like [`rusttv_with_state`], run in `dir` (where the default APK path is looked up).
fn rusttv_in(dir: &std::path::Path, scenario: &str, args: &[&str]) -> Output {
    let fake_adb = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fake-adb");
    rusttv()
        .args(args)
        .current_dir(dir)
        .env("RUSTTV_ADB", fake_adb)
        .env("FAKE_ADB_SCENARIO", scenario)
        .env("RUSTTV_STATE_DIR", dir)
        .output()
        .unwrap()
}

/// Writes a stand-in APK (not a real one) at `dir/relative`.
fn fake_apk(dir: &std::path::Path, relative: &str) -> std::path::PathBuf {
    let path = dir.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"synthetic").unwrap();
    path
}

#[test]
fn launcher_install_dry_run_picks_the_apk_for_the_primary_abi() {
    let dir = state_dir("install-default-apk");
    fake_apk(
        &dir,
        "android/app/build/outputs/apk/release/app-arm64-v8a-release.apk",
    );
    let output = rusttv_in(
        &dir,
        "tv-fresh",
        &["launcher", "install", "--dry-run", "--json"],
    );

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let json = json_stdout(&output);
    assert_eq!(json["data"]["plan"], "launcher-install");
    assert_eq!(json["data"]["confirmation"], "install");
    assert_eq!(json["data"]["max_risk"], "R3");
    let step = &json["data"]["steps"][0];
    let command = step["command"].as_str().unwrap();
    assert!(
        command.starts_with("adb -s emulator-5554 install --pkg ai.rusttv.launcher /"),
        "{command}"
    );
    assert!(
        command.ends_with("/android/app/build/outputs/apk/release/app-arm64-v8a-release.apk"),
        "{command}"
    );
    assert_eq!(step["records"], "ai.rusttv.launcher is not installed");
    assert_eq!(
        step["undo"],
        "adb -s emulator-5554 shell pm uninstall ai.rusttv.launcher"
    );
    assert_eq!(
        std::fs::read_dir(dir.join("journals")).unwrap().count(),
        0,
        "a dry run writes no journal"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn launcher_install_without_a_built_apk_says_how_to_get_one() {
    let dir = state_dir("install-no-apk");
    let output = rusttv_in(
        &dir,
        "tv-fresh",
        &["launcher", "install", "--dry-run", "--json"],
    );

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let json = json_stdout(&output);
    assert_eq!(json["error"]["code"], "INVALID_APK");
    let message = json["error"]["message"].as_str().unwrap();
    assert!(message.contains("app-arm64-v8a-release.apk"), "{message}");
    assert!(message.contains("--apk"), "{message}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn launcher_install_rejects_an_apk_name_with_shell_syntax() {
    let dir = state_dir("install-bad-apk");
    let apk = fake_apk(&dir, "app;reboot.apk");
    let output = rusttv_in(
        &dir,
        "tv-fresh",
        &[
            "launcher",
            "install",
            "--apk",
            apk.to_str().unwrap(),
            "--dry-run",
            "--json",
        ],
    );

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(json_stdout(&output)["error"]["code"], "INVALID_APK");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn launcher_install_over_an_installed_launcher_is_shown_as_an_upgrade() {
    let dir = state_dir("install-upgrade");
    let apk = fake_apk(&dir, "builds/app-x86_64-release.apk");
    let output = rusttv_in(
        &dir,
        "tv",
        &[
            "launcher",
            "install",
            "--apk",
            apk.to_str().unwrap(),
            "--dry-run",
        ],
    );

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("install -r --pkg ai.rusttv.launcher"),
        "{stdout}"
    );
    assert!(stdout.contains("this is an upgrade"), "{stdout}");
    assert!(stdout.contains("can't be undone"), "{stdout}");
    assert!(!stdout.contains("undo:"), "{stdout}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn launcher_uninstall_dry_run_checks_home_opens_the_stock_launcher() {
    let dir = state_dir("uninstall-dry-run");
    let output = rusttv_in(
        &dir,
        "tv",
        &["launcher", "uninstall", "--dry-run", "--json"],
    );

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let json = json_stdout(&output);
    assert_eq!(json["data"]["plan"], "launcher-uninstall");
    assert_eq!(json["data"]["confirmation"], "uninstall");
    let step = &json["data"]["steps"][0];
    assert_eq!(
        step["command"],
        "adb -s emulator-5554 shell pm uninstall ai.rusttv.launcher"
    );
    assert_eq!(
        step["verify"][1],
        "Home opens com.google.android.apps.tv.launcherx"
    );
    assert_eq!(step["undo"], Value::Null);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn launcher_uninstall_is_refused_while_a_takeover_is_active() {
    let dir = state_dir("uninstall-takeover");
    // Synthetic journal of an applied takeover, for the synthetic getprop's firmware.
    let journal = r#"schema_version = 1
plan = "home-takeover"
serial = "emulator-5554"
fingerprint = "google/sdk_google_atv64_arm64/emu64a:14/UTT3.240625.001.K5/12345678:userdebug/dev-keys"
started_at_unix_ms = 1761220000000
status = "applied"

[[steps]]
adb = "adb -s emulator-5554 shell pm disable-user --user 0 com.google.android.apps.tv.launcherx"
command = { kind = "disable_package", arg = "com.google.android.apps.tv.launcherx" }
original_state = { kind = "package_enabled", arg = "com.google.android.apps.tv.launcherx" }
status = "done"
"#;
    std::fs::write(
        dir.join("journals/1761220000000-home-takeover.toml"),
        journal,
    )
    .unwrap();

    let output = rusttv_in(
        &dir,
        "tv",
        &["launcher", "uninstall", "--dry-run", "--json"],
    );

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let json = json_stdout(&output);
    assert_eq!(json["error"]["code"], "PRECONDITION_FAILED");
    let message = json["error"]["message"].as_str().unwrap();
    assert!(message.contains("rusttv launcher restore"), "{message}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn launcher_install_and_uninstall_without_a_terminal_are_refused_before_touching_a_device() {
    for command in ["install", "uninstall"] {
        let output = rusttv_with_fake_adb("no-devices", &["launcher", command, "--json"]);
        assert_eq!(output.status.code(), Some(1), "{command}");
        assert_eq!(
            json_stdout(&output)["error"]["code"],
            "NOT_APPROVED",
            "{command}"
        );
    }
}

/// `ro.build.fingerprint` of the synthetic getprop fixture (scenario `tv`).
const TV_FINGERPRINT: &str =
    "google/sdk_google_atv64_arm64/emu64a:14/UTT3.240625.001.K5/12345678:userdebug/dev-keys";

fn now_unix_ms() -> u64 {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    u64::try_from(elapsed.as_millis()).unwrap()
}

/// Writes a launcher-scope grant for the `tv` scenario's device into `state`,
/// valid from `granted` to `expires` (unix ms), as `rusttv approve` would.
fn write_grant(state: &std::path::Path, granted: u64, expires: u64) {
    std::fs::write(
        state.join("approval-grant.toml"),
        format!(
            "schema_version = 1\nserial = \"emulator-5554\"\nfingerprint = \"{TV_FINGERPRINT}\"\n\
             scope = \"launcher\"\ngranted_at_unix_ms = {granted}\nexpires_at_unix_ms = {expires}\n"
        ),
    )
    .unwrap();
}

const MINUTE_MS: u64 = 60_000;

#[test]
fn approve_without_a_terminal_is_refused_and_grants_nothing() {
    let state = state_dir("approve-no-terminal");
    let output = rusttv_with_state(
        "tv",
        &state,
        &["approve", "--for", "30m", "--only", "launcher", "--json"],
    );

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let json = json_stdout(&output);
    assert_eq!(json["error"]["code"], "NOT_APPROVED");
    let message = json["error"]["message"].as_str().unwrap();
    assert!(message.contains("interactive terminal"), "{message}");
    assert!(!state.join("approval-grant.toml").exists());

    // Longer than 2 h, or no scope: usage errors.
    let output = rusttv_with_state(
        "tv",
        &state,
        &["approve", "--for", "3h", "--only", "launcher"],
    );
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let output = rusttv_with_state("tv", &state, &["approve", "--for", "30m"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    std::fs::remove_dir_all(state).unwrap();
}

#[test]
fn approve_status_shows_the_grant_and_revoke_deletes_it() {
    let state = state_dir("approve-status");
    let status = |state: &std::path::Path| {
        let output = rusttv_with_state("no-devices", state, &["approve", "--status", "--json"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        json_stdout(&output)["data"].clone()
    };
    let data = status(&state);
    assert_eq!(data["grant"], Value::Null);
    assert_eq!(data["valid"], false);

    let now = now_unix_ms();
    write_grant(&state, now - MINUTE_MS, now + 29 * MINUTE_MS);
    let data = status(&state);
    assert_eq!(data["valid"], true, "{data}");
    assert_eq!(data["reason"], Value::Null);
    assert_eq!(data["grant"]["serial"], "emulator-5554");
    assert_eq!(data["grant"]["scope"], "launcher");

    write_grant(&state, now - 60 * MINUTE_MS, now - 30 * MINUTE_MS);
    let data = status(&state);
    assert_eq!(data["valid"], false);
    assert!(
        data["reason"].as_str().unwrap().contains("expired"),
        "{data}"
    );

    // Revoking needs no terminal and no device.
    let revoke = |state: &std::path::Path| {
        let output = rusttv_with_state("no-devices", state, &["approve", "--revoke", "--json"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        json_stdout(&output)["data"]["revoked"].clone()
    };
    assert_eq!(revoke(&state), true);
    assert!(!state.join("approval-grant.toml").exists());
    assert_eq!(revoke(&state), false);
    std::fs::remove_dir_all(state).unwrap();
}

#[test]
fn launcher_install_without_a_terminal_applies_only_with_a_valid_grant() {
    let dir = state_dir("install-grant");
    let apk = fake_apk(&dir, "builds/app-x86_64-release.apk");
    let install = [
        "launcher",
        "install",
        "--apk",
        apk.to_str().unwrap(),
        "--json",
    ];
    let now = now_unix_ms();

    // Expired: refused before the device is touched, pointing to `rusttv approve`.
    write_grant(&dir, now - 60 * MINUTE_MS, now - 30 * MINUTE_MS);
    let output = rusttv_in(&dir, "no-devices", &install);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let json = json_stdout(&output);
    assert_eq!(json["error"]["code"], "NOT_APPROVED");
    let message = json["error"]["message"].as_str().unwrap();
    assert!(message.contains("rusttv approve"), "{message}");
    assert!(message.contains("expired"), "{message}");

    // Valid: the upgrade is applied and the journal says how it was approved.
    let granted = now - MINUTE_MS;
    write_grant(&dir, granted, now + 29 * MINUTE_MS);
    let output = rusttv_in(&dir, "tv", &install);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = &json_stdout(&output)["data"];
    assert_eq!(data["status"], "applied");
    let journal = std::fs::read_to_string(data["journal"].as_str().unwrap()).unwrap();
    assert!(journal.contains("by = \"delegation\""), "{journal}");
    assert!(
        journal.contains(&format!("granted_at_unix_ms = {granted}")),
        "{journal}"
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("Approved by the delegation granted at"),
        "{stderr}"
    );

    // The launcher scope never covers uninstalling.
    let output = rusttv_in(&dir, "tv", &["launcher", "uninstall", "--json"]);
    assert_eq!(json_stdout(&output)["error"]["code"], "NOT_APPROVED");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn launcher_upgrade_dry_run_without_a_takeover_is_the_install_plan() {
    let dir = state_dir("upgrade-no-takeover");
    let apk = fake_apk(&dir, "builds/app-x86_64-release.apk");
    let output = rusttv_in(
        &dir,
        "tv",
        &[
            "launcher",
            "upgrade",
            "--apk",
            apk.to_str().unwrap(),
            "--dry-run",
            "--json",
        ],
    );

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = &json_stdout(&output)["data"];
    assert_eq!(data["takeover_active"], false);
    assert_eq!(data["confirmation"], "upgrade");
    let stages = data["stages"].as_array().unwrap();
    assert_eq!(stages.len(), 1);
    assert_eq!(stages[0]["stage"], "install");
    assert_eq!(stages[0]["plan"]["plan"], "launcher-install");
    assert_eq!(stages[0]["plan"]["confirmation"], "upgrade");
    let command = stages[0]["plan"]["steps"][0]["command"].as_str().unwrap();
    assert!(
        command.contains("install -r --pkg ai.rusttv.launcher"),
        "{command}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn launcher_upgrade_dry_run_during_a_takeover_shows_the_restore_and_what_follows() {
    let dir = state_dir("upgrade-takeover");
    std::fs::write(
        dir.join("journals/1761220000000-home-takeover.toml"),
        TAKEOVER_JOURNAL,
    )
    .unwrap();
    let apk = fake_apk(&dir, "builds/app-x86_64-release.apk");
    let args = [
        "launcher",
        "upgrade",
        "--apk",
        apk.to_str().unwrap(),
        "--dry-run",
    ];
    let output = rusttv_in(&dir, "tv", &[&args[..], &["--json"]].concat());

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = &json_stdout(&output)["data"];
    assert_eq!(data["takeover_active"], true);
    assert_eq!(data["confirmation"], "upgrade");
    let stages = data["stages"].as_array().unwrap();
    let names: Vec<&str> = stages
        .iter()
        .map(|stage| stage["stage"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["restore", "install", "takeover"]);
    assert_eq!(stages[0]["plan"]["plan"], "home-takeover-restore");
    assert_eq!(stages[1]["plan"], Value::Null);
    let install = stages[1]["description"].as_str().unwrap();
    assert!(install.contains(apk.to_str().unwrap()), "{install}");
    assert_eq!(stages[2]["plan"], Value::Null);
    let takeover = stages[2]["description"].as_str().unwrap();
    assert!(
        takeover.contains("com.google.android.apps.tv.launcherx"),
        "{takeover}"
    );

    let output = rusttv_in(&dir, "tv", &args);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("pm enable --user 0"), "{stdout}");
    assert!(stdout.contains("three stages"), "{stdout}");
    // A dry run changes nothing: the takeover journal is still restorable.
    let journal =
        std::fs::read_to_string(dir.join("journals/1761220000000-home-takeover.toml")).unwrap();
    assert_eq!(journal, TAKEOVER_JOURNAL);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn optimize_analyze_json_lists_catalog_packages_with_state_and_testing() {
    let output = rusttv_with_fake_adb("philips", &["optimize", "analyze", "--json"]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = &json_stdout(&output)["data"];
    assert_eq!(data["serial"], "emulator-5554");
    assert_eq!(data["profile"], "tpv-ph1m-ww-9972.toml");
    assert_eq!(data["choices"], Value::Null);
    assert_eq!(
        data["categories"],
        serde_json::json!(["telemetry", "ads", "extras"])
    );
    let packages = data["packages"].as_array().unwrap();
    // The thirteen tested packages and two untested ones of the fixture.
    assert_eq!(packages.len(), 15);
    let first = &packages[0];
    assert_eq!(first["package"], "org.droidtv.candeebug");
    assert_eq!(first["source"], "catalog");
    assert_eq!(first["category"], "telemetry");
    assert_eq!(first["default"], true);
    assert_eq!(first["tested"], true);
    assert_eq!(first["checked"], "2026-10-23");
    assert_eq!(first["chosen"], true);
    assert_eq!(first["state"], "enabled");
    assert_eq!(first["protected"], Value::Null);
    assert_eq!(packages[1]["package"], "org.droidtv.gel");
    assert_eq!(packages[1]["state"], "disabled");
    let entry = |name: &str| {
        packages
            .iter()
            .find(|entry| entry["package"] == name)
            .unwrap()
    };
    let usagelogger = entry("org.droidtv.usagelogger");
    assert_eq!(
        (usagelogger["tested"].clone(), usagelogger["chosen"].clone()),
        (false.into(), true.into())
    );
    let cast = entry("com.google.android.apps.mediashell");
    assert_eq!(cast["category"], "casting");
    assert_eq!(cast["default"], false);
    assert_eq!(cast["chosen"], false);
}

#[test]
fn optimize_analyze_text_shows_tested_dates_and_opt_in() {
    let output = rusttv_with_fake_adb("philips", &["optimize", "analyze"]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("Profile: tpv-ph1m-ww-9972.toml"),
        "{stdout}"
    );
    let line = |package: &str| {
        stdout
            .lines()
            .find(|line| line.ends_with(package))
            .unwrap_or_else(|| panic!("{package} missing: {stdout}"))
            .split_whitespace()
            .collect::<Vec<_>>()
    };
    assert_eq!(
        line("org.droidtv.gel"),
        [
            "telemetry",
            "default",
            "disabled",
            "2026-10-23",
            "yes",
            "org.droidtv.gel"
        ]
    );
    assert_eq!(
        line("com.google.android.apps.mediashell"),
        [
            "casting",
            "opt-in",
            "enabled",
            "no",
            "no",
            "com.google.android.apps.mediashell"
        ]
    );
}

/// The packages a `--dry-run --json` plan disables, in order.
fn planned_packages(data: &Value) -> Vec<String> {
    data["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|step| {
            step["command"]
                .as_str()
                .unwrap()
                .rsplit(' ')
                .next()
                .unwrap()
                .to_owned()
        })
        .collect()
}

fn notes(data: &Value) -> Vec<&str> {
    data["notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|note| note.as_str().unwrap())
        .collect()
}

#[test]
fn optimize_apply_privacy_dry_run_plans_the_tested_packages_of_the_categories() {
    // Packages tested on the Philips (2026-10-23). A dry run plans
    // one step per enabled tested package and writes no journal.
    let state = state_dir("privacy-dry-run");
    let output = rusttv_with_state(
        "philips",
        &state,
        &[
            "optimize",
            "apply",
            "privacy",
            "--only",
            "telemetry,ads",
            "--dry-run",
            "--json",
        ],
    );

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = &json_stdout(&output)["data"];
    // gel is already disabled in this scenario, so it is skipped.
    assert_eq!(
        planned_packages(data),
        [
            "org.droidtv.candeebug",
            "com.google.android.partnersetup",
            "com.google.android.feedback",
            "com.android.tv.feedbackconsent",
            "com.android.adservices.api",
            "com.android.ondevicepersonalization.services",
            "com.android.federatedcompute.services",
        ]
    );
    assert!(
        notes(data).iter().any(
            |note| note.starts_with("Not planned, UNTESTED on this model")
                && note.contains("org.droidtv.usagelogger")
        ),
        "{data}"
    );
    assert!(
        !state.join("journals").exists()
            || std::fs::read_dir(state.join("journals")).unwrap().count() == 0
    );
    std::fs::remove_dir_all(state).ok();
}

#[test]
fn optimize_apply_privacy_allow_untested_marks_each_untested_package() {
    let output = rusttv_with_fake_adb(
        "philips",
        &[
            "optimize",
            "apply",
            "privacy",
            "--only",
            "telemetry",
            "--allow-untested",
            "--dry-run",
            "--json",
        ],
    );

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = &json_stdout(&output)["data"];
    let planned = planned_packages(data);
    assert_eq!(planned.last().unwrap(), "org.droidtv.usagelogger");
    assert!(
        notes(data)
            .iter()
            .any(|note| note
                .starts_with("org.droidtv.usagelogger (telemetry): UNTESTED on this model.")),
        "{data}"
    );
}

#[test]
fn optimize_apply_privacy_reads_the_choices_file() {
    let config = state_dir("privacy-choices");
    std::fs::write(
        config.join("privacy.toml"),
        "categories = [\"casting\"]\nalso = [\"org.droidtv.usagelogger\"]\n",
    )
    .unwrap();
    let dry_run = |extra: &[&str]| {
        let mut args = vec!["optimize", "apply", "privacy", "--dry-run", "--json"];
        args.extend_from_slice(extra);
        let fake_adb = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fake-adb");
        rusttv()
            .args(&args)
            .env("RUSTTV_ADB", fake_adb)
            .env("FAKE_ADB_SCENARIO", "philips")
            .env("RUSTTV_CONFIG_DIR", &config)
            .output()
            .unwrap()
    };

    // Opt-in casting and the usage logger, neither tested on the Philips.
    let output = dry_run(&[]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let json = json_stdout(&output);
    assert_eq!(json["error"]["code"], "PROFILE_NOT_TESTED");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("--allow-untested"),
        "{json}"
    );

    let output = dry_run(&["--allow-untested"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        planned_packages(&json_stdout(&output)["data"]),
        [
            "org.droidtv.usagelogger",
            "com.google.android.apps.mediashell"
        ]
    );

    // `--choices` points elsewhere; a broken file is refused, never ignored.
    let broken = config.join("broken.toml");
    std::fs::write(&broken, "keep = \"org.droidtv.gel\"\n").unwrap();
    let output = dry_run(&["--choices", broken.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(json_stdout(&output)["error"]["code"], "INVALID_CHOICES");

    // `optimize analyze` names the file it read.
    let fake_adb = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fake-adb");
    let output = rusttv()
        .args(["optimize", "analyze", "--json"])
        .env("RUSTTV_ADB", fake_adb)
        .env("FAKE_ADB_SCENARIO", "philips")
        .env("RUSTTV_CONFIG_DIR", &config)
        .output()
        .unwrap();
    let data = &json_stdout(&output)["data"];
    assert!(
        data["choices"].as_str().unwrap().ends_with("privacy.toml"),
        "{data}"
    );
    assert_eq!(data["categories"], serde_json::json!(["casting"]));
    std::fs::remove_dir_all(config).ok();
}

#[test]
fn optimize_apply_rejects_an_unknown_category() {
    for bad in ["bloat", "smart_home"] {
        let output = rusttv_with_fake_adb(
            "philips",
            &["optimize", "apply", "privacy", "--only", bad, "--dry-run"],
        );
        assert_eq!(output.status.code(), Some(2), "{bad}");
    }
    // An opt-in category isn't chosen by default: `--only` narrows, never adds.
    let output = rusttv_with_fake_adb(
        "philips",
        &[
            "optimize",
            "apply",
            "privacy",
            "--only",
            "smart-home",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(json_stdout(&output)["error"]["code"], "PRECONDITION_FAILED");
}

#[test]
fn optimize_apply_and_restore_without_a_terminal_are_refused_before_touching_a_device() {
    for command in [
        &["optimize", "apply", "privacy", "--json"][..],
        &["optimize", "apply", "reduced-animations", "--json"][..],
        &[
            "optimize",
            "apply",
            "reduced-animations",
            "--allow-untested",
            "--json",
        ][..],
        &["optimize", "restore", "--json"][..],
        &["optimize", "restore", "reduced-animations", "--json"][..],
    ] {
        let output = rusttv_with_fake_adb("no-devices", command);
        assert_eq!(output.status.code(), Some(1), "{command:?}");
        assert_eq!(
            json_stdout(&output)["error"]["code"],
            "NOT_APPROVED",
            "{command:?}"
        );
    }
}

/// A privacy journal for the Philips getprop fixture's fingerprint.
const PRIVACY_JOURNAL: &str = r#"schema_version = 1
plan = "privacy"
serial = "emulator-5554"
fingerprint = "Philips/PH1M_WW_9972/PH1M_WW_9972:14/UKN2.241117.001/3000518:user/release-keys"
started_at_unix_ms = 1761220000000
status = "applied"

[[steps]]
adb = "adb -s emulator-5554 shell pm disable-user --user 0 org.droidtv.candeebug"
command = { kind = "disable_package", arg = "org.droidtv.candeebug" }
original_state = { kind = "package_enabled", arg = "org.droidtv.candeebug" }
status = "done"

[[steps]]
adb = "adb -s emulator-5554 shell pm disable-user --user 0 org.droidtv.gel"
command = { kind = "disable_package", arg = "org.droidtv.gel" }
original_state = { kind = "package_enabled", arg = "org.droidtv.gel" }
status = "done"
"#;

#[test]
fn optimize_restore_dry_run_re_enables_only_what_is_still_disabled() {
    let state = state_dir("privacy-restore");
    let journal = state.join("journals/1761220000000-privacy.toml");
    std::fs::write(&journal, PRIVACY_JOURNAL).unwrap();
    // A newer takeover journal next to it is not a privacy journal.
    std::fs::write(
        state.join("journals/1761220000001-home-takeover.toml"),
        TAKEOVER_JOURNAL,
    )
    .unwrap();

    let output = rusttv_with_state(
        "philips",
        &state,
        &["optimize", "restore", "--dry-run", "--json"],
    );

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = &json_stdout(&output)["data"];
    assert_eq!(data["plan"], "privacy-restore");
    assert_eq!(data["confirmation"], "restore");
    let steps = data["steps"].as_array().unwrap();
    // candeebug is enabled on the device again: skipped, and said so.
    assert_eq!(steps.len(), 1);
    assert_eq!(
        steps[0]["command"],
        "adb -s emulator-5554 shell pm enable --user 0 org.droidtv.gel"
    );
    let notes: Vec<&str> = data["notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|note| note.as_str().unwrap())
        .collect();
    assert!(
        notes
            .iter()
            .any(|note| note.starts_with("Skipped org.droidtv.candeebug")),
        "{notes:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&journal).unwrap(),
        PRIVACY_JOURNAL,
        "a dry run doesn't write the journal"
    );

    // `launcher restore` doesn't pick up the privacy journal.
    std::fs::remove_file(state.join("journals/1761220000001-home-takeover.toml")).unwrap();
    let output = rusttv_with_state(
        "philips",
        &state,
        &["launcher", "restore", "--dry-run", "--json"],
    );
    assert_eq!(json_stdout(&output)["error"]["code"], "NO_JOURNAL");
    std::fs::remove_dir_all(state).unwrap();
}

#[test]
fn optimize_restore_without_a_journal_says_so() {
    let state = state_dir("privacy-restore-none");
    let output = rusttv_with_state(
        "philips",
        &state,
        &["optimize", "restore", "--dry-run", "--json"],
    );

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let json = json_stdout(&output);
    assert_eq!(json["error"]["code"], "NO_JOURNAL");
    let message = json["error"]["message"].as_str().unwrap();
    assert!(message.contains("no privacy journal"), "{message}");
    assert!(!message.contains("--from-profile"), "{message}");
    std::fs::remove_dir_all(state).unwrap();
}

#[test]
fn optimize_analyze_reports_the_optimizer_profile_and_current_settings() {
    let output = rusttv_with_fake_adb("philips", &["optimize", "analyze", "--json"]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = &json_stdout(&output)["data"];
    let optimizer = data["optimizer"].as_array().unwrap();
    assert_eq!(optimizer.len(), 1);
    let profile = &optimizer[0];
    assert_eq!(profile["profile"], "reduced-animations");
    assert_eq!(profile["version"], 1);
    // Measured A–B–A on the Philips (2026-10-23): tested, with evidence.
    assert_eq!(profile["status"], "suggested");
    assert_eq!(profile["tested"], true);
    assert_eq!(
        profile["evidence"],
        "benchmarks/raw/philips-65oled818/2026-10-23/reduced-animations/report.md"
    );
    assert_eq!(profile["journal"], Value::Null);
    assert_eq!(
        profile["settings"],
        serde_json::json!([
            {"key": "window_animation_scale", "value": "0.5", "current": "1.0", "read_error": null},
            {"key": "transition_animation_scale", "value": "0.5", "current": null, "read_error": null},
            {"key": "animator_duration_scale", "value": "0.5", "current": "1.0", "read_error": null},
        ])
    );

    let output = rusttv_with_fake_adb("philips", &["optimize", "analyze"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("reduced-animations v1: suggested"),
        "{stdout}"
    );
    assert!(
        stdout.contains("transition_animation_scale   now unset   profile 0.5"),
        "{stdout}"
    );
}

#[test]
fn optimize_apply_reduced_animations_is_refused_on_an_untested_model() {
    // The synthetic Google TV device matches no compat profile.
    let state = state_dir("animations-untested");
    let output = rusttv_with_state(
        "tv-fresh",
        &state,
        &[
            "optimize",
            "apply",
            "reduced-animations",
            "--dry-run",
            "--json",
        ],
    );

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let json = json_stdout(&output);
    assert_eq!(json["error"]["code"], "OPTIMIZE_NOT_TESTED");
    let message = json["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("no profile in compat/devices/"),
        "{message}"
    );
    assert!(message.contains("--allow-untested"), "{message}");
    std::fs::remove_dir_all(state).unwrap();
}

#[test]
fn optimize_apply_reduced_animations_dry_run_plans_three_settings_on_the_tested_philips() {
    let state = state_dir("animations-dry-run");
    let output = rusttv_with_state(
        "philips",
        &state,
        &[
            "optimize",
            "apply",
            "reduced-animations",
            "--dry-run",
            "--json",
        ],
    );

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = &json_stdout(&output)["data"];
    assert_eq!(data["plan"], "optimize-reduced-animations-v1");
    assert_eq!(data["max_risk"], "R2");
    assert_eq!(data["confirmation"], "optimize");
    let steps = data["steps"].as_array().unwrap();
    let commands: Vec<&str> = steps
        .iter()
        .map(|step| step["command"].as_str().unwrap())
        .collect();
    assert_eq!(
        commands,
        [
            "adb -s emulator-5554 shell settings put global window_animation_scale 0.5",
            "adb -s emulator-5554 shell settings put global transition_animation_scale 0.5",
            "adb -s emulator-5554 shell settings put global animator_duration_scale 0.5",
        ]
    );
    assert_eq!(steps[1]["records"], "transition_animation_scale is unset");
    assert_eq!(
        steps[1]["undo"],
        "adb -s emulator-5554 shell settings delete global transition_animation_scale"
    );
    assert!(
        steps[2]["verify"][1]
            .as_str()
            .unwrap()
            .starts_with("Home opens "),
        "{data}"
    );
    assert!(
        !notes(data)
            .iter()
            .any(|note| note.starts_with("UNTESTED on this model")),
        "{data}"
    );
    assert_eq!(
        std::fs::read_dir(state.join("journals")).unwrap().count(),
        0,
        "a dry run writes no journal"
    );

    // The text dry run, as the person sees it.
    let output = rusttv_with_state(
        "philips",
        &state,
        &[
            "optimize",
            "apply",
            "reduced-animations",
            "--allow-untested",
            "--dry-run",
        ],
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains(
            "1. [R2] adb -s emulator-5554 shell settings put global window_animation_scale 0.5\n   \
             records first: window_animation_scale is 1.0\n   \
             then checks:   window_animation_scale is 0.5\n   \
             undo:          adb -s emulator-5554 shell settings put global window_animation_scale 1.0"
        ),
        "{stdout}"
    );
    std::fs::remove_dir_all(state).unwrap();
}

/// A `reduced-animations` journal for the Philips getprop fixture's
/// fingerprint: window set from 1.0, transition and animator from unset.
const ANIMATIONS_JOURNAL: &str = r#"schema_version = 1
plan = "optimize-reduced-animations-v1"
serial = "emulator-5554"
fingerprint = "Philips/PH1M_WW_9972/PH1M_WW_9972:14/UKN2.241117.001/3000518:user/release-keys"
started_at_unix_ms = 1761220000002
status = "applied"

[[steps]]
adb = "adb -s emulator-5554 shell settings put global window_animation_scale 0.5"
command = { kind = "set_setting", arg = { key = "window_animation_scale", to = { value = "0.5" }, from = { value = "1.0" } } }
original_state = { kind = "setting_is", arg = { key = "window_animation_scale", state = { value = "1.0" } } }
status = "done"

[[steps]]
adb = "adb -s emulator-5554 shell settings put global transition_animation_scale 0.5"
command = { kind = "set_setting", arg = { key = "transition_animation_scale", to = { value = "0.5" }, from = "unset" } }
original_state = { kind = "setting_is", arg = { key = "transition_animation_scale", state = "unset" } }
status = "done"

[[steps]]
adb = "adb -s emulator-5554 shell settings put global animator_duration_scale 0.5"
command = { kind = "set_setting", arg = { key = "animator_duration_scale", to = { value = "0.5" }, from = "unset" } }
original_state = { kind = "setting_is", arg = { key = "animator_duration_scale", state = "unset" } }
status = "done"
"#;

#[test]
fn optimize_restore_picks_the_one_kind_restorable_or_asks_which() {
    let state = state_dir("animations-restore");
    let optimizer = state.join("journals/1761220000002-optimize-reduced-animations-v1.toml");
    std::fs::write(&optimizer, ANIMATIONS_JOURNAL).unwrap();

    // Only an optimizer journal: `optimize restore` takes it. On the fake TV
    // window is back at 1.0 and transition unset again (both skipped), and
    // animator reads 1.0, neither the 0.5 set nor the unset found (left as
    // is). So nothing is left to do.
    let output = rusttv_with_state(
        "philips",
        &state,
        &["optimize", "restore", "--dry-run", "--json"],
    );
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let json = json_stdout(&output);
    assert_eq!(json["error"]["code"], "PRECONDITION_FAILED");
    let message = json["error"]["message"].as_str().unwrap();
    assert!(message.contains("nothing to restore"), "{message}");
    assert!(
        message.contains("Skipped animator_duration_scale: it is 1.0 now, not the 0.5 RustTV set"),
        "{message}"
    );
    assert_eq!(
        std::fs::read_to_string(&optimizer).unwrap(),
        ANIMATIONS_JOURNAL,
        "a dry run doesn't mark the journal"
    );

    // With a privacy journal too, a name is needed.
    let privacy = state.join("journals/1761220000000-privacy.toml");
    std::fs::write(&privacy, PRIVACY_JOURNAL).unwrap();
    let output = rusttv_with_state(
        "philips",
        &state,
        &["optimize", "restore", "--dry-run", "--json"],
    );
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let json = json_stdout(&output);
    assert_eq!(json["error"]["code"], "PRECONDITION_FAILED");
    let message = json["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("`rusttv optimize restore privacy`"),
        "{message}"
    );
    assert!(
        message.contains("`rusttv optimize restore reduced-animations`"),
        "{message}"
    );

    // Named, each finds its own journal.
    let output = rusttv_with_state(
        "philips",
        &state,
        &["optimize", "restore", "privacy", "--dry-run", "--json"],
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(json_stdout(&output)["data"]["plan"], "privacy-restore");
    let output = rusttv_with_state(
        "philips",
        &state,
        &[
            "optimize",
            "restore",
            "reduced-animations",
            "--dry-run",
            "--json",
        ],
    );
    let json = json_stdout(&output);
    assert_eq!(json["error"]["code"], "PRECONDITION_FAILED", "{json}");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("nothing to restore"),
        "{json}"
    );

    // Each validator refuses the other kind's journal.
    for (kind, journal, expected) in [
        (
            "reduced-animations",
            &privacy,
            "not an optimize-reduced-animations",
        ),
        ("privacy", &optimizer, "not a privacy journal"),
    ] {
        let output = rusttv_with_state(
            "philips",
            &state,
            &[
                "optimize",
                "restore",
                kind,
                "--journal",
                journal.to_str().unwrap(),
                "--dry-run",
                "--json",
            ],
        );
        let json = json_stdout(&output);
        assert_eq!(json["error"]["code"], "PRECONDITION_FAILED", "{kind}");
        let message = json["error"]["message"].as_str().unwrap();
        assert!(message.contains(expected), "{kind}: {message}");
    }
    // `--journal` without a name goes by the journal's plan.
    let output = rusttv_with_state(
        "philips",
        &state,
        &[
            "optimize",
            "restore",
            "--journal",
            privacy.to_str().unwrap(),
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(json_stdout(&output)["data"]["plan"], "privacy-restore");

    // The restorable optimizer journal blocks a second optimizer plan.
    let output = rusttv_with_state(
        "philips",
        &state,
        &[
            "optimize",
            "apply",
            "reduced-animations",
            "--allow-untested",
            "--dry-run",
            "--json",
        ],
    );
    let json = json_stdout(&output);
    assert_eq!(json["error"]["code"], "PRECONDITION_FAILED");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("one at a time"),
        "{json}"
    );
    std::fs::remove_dir_all(state).unwrap();
}

#[test]
fn optimize_restore_of_an_optimizer_journal_puts_back_only_what_rusttv_left() {
    // Window set from 0.5 to 1.0: the fake TV reads 1.0, what RustTV set,
    // so it is put back to 0.5. Transition reads unset, its original
    // (skipped); animator reads 1.0, changed since (left as is).
    let state = state_dir("animations-restore-steps");
    let journal = ANIMATIONS_JOURNAL
        .replacen(
            "to = { value = \"0.5\" }, from = { value = \"1.0\" }",
            "to = { value = \"1.0\" }, from = { value = \"0.5\" }",
            1,
        )
        .replacen(
            "state = { value = \"1.0\" }",
            "state = { value = \"0.5\" }",
            1,
        );
    std::fs::write(
        state.join("journals/1761220000002-optimize-reduced-animations-v1.toml"),
        &journal,
    )
    .unwrap();
    let output = rusttv_with_state(
        "philips",
        &state,
        &[
            "optimize",
            "restore",
            "reduced-animations",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = &json_stdout(&output)["data"];
    assert_eq!(data["plan"], "optimize-reduced-animations-v1-restore");
    assert_eq!(data["confirmation"], "restore");
    let steps = data["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(
        steps[0]["command"],
        "adb -s emulator-5554 shell settings put global window_animation_scale 0.5"
    );
    let notes = notes(data);
    assert!(
        notes.iter().any(|note| note
            .starts_with("Skipped transition_animation_scale: it is unset again already")),
        "{notes:?}"
    );
    assert!(
        notes.iter().any(|note| note
            .starts_with("Skipped animator_duration_scale: it is 1.0 now, not the 0.5 RustTV set")),
        "{notes:?}"
    );
    std::fs::remove_dir_all(state).unwrap();
}

#[test]
fn optimize_analyze_reports_a_setting_it_cant_read_as_unknown_and_still_lists_privacy() {
    let fake_adb = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fake-adb");
    let analyze = |json: bool| {
        let mut args = vec!["optimize", "analyze"];
        if json {
            args.push("--json");
        }
        rusttv()
            .args(&args)
            .env("RUSTTV_ADB", fake_adb)
            .env("FAKE_ADB_SCENARIO", "philips")
            .env("FAKE_ADB_SETTINGS_FAIL", "1")
            .output()
            .unwrap()
    };

    let output = analyze(true);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = &json_stdout(&output)["data"];
    assert_eq!(data["packages"].as_array().unwrap().len(), 15);
    let profile = &data["optimizer"][0];
    assert_eq!(profile["status"], "unknown");
    let animator = &profile["settings"][2];
    assert_eq!(animator["key"], "animator_duration_scale");
    assert_eq!(animator["current"], Value::Null);
    assert!(
        animator["read_error"]
            .as_str()
            .unwrap()
            .contains("Can't find service: settings"),
        "{animator}"
    );
    assert_eq!(profile["settings"][0]["read_error"], Value::Null);

    let stdout = String::from_utf8(analyze(false).stdout).unwrap();
    assert!(
        stdout.contains("animator_duration_scale      now unknown profile 0.5"),
        "{stdout}"
    );

    // `apply` stays strict.
    let output = rusttv()
        .args([
            "optimize",
            "apply",
            "reduced-animations",
            "--allow-untested",
            "--dry-run",
            "--json",
        ])
        .env("RUSTTV_ADB", fake_adb)
        .env("FAKE_ADB_SCENARIO", "philips")
        .env("FAKE_ADB_SETTINGS_FAIL", "1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(json_stdout(&output)["error"]["code"], "COMMAND_FAILED");
}

#[test]
fn optimize_apply_reduced_animations_is_refused_while_a_journal_file_is_unreadable() {
    let state = state_dir("animations-unreadable");
    let corrupt = state.join("journals/1761220000009-optimize-reduced-animations-v1.toml");
    std::fs::write(&corrupt, "not a journal").unwrap();
    let output = rusttv_with_state(
        "philips",
        &state,
        &[
            "optimize",
            "apply",
            "reduced-animations",
            "--allow-untested",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let json = json_stdout(&output);
    assert_eq!(json["error"]["code"], "PRECONDITION_FAILED");
    let message = json["error"]["message"].as_str().unwrap();
    assert!(
        message.contains(&corrupt.display().to_string()),
        "{message}"
    );
    assert!(message.contains("move the file out of"), "{message}");
    std::fs::remove_dir_all(state).unwrap();
}

// `rusttv update`: fake releases signed with a throwaway test key, read from
// RUSTTV_UPDATE_DIR. RUSTTV_CURL points nowhere, so nothing can reach the
// network. The test build honours RUSTTV_UPDATE_TEST_KEY (debug +
// `test-util`); a release build never does.

use rusttv_core::test_release::{TestSigningKey, write_test_release};

/// A release newer than this build, whatever its version.
const NEWER: &str = "99.0.0";
const NEWER_APK: &str = "rusttv-launcher-99.0.0-arm64-v8a.apk";

/// `<dir>/release` holds a signed release `version` with these files.
fn write_release(
    dir: &std::path::Path,
    version: &str,
    assets: &[(&str, &[u8])],
    key: &TestSigningKey,
) {
    write_test_release(&dir.join("release"), version, assets, key).unwrap();
}

/// `program <args>` with `<dir>` as state dir and `<dir>/release` as the
/// release, the fake adb in `scenario`, and `key` as the trusted key.
fn rusttv_update(
    program: &std::path::Path,
    dir: &std::path::Path,
    scenario: &str,
    key: Option<&TestSigningKey>,
    args: &[&str],
) -> Output {
    let fake_adb = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fake-adb");
    let mut command = Command::new(program);
    command
        .args(args)
        .env(
            "RUSTTV_CONFIG_DIR",
            std::env::temp_dir().join("rusttv-cli-tests-no-config"),
        )
        .env("RUSTTV_STATE_DIR", dir)
        .env("RUSTTV_UPDATE_DIR", dir.join("release"))
        .env("RUSTTV_CURL", "/nonexistent/rusttv-tests/curl")
        .env("RUSTTV_ADB", fake_adb)
        .env("FAKE_ADB_SCENARIO", scenario)
        .env_remove("RUSTTV_UPDATE_TEST_KEY");
    if let Some(key) = key {
        command.env("RUSTTV_UPDATE_TEST_KEY", key.public_key());
    }
    command.output().unwrap()
}

fn rusttv_exe() -> &'static std::path::Path {
    std::path::Path::new(env!("CARGO_BIN_EXE_rusttv"))
}

/// Nothing is left in `<state dir>/updates/` after a run.
fn assert_no_downloads_left(dir: &std::path::Path) {
    let updates = dir.join("updates");
    let left = std::fs::read_dir(&updates).map_or(0, Iterator::count);
    assert_eq!(left, 0, "downloads left in {}", updates.display());
}

#[test]
fn update_check_compares_the_cli_and_the_launcher_with_the_latest_release() {
    let dir = state_dir("update-check");
    let archive = format!("rusttv-{NEWER}-{}.tar.gz", env!("RUSTTV_TARGET"));
    let key = TestSigningKey::new(1);
    write_release(
        &dir,
        NEWER,
        &[(NEWER_APK, b"apk"), (archive.as_str(), b"archive")],
        &key,
    );
    let output = rusttv_update(
        rusttv_exe(),
        &dir,
        "tv",
        None,
        &["update", "check", "--json"],
    );

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = &json_stdout(&output)["data"];
    assert_eq!(data["latest"], NEWER);
    assert_eq!(
        data["page"],
        format!("https://github.com/damienmiheev/rusttv/releases/tag/v{NEWER}")
    );
    assert_eq!(data["can_verify"], false, "no release signing key yet");
    assert_eq!(data["cli"]["current"], env!("CARGO_PKG_VERSION"));
    assert_eq!(data["cli"]["target"], env!("RUSTTV_TARGET"));
    assert_eq!(data["cli"]["update_available"], true);
    assert_eq!(data["cli"]["archive"], archive.as_str());
    assert_eq!(data["launcher"]["serial"], "emulator-5554");
    assert_eq!(data["launcher"]["installed"], "0.1.0");
    assert_eq!(data["launcher"]["abi"], "arm64-v8a");
    assert_eq!(data["launcher"]["update_available"], true);
    assert_eq!(data["launcher"]["apk"], NEWER_APK);
    assert_eq!(data["launcher_note"], Value::Null);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn update_check_without_a_device_still_checks_the_cli() {
    let dir = state_dir("update-check-no-device");
    write_release(&dir, NEWER, &[(NEWER_APK, b"apk")], &TestSigningKey::new(1));
    let output = rusttv_update(
        rusttv_exe(),
        &dir,
        "no-devices",
        None,
        &["update", "check", "--json"],
    );

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = &json_stdout(&output)["data"];
    assert_eq!(data["cli"]["update_available"], true);
    assert_eq!(
        data["cli"]["archive"],
        Value::Null,
        "no archive for this target"
    );
    assert_eq!(data["launcher"], Value::Null);
    let note = data["launcher_note"].as_str().unwrap();
    assert!(note.starts_with("no device checked"), "{note}");

    // With --serial, the device must be there, as for every device command.
    let output = rusttv_update(
        rusttv_exe(),
        &dir,
        "no-devices",
        None,
        &["update", "check", "--serial", "tv", "--json"],
    );
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn update_check_text_says_what_to_run() {
    let dir = state_dir("update-check-text");
    write_release(
        &dir,
        env!("CARGO_PKG_VERSION"),
        &[],
        &TestSigningKey::new(1),
    );
    let output = rusttv_update(rusttv_exe(), &dir, "tv-fresh", None, &["update", "check"]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("(up to date)"), "{stdout}");
    assert!(
        stdout.contains("not installed on emulator-5554"),
        "{stdout}"
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("no release signing key"), "{stderr}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn update_check_refuses_pre_releases_and_bad_tags() {
    let dir = state_dir("update-check-bad-release");
    write_release(&dir, NEWER, &[], &TestSigningKey::new(1));
    let path = dir.join("release/release.json");
    let original: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    for (field, value) in [
        ("prerelease", Value::from(true)),
        ("draft", Value::from(true)),
        ("tag_name", Value::from("v99.0")),
        ("tag_name", Value::from("99.0.0")),
    ] {
        let mut release = original.clone();
        release[field] = value;
        std::fs::write(&path, release.to_string()).unwrap();
        let output = rusttv_update(
            rusttv_exe(),
            &dir,
            "tv",
            None,
            &["update", "check", "--json"],
        );
        assert_eq!(output.status.code(), Some(1), "{field}: {output:?}");
        assert_eq!(json_stdout(&output)["error"]["code"], "UPDATE_CHECK_FAILED");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn update_launcher_dry_run_plans_the_verified_release_apk() {
    let dir = state_dir("update-launcher");
    let key = TestSigningKey::new(1);
    write_release(&dir, NEWER, &[(NEWER_APK, b"signed apk")], &key);
    let output = rusttv_update(
        rusttv_exe(),
        &dir,
        "tv-fresh",
        Some(&key),
        &["update", "launcher", "--dry-run", "--json"],
    );

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = &json_stdout(&output)["data"];
    assert_eq!(data["plan"], "launcher-install");
    assert_eq!(data["confirmation"], "install");
    let command = data["steps"][0]["command"].as_str().unwrap();
    assert!(
        command.starts_with("adb -s emulator-5554 install --pkg ai.rusttv.launcher /"),
        "{command}"
    );
    assert!(command.ends_with(&format!("/{NEWER_APK}")), "{command}");
    let notes = data["notes"].to_string();
    assert!(notes.contains(&format!("release {NEWER}")), "{notes}");
    assert_no_downloads_left(&dir);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn update_launcher_over_an_older_launcher_is_an_upgrade() {
    let dir = state_dir("update-launcher-upgrade");
    let key = TestSigningKey::new(1);
    write_release(&dir, NEWER, &[(NEWER_APK, b"signed apk")], &key);
    let output = rusttv_update(
        rusttv_exe(),
        &dir,
        "tv",
        Some(&key),
        &["update", "launcher", "--dry-run"],
    );

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("install -r --pkg ai.rusttv.launcher"),
        "{stdout}"
    );
    assert!(stdout.contains("this is an upgrade"), "{stdout}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn update_launcher_never_reinstalls_or_downgrades() {
    let dir = state_dir("update-launcher-same");
    let key = TestSigningKey::new(1);
    // The fake TV has launcher 0.1.0.
    for version in ["0.1.0", "0.0.9"] {
        let apk = format!("rusttv-launcher-{version}-arm64-v8a.apk");
        let _ = std::fs::remove_dir_all(dir.join("release"));
        write_release(&dir, version, &[(apk.as_str(), b"apk")], &key);
        let output = rusttv_update(
            rusttv_exe(),
            &dir,
            "tv",
            Some(&key),
            &["update", "launcher", "--dry-run", "--json"],
        );
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let error = &json_stdout(&output)["error"];
        assert_eq!(error["code"], "PRECONDITION_FAILED");
        assert!(
            error["message"].as_str().unwrap().contains("up to date"),
            "{error}"
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn update_launcher_refuses_without_a_signing_key_a_wrong_key_or_a_tampered_apk() {
    let dir = state_dir("update-launcher-verify");
    let key = TestSigningKey::new(1);
    write_release(&dir, NEWER, &[(NEWER_APK, b"signed apk")], &key);
    let args = ["update", "launcher", "--dry-run", "--json"];
    let message = |output: &Output| {
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let error = &json_stdout(output)["error"];
        assert_eq!(error["code"], "UPDATE_VERIFY_FAILED");
        error["message"].as_str().unwrap().to_owned()
    };

    // This build has no release signing key.
    let output = rusttv_update(rusttv_exe(), &dir, "tv-fresh", None, &args);
    assert!(message(&output).contains("no release signing key"));

    let other = TestSigningKey::new(2);
    let output = rusttv_update(rusttv_exe(), &dir, "tv-fresh", Some(&other), &args);
    assert!(message(&output).contains("another key"));

    // Same size, other bytes.
    std::fs::write(dir.join("release").join(NEWER_APK), b"evil   apk").unwrap();
    let output = rusttv_update(rusttv_exe(), &dir, "tv-fresh", Some(&key), &args);
    assert!(message(&output).contains("SHA-256"));
    assert_no_downloads_left(&dir);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn update_launcher_and_cli_without_a_terminal_are_refused() {
    let dir = state_dir("update-no-terminal");
    let key = TestSigningKey::new(1);
    write_release(&dir, NEWER, &[(NEWER_APK, b"apk")], &key);
    for args in [
        ["update", "launcher", "--json"],
        ["update", "cli", "--json"],
    ] {
        let output = rusttv_update(rusttv_exe(), &dir, "tv", Some(&key), &args);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let error = &json_stdout(&output)["error"];
        assert_eq!(error["code"], "NOT_APPROVED", "{args:?}");
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("interactive terminal")
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// `<dir>/bin/rusttv` (a copy of the test binary) and `<dir>/bin/rusttv-mcp`,
/// plus a signed release `NEWER` whose archive holds scripts printing
/// `rusttv <reported>`.
fn installed_cli(
    dir: &std::path::Path,
    reported: &str,
    key: &TestSigningKey,
) -> std::path::PathBuf {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::copy(rusttv_exe(), bin.join("rusttv")).unwrap();
    std::fs::write(bin.join("rusttv-mcp"), b"old mcp").unwrap();

    let name = format!("rusttv-{NEWER}-{}", env!("RUSTTV_TARGET"));
    let src = dir.join("archive-src");
    std::fs::create_dir_all(src.join(&name)).unwrap();
    let script = format!("#!/bin/sh\necho 'rusttv {reported}'\n");
    std::fs::write(src.join(&name).join("rusttv"), &script).unwrap();
    std::fs::write(src.join(&name).join("rusttv-mcp"), b"new mcp").unwrap();
    let archive = dir.join(format!("{name}.tar.gz"));
    let status = Command::new("tar")
        .arg("-czf")
        .arg(&archive)
        .arg("-C")
        .arg(&src)
        .arg(&name)
        .status()
        .unwrap();
    assert!(status.success());
    let bytes = std::fs::read(&archive).unwrap();
    write_release(
        dir,
        NEWER,
        &[(format!("{name}.tar.gz").as_str(), &bytes)],
        key,
    );
    bin.join("rusttv")
}

#[test]
fn update_cli_dry_run_lists_the_files_it_would_replace() {
    let dir = state_dir("update-cli");
    let key = TestSigningKey::new(1);
    let exe = installed_cli(&dir, NEWER, &key);
    let before = std::fs::read(&exe).unwrap();
    let output = rusttv_update(
        &exe,
        &dir,
        "tv",
        Some(&key),
        &["update", "cli", "--dry-run", "--json"],
    );

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = &json_stdout(&output)["data"];
    assert_eq!(data["from"], env!("CARGO_PKG_VERSION"));
    assert_eq!(data["to"], NEWER);
    assert_eq!(
        data["archive"],
        format!("rusttv-{NEWER}-{}.tar.gz", env!("RUSTTV_TARGET"))
    );
    assert_eq!(data["confirmation"], "update");
    assert_eq!(data["applied"], false);
    let files = data["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert!(files[0]["path"].as_str().unwrap().ends_with("/bin/rusttv"));
    assert!(
        files[0]["previous"]
            .as_str()
            .unwrap()
            .ends_with("/bin/rusttv.previous")
    );
    assert!(
        files[1]["path"]
            .as_str()
            .unwrap()
            .ends_with("/bin/rusttv-mcp")
    );
    assert_eq!(
        std::fs::read(&exe).unwrap(),
        before,
        "a dry run replaces nothing"
    );
    assert!(!dir.join("bin/rusttv.previous").exists());
    assert_no_downloads_left(&dir);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn update_cli_refuses_a_tampered_archive_and_the_same_version() {
    let dir = state_dir("update-cli-refused");
    let key = TestSigningKey::new(1);
    let exe = installed_cli(&dir, NEWER, &key);
    let args = ["update", "cli", "--dry-run", "--json"];

    let output = rusttv_update(&exe, &dir, "tv", Some(&TestSigningKey::new(3)), &args);
    assert_eq!(
        json_stdout(&output)["error"]["code"],
        "UPDATE_VERIFY_FAILED"
    );

    let output = rusttv_update(&exe, &dir, "tv", None, &args);
    let error = &json_stdout(&output)["error"];
    assert_eq!(error["code"], "UPDATE_VERIFY_FAILED");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("no release signing key")
    );

    let _ = std::fs::remove_dir_all(dir.join("release"));
    write_release(&dir, env!("CARGO_PKG_VERSION"), &[], &key);
    let output = rusttv_update(&exe, &dir, "tv", Some(&key), &args);
    let error = &json_stdout(&output)["error"];
    assert_eq!(error["code"], "PRECONDITION_FAILED");
    assert!(
        error["message"].as_str().unwrap().contains("up to date"),
        "{error}"
    );
    assert_no_downloads_left(&dir);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn update_cli_refuses_a_development_build() {
    let dir = state_dir("update-cli-dev");
    let key = TestSigningKey::new(1);
    write_release(&dir, NEWER, &[], &key);
    // The test binary itself lives in target/debug.
    let output = rusttv_update(
        rusttv_exe(),
        &dir,
        "tv",
        Some(&key),
        &["update", "cli", "--dry-run", "--json"],
    );

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let error = &json_stdout(&output)["error"];
    assert_eq!(error["code"], "PRECONDITION_FAILED");
    assert!(
        error["message"].as_str().unwrap().contains("git pull"),
        "{error}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
