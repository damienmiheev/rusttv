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
    assert_eq!(value["schema"], "rusttv.v4");
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

#[test]
fn optimize_analyze_json_lists_the_privacy_profile_with_package_states() {
    let output = rusttv_with_fake_adb("philips", &["optimize", "analyze", "--json"]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = &json_stdout(&output)["data"];
    assert_eq!(data["serial"], "emulator-5554");
    assert_eq!(data["profile"], "tpv-ph1m-ww-9972.toml");
    assert_eq!(data["privacy"]["tested"], true);
    let packages = data["privacy"]["packages"].as_array().unwrap();
    assert_eq!(packages.len(), 12);
    assert_eq!(packages[0]["package"], "org.droidtv.candeebug");
    assert_eq!(packages[0]["category"], "telemetry");
    assert_eq!(packages[0]["state"], "enabled");
    assert_eq!(packages[0]["protected"], Value::Null);
    assert_eq!(packages[1]["package"], "org.droidtv.gel");
    assert_eq!(packages[1]["state"], "disabled");
    assert_eq!(packages[11]["category"], "extras");
}

#[test]
fn optimize_analyze_text_says_the_profile_is_tested() {
    let output = rusttv_with_fake_adb("philips", &["optimize", "analyze"]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("Privacy profile: tested on this model."),
        "{stdout}"
    );
    assert!(
        stdout.contains("telemetry  disabled  org.droidtv.gel"),
        "{stdout}"
    );
}

#[test]
fn optimize_apply_privacy_dry_run_plans_the_chosen_categories() {
    // The bundled Philips privacy profile is tested (2026-10-23). A dry run
    // plans one step per enabled package and writes no journal.
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
    let packages: Vec<&str> = data["steps"]
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
        })
        .collect();
    // gel is already disabled in this scenario, so it is skipped.
    assert_eq!(
        packages,
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
        !state.join("journals").exists()
            || std::fs::read_dir(state.join("journals")).unwrap().count() == 0
    );
    std::fs::remove_dir_all(state).ok();
}

#[test]
fn optimize_apply_rejects_an_unknown_category() {
    let output = rusttv_with_fake_adb(
        "philips",
        &[
            "optimize",
            "apply",
            "privacy",
            "--only",
            "bloat",
            "--dry-run",
        ],
    );
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn optimize_apply_and_restore_without_a_terminal_are_refused_before_touching_a_device() {
    for command in [
        &["optimize", "apply", "privacy", "--json"][..],
        &["optimize", "restore", "--json"][..],
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
