//! Drives the MCP server over an in-memory stdio pipe with `FakeAdb`, speaking
//! raw newline-delimited JSON-RPC like an MCP client does.

#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test helpers should panic on unexpected failures; clippy only exempts #[test] fns"
)]

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rmcp::ServiceExt;
use rusttv_core::{
    Adb, AdbError, Check, ConnectOutcome, DeviceAddress, DeviceCommand, DeviceEntry, DeviceState,
    FakeAdb, FileJournal, HOME_TAKEOVER, JOURNAL_SCHEMA_VERSION, Journal, JournalStatus,
    JournalStep, JournalStore, PackageName, RiskClass, StepStatus, inspect, install_plan,
    launcher_status, list_apps, parse_device_list, snapshot, takeover_restore_plan, uninstall_plan,
};
use rusttv_mcp::RustTvServer;
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines, ReadHalf, WriteHalf,
};

const SERIAL: &str = "emulator-5554";

macro_rules! capture {
    ($device:literal, $file:literal) => {
        include_str!(concat!(
            "../../rusttv-core/src/fixtures/",
            $device,
            "/",
            $file
        ))
    };
}

/// A fake emulator that answers every command the read-only tools run.
fn emulator() -> FakeAdb {
    let mut adb = FakeAdb::new()
        .respond(
            DeviceCommand::GetProperties,
            capture!("emulator-pixel3a-api34", "getprop.txt"),
        )
        .respond(
            DeviceCommand::MemInfo,
            capture!("emulator-pixel3a-api34", "meminfo.txt"),
        )
        .respond(
            DeviceCommand::DataStorage,
            capture!("emulator-pixel3a-api34", "df-data.txt"),
        )
        .respond(
            DeviceCommand::ListPackages,
            capture!("emulator-pixel3a-api34", "packages-f.txt"),
        )
        .respond(
            DeviceCommand::ListThirdPartyPackages,
            capture!("emulator-pixel3a-api34", "packages-3.txt"),
        )
        .respond(
            DeviceCommand::ListDisabledPackages,
            capture!("emulator-pixel3a-api34", "packages-d.txt"),
        )
        .respond_sequence(
            DeviceCommand::CpuStat,
            [
                capture!("emulator-pixel3a-api34", "proc-stat-1.txt").to_owned(),
                capture!("emulator-pixel3a-api34", "proc-stat-2.txt").to_owned(),
            ],
        )
        .respond(
            DeviceCommand::ProcessList,
            capture!("emulator-pixel3a-api34", "ps.txt"),
        )
        .respond(
            DeviceCommand::HomeActivity,
            capture!("google-tv-emulator-api34", "home-activity.txt"),
        )
        .respond(
            DeviceCommand::HomeCandidates,
            capture!("google-tv-emulator-api34", "home-candidates.txt"),
        )
        // The Pixel's launcher memory, served for the Google TV launcher above.
        .respond(
            DeviceCommand::PackageMemInfo(
                PackageName::new("com.google.android.apps.tv.launcherx").unwrap(),
            ),
            capture!("emulator-pixel3a-api34", "meminfo-launcher.txt"),
        );
    for device in parse_device_list(capture!("emulator-pixel3a-api34", "devices-l.txt")) {
        adb = adb.with_device(device);
    }
    adb
}

fn device(serial: &str, state: DeviceState) -> DeviceEntry {
    DeviceEntry {
        serial: serial.to_owned(),
        state,
        model: None,
        product: None,
        device: None,
    }
}

/// The client end of an initialized MCP session.
struct Client {
    writer: WriteHalf<DuplexStream>,
    lines: Lines<BufReader<ReadHalf<DuplexStream>>>,
    next_id: u64,
}

impl Client {
    async fn start(adb: FakeAdb) -> Self {
        Self::serve(RustTvServer::new(adb)).await
    }

    async fn serve<A: Adb + Send + 'static>(server: RustTvServer<A>) -> Self {
        let (client_io, server_io) = tokio::io::duplex(1 << 20);
        tokio::spawn(async move {
            let running = server.serve(server_io).await.unwrap();
            running.waiting().await.unwrap();
        });
        let (reader, writer) = tokio::io::split(client_io);
        let mut client = Self {
            writer,
            lines: BufReader::new(reader).lines(),
            next_id: 1,
        };
        let init = client
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {"name": "test-client", "version": "0.0.1"}
                }),
            )
            .await;
        assert_eq!(init["result"]["serverInfo"]["name"], "rusttv-mcp");
        client
            .send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await;
        client
    }

    async fn send(&mut self, message: &Value) {
        let mut line = message.to_string();
        line.push('\n');
        self.writer.write_all(line.as_bytes()).await.unwrap();
    }

    /// Sends a request and returns the response with the same id.
    async fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await;
        loop {
            let line = tokio::time::timeout(Duration::from_secs(10), self.lines.next_line())
                .await
                .expect("server answered in time")
                .unwrap()
                .expect("server closed the pipe");
            let message: Value = serde_json::from_str(&line).unwrap();
            if message["id"] == id {
                return message;
            }
        }
    }

    /// Calls a tool and returns its `structuredContent`, asserting `isError`.
    async fn call(&mut self, tool: &str, arguments: Value, is_error: bool) -> Value {
        let response = self
            .request("tools/call", json!({"name": tool, "arguments": arguments}))
            .await;
        let result = &response["result"];
        assert_eq!(result["isError"], is_error, "{response}");
        // The text content carries the same JSON, for clients that ignore structuredContent.
        let text: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(text, result["structuredContent"]);
        result["structuredContent"].clone()
    }
}

/// `data` of a successful `rusttv.v3` envelope.
fn data(envelope: Value) -> Value {
    assert_eq!(envelope["schema"], "rusttv.v3");
    assert_eq!(envelope["ok"], true, "{envelope}");
    envelope["data"].clone()
}

/// `error.code` of a failed `rusttv.v3` envelope.
fn error_code(envelope: &Value) -> &str {
    assert_eq!(envelope["schema"], "rusttv.v3");
    assert_eq!(envelope["ok"], false, "{envelope}");
    assert!(
        envelope["error"]["message"]
            .as_str()
            .is_some_and(|m| !m.is_empty())
    );
    envelope["error"]["code"].as_str().unwrap()
}

#[tokio::test]
async fn lists_exactly_the_read_only_and_plan_tools() {
    let mut client = Client::start(emulator()).await;
    let response = client.request("tools/list", json!({})).await;
    let tools = response["result"]["tools"].as_array().unwrap();

    let mut names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "apps_list",
            "device_inspect",
            "device_list",
            "launcher_install_plan",
            "launcher_restore_plan",
            "launcher_status",
            "launcher_uninstall_plan",
            "performance_snapshot"
        ]
    );
    for tool in tools {
        assert_eq!(tool["annotations"]["readOnlyHint"], true, "{tool}");
    }
}

#[tokio::test]
async fn tools_return_the_same_data_as_the_cli() {
    let mut client = Client::start(emulator()).await;
    let expected = emulator();

    let devices = data(client.call("device_list", json!({}), false).await);
    assert_eq!(
        devices,
        serde_json::to_value(expected.devices().unwrap()).unwrap()
    );

    let info = data(client.call("device_inspect", json!({}), false).await);
    assert_eq!(
        info,
        serde_json::to_value(inspect(&expected, SERIAL).unwrap()).unwrap()
    );

    let apps = data(
        client
            .call("apps_list", json!({"serial": SERIAL}), false)
            .await,
    );
    assert_eq!(
        apps,
        serde_json::to_value(list_apps(&expected, SERIAL).unwrap()).unwrap()
    );

    let status = data(client.call("launcher_status", json!({}), false).await);
    assert_eq!(
        status,
        serde_json::to_value(launcher_status(&expected, SERIAL).unwrap()).unwrap()
    );
}

#[tokio::test]
async fn performance_snapshot_matches_the_core_and_samples_cpu() {
    let mut client = Client::start(emulator()).await;
    let mut snap = data(
        client
            .call("performance_snapshot", json!({"sample_ms": 100}), false)
            .await,
    );
    assert_eq!(snap["cpu"]["sample_ms"], 100);

    let mut expected =
        serde_json::to_value(snapshot(&emulator(), SERIAL, Duration::from_millis(100)).unwrap())
            .unwrap();
    // The only field that depends on the clock.
    assert!(snap["taken_at_unix_ms"].as_u64().unwrap() > 0);
    snap["taken_at_unix_ms"] = Value::Null;
    expected["taken_at_unix_ms"] = Value::Null;
    assert_eq!(snap, expected);
}

#[tokio::test]
async fn errors_carry_the_cli_error_codes() {
    let mut client = Client::start(FakeAdb::new()).await;
    let none = client.call("device_inspect", json!({}), true).await;
    assert_eq!(error_code(&none), "NO_DEVICE");

    let adb = FakeAdb::new()
        .with_device(device("tv-1", DeviceState::Device))
        .with_device(device("tv-2", DeviceState::Unauthorized));
    let mut client = Client::start(adb).await;

    let several = client.call("apps_list", json!({}), true).await;
    assert_eq!(error_code(&several), "MULTIPLE_DEVICES");

    let unauthorized = client
        .call("launcher_status", json!({"serial": "tv-2"}), true)
        .await;
    assert_eq!(error_code(&unauthorized), "DEVICE_UNAUTHORIZED");

    let missing = client
        .call("device_inspect", json!({"serial": "tv-3"}), true)
        .await;
    assert_eq!(error_code(&missing), "DEVICE_NOT_FOUND");

    // FakeAdb has no canned output for tv-1, like a command failing on the device.
    let failed = client
        .call("device_inspect", json!({"serial": "tv-1"}), true)
        .await;
    assert_eq!(error_code(&failed), "COMMAND_FAILED");

    // `pm` printing an error is a failure, not an empty package list.
    let adb = emulator().respond(
        DeviceCommand::ListDisabledPackages,
        capture!("synthetic-pm-list-errors", "cant-find-service.txt"),
    );
    let mut client = Client::start(adb).await;
    let pm_failed = client.call("apps_list", json!({}), true).await;
    assert_eq!(error_code(&pm_failed), "PACKAGE_LIST_FAILED");
}

#[tokio::test]
async fn invalid_arguments_and_unknown_tools_are_rejected() {
    let mut client = Client::start(emulator()).await;
    for (tool, arguments) in [
        ("performance_snapshot", json!({"sample_ms": 99})),
        ("performance_snapshot", json!({"sample_ms": 60_001})),
        ("device_inspect", json!({"serial": 5})),
        // Unknown arguments are rejected, not ignored.
        (
            "apps_list",
            json!({"serial": SERIAL, "command": "pm disable-user"}),
        ),
        ("device_disable", json!({})),
        ("launcher_takeover", json!({})),
        ("launcher_takeover_plan", json!({})),
        ("launcher_apply", json!({})),
        ("launcher_install_plan", json!({"apk": 5})),
        ("launcher_install_plan", json!({"approve": true})),
        ("launcher_install_plan", json!({"confirmation": "install"})),
        (
            "launcher_uninstall_plan",
            json!({"package": "com.google.android.apps.tv.launcherx"}),
        ),
        ("launcher_restore_plan", json!({"journal": "/etc/passwd"})),
    ] {
        let response = client
            .request("tools/call", json!({"name": tool, "arguments": arguments}))
            .await;
        // A JSON-RPC error, or (for arguments rmcp can't deserialize) a tool
        // error without a `rusttv.v3` envelope: either way nothing ran.
        let rejected = response["error"]["code"].is_i64()
            || (response["result"]["isError"] == true
                && response["result"]["structuredContent"].is_null());
        assert!(rejected, "{tool}: {response}");
    }
}

/// The core functions behind the tools only run R0 (read) commands.
#[test]
fn tools_only_run_read_only_commands() {
    let adb = emulator();
    let _ = inspect(&adb, SERIAL).unwrap();
    let _ = list_apps(&adb, SERIAL).unwrap();
    let _ = launcher_status(&adb, SERIAL).unwrap();
    let _ = snapshot(&adb, SERIAL, Duration::ZERO).unwrap();
    for command in adb.calls() {
        assert_eq!(command.risk(), RiskClass::Read, "{command:?}");
    }
}

// --- Plan tools -------------------------------------------------------------

/// A `FakeAdb` the test can still read after handing it to the server.
#[derive(Clone)]
struct SharedAdb(Arc<Mutex<FakeAdb>>);

impl SharedAdb {
    fn new(adb: FakeAdb) -> Self {
        Self(Arc::new(Mutex::new(adb)))
    }

    fn calls(&self) -> Vec<DeviceCommand> {
        self.0.lock().unwrap().calls()
    }

    /// Every command above R0 that reached the device.
    fn changes(&self) -> Vec<DeviceCommand> {
        self.calls()
            .into_iter()
            .filter(|command| command.risk() > RiskClass::Read)
            .collect()
    }
}

impl Adb for SharedAdb {
    fn run(&self, serial: &str, command: &DeviceCommand) -> Result<String, AdbError> {
        self.0.lock().unwrap().run(serial, command)
    }

    fn devices(&self) -> Result<Vec<DeviceEntry>, AdbError> {
        self.0.lock().unwrap().devices()
    }

    fn connect(&self, address: &DeviceAddress) -> Result<ConnectOutcome, AdbError> {
        self.0.lock().unwrap().connect(address)
    }
}

/// A temp directory with a stand-in APK (not a real one) and `journals/`.
struct Dir(PathBuf);

impl Dir {
    fn new(test: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rusttv-mcp-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::write(dir.join("app-arm64-v8a-release.apk"), b"synthetic").unwrap();
        // Canonical, as the plan resolves it (macOS temp dirs are symlinks).
        Self(dir.canonicalize().unwrap())
    }

    fn apk(&self) -> PathBuf {
        self.0.join("app-arm64-v8a-release.apk")
    }

    fn journals(&self) -> PathBuf {
        self.0.join("journals")
    }

    /// Every file under `journals/` with its contents.
    fn journal_files(&self) -> Vec<(PathBuf, Vec<u8>)> {
        let mut files: Vec<_> = fs::read_dir(self.journals())
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                let bytes = fs::read(&path).unwrap();
                (path, bytes)
            })
            .collect();
        files.sort();
        files
    }

    /// A Home takeover journal for the synthetic Google TV, waiting to be restored.
    fn takeover_journal(&self) -> PathBuf {
        let stock = PackageName::new("com.google.android.apps.tv.launcherx").unwrap();
        let path = self.journals().join("0000000000001-home-takeover.toml");
        FileJournal::new(&path)
            .save(&Journal {
                schema_version: JOURNAL_SCHEMA_VERSION,
                plan: HOME_TAKEOVER.to_owned(),
                serial: SERIAL.to_owned(),
                fingerprint: TV_FINGERPRINT.to_owned(),
                started_at_unix_ms: 1,
                status: JournalStatus::Applied,
                steps: vec![JournalStep {
                    adb: String::new(),
                    command: DeviceCommand::DisablePackage(stock.clone()),
                    original_state: Some(Check::PackageEnabled(stock)),
                    status: StepStatus::Done,
                }],
            })
            .unwrap();
        path
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// `ro.build.fingerprint` in `synthetic-home-takeover/getprop.txt`.
const TV_FINGERPRINT: &str =
    "google/sdk_google_atv64_arm64/emu64a:14/UTT3.240625.001.K5/12345678:userdebug/dev-keys";

/// The Google TV emulator (synthetic install states), with or without the
/// RustTV launcher. Answers only read commands, and the commands that would
/// change it, so a test sees it if one ran.
fn google_tv(dir: &Dir, rusttv_installed: bool) -> FakeAdb {
    let (packages, candidates) = if rusttv_installed {
        (
            capture!("synthetic-launcher-install", "packages-f-rusttv.txt"),
            capture!("google-tv-emulator-api34", "home-candidates.txt"),
        )
    } else {
        (
            capture!("synthetic-launcher-install", "packages-f-no-rusttv.txt"),
            capture!(
                "synthetic-launcher-install",
                "home-candidates-no-rusttv.txt"
            ),
        )
    };
    let apk = rusttv_core::ApkPath::from_local_file(&dir.apk()).unwrap();
    let mut adb = FakeAdb::new()
        .respond(
            DeviceCommand::GetProperties,
            capture!("synthetic-home-takeover", "getprop.txt"),
        )
        .respond(DeviceCommand::ListPackages, packages)
        .respond(DeviceCommand::HomeCandidates, candidates)
        .respond(
            DeviceCommand::HomeActivity,
            capture!("google-tv-emulator-api34", "home-activity.txt"),
        )
        .respond(
            DeviceCommand::ListThirdPartyPackages,
            capture!("synthetic-home-takeover", "packages-3.txt"),
        )
        .respond(
            DeviceCommand::ListDisabledPackages,
            capture!(
                "synthetic-home-takeover",
                "packages-d-launcher-disabled.txt"
            ),
        )
        .respond(
            DeviceCommand::InstallLauncher(apk.clone()),
            capture!("synthetic-launcher-install", "adb-install.txt"),
        )
        .respond(
            DeviceCommand::UpgradeLauncher(apk),
            capture!("synthetic-launcher-install", "adb-install.txt"),
        )
        .respond(
            DeviceCommand::UninstallLauncher,
            capture!("synthetic-launcher-install", "pm-uninstall.txt"),
        );
    for device in parse_device_list(capture!("emulator-pixel3a-api34", "devices-l.txt")) {
        adb = adb.with_device(device);
    }
    adb
}

async fn plan_client(adb: &SharedAdb, dir: &Dir) -> Client {
    Client::serve(RustTvServer::new(adb.clone()).with_journals_dir(dir.journals())).await
}

#[tokio::test]
async fn install_plan_returns_the_cli_dry_run_and_the_command_to_apply_it() {
    let dir = Dir::new("install");
    let adb = SharedAdb::new(google_tv(&dir, false));
    let mut client = plan_client(&adb, &dir).await;
    let apk = dir.apk();
    let apk = apk.to_str().unwrap();

    let proposal = data(
        client
            .call("launcher_install_plan", json!({"apk": apk}), false)
            .await,
    );

    let expected = install_plan(
        &google_tv(&dir, false),
        SERIAL,
        Some(&dir.apk()),
        &dir.journals(),
    )
    .unwrap()
    .dry_run();
    assert_eq!(
        proposal["dry_run"],
        serde_json::to_value(&expected).unwrap()
    );
    assert_eq!(proposal["dry_run"]["plan"], "launcher-install");
    assert_eq!(proposal["dry_run"]["max_risk"], "R3");
    assert_eq!(proposal["dry_run"]["confirmation"], "install");
    assert_eq!(
        proposal["dry_run"]["steps"][0]["command"],
        format!("adb -s {SERIAL} install --pkg ai.rusttv.launcher {apk}")
    );
    assert_eq!(
        proposal["apply_command"],
        format!("rusttv launcher install --serial {SERIAL} --apk {apk}")
    );
    assert_eq!(adb.changes(), [], "planning changed the device");
    assert_eq!(dir.journal_files(), [], "planning wrote a journal");
}

#[tokio::test]
async fn install_plan_on_an_installed_launcher_is_an_upgrade() {
    let dir = Dir::new("upgrade");
    let adb = SharedAdb::new(google_tv(&dir, true));
    let mut client = plan_client(&adb, &dir).await;

    let proposal = data(
        client
            .call(
                "launcher_install_plan",
                json!({"serial": SERIAL, "apk": dir.apk()}),
                false,
            )
            .await,
    );
    let command = proposal["dry_run"]["steps"][0]["command"].as_str().unwrap();
    assert!(
        command.contains(" install -r --pkg ai.rusttv.launcher "),
        "{command}"
    );
    assert_eq!(proposal["dry_run"]["steps"][0]["undo"], Value::Null);
    assert_eq!(adb.changes(), []);
}

#[tokio::test]
async fn uninstall_plan_returns_the_cli_dry_run_and_the_command_to_apply_it() {
    let dir = Dir::new("uninstall");
    let adb = SharedAdb::new(google_tv(&dir, true));
    let mut client = plan_client(&adb, &dir).await;

    let proposal = data(
        client
            .call("launcher_uninstall_plan", json!({}), false)
            .await,
    );

    let expected = uninstall_plan(&google_tv(&dir, true), SERIAL, &dir.journals())
        .unwrap()
        .dry_run();
    assert_eq!(
        proposal["dry_run"],
        serde_json::to_value(&expected).unwrap()
    );
    assert_eq!(
        proposal["dry_run"]["steps"][0]["command"],
        format!("adb -s {SERIAL} shell pm uninstall ai.rusttv.launcher")
    );
    assert_eq!(
        proposal["apply_command"],
        format!("rusttv launcher uninstall --serial {SERIAL}")
    );
    assert_eq!(adb.changes(), []);
    assert_eq!(dir.journal_files(), []);
}

#[tokio::test]
async fn restore_plan_names_the_journal_and_leaves_it_untouched() {
    let dir = Dir::new("restore");
    let journal = dir.takeover_journal();
    let before = dir.journal_files();
    let adb = SharedAdb::new(google_tv(&dir, true));
    let mut client = plan_client(&adb, &dir).await;

    let proposal = data(client.call("launcher_restore_plan", json!({}), false).await);

    let loaded = rusttv_core::load_journal(&journal).unwrap();
    let expected = takeover_restore_plan(&google_tv(&dir, true), SERIAL, &loaded)
        .unwrap()
        .dry_run();
    assert_eq!(
        proposal["dry_run"],
        serde_json::to_value(&expected).unwrap()
    );
    assert_eq!(proposal["dry_run"]["plan"], "home-takeover-restore");
    assert_eq!(
        proposal["apply_command"],
        format!(
            "rusttv launcher restore --serial {SERIAL} --journal {}",
            journal.display()
        )
    );
    assert_eq!(adb.changes(), []);
    assert_eq!(dir.journal_files(), before, "planning changed the journal");
}

#[tokio::test]
async fn plan_tools_fail_with_the_cli_error_codes() {
    let dir = Dir::new("errors");
    let adb = SharedAdb::new(google_tv(&dir, false));
    let mut client = plan_client(&adb, &dir).await;

    // Uninstalling a launcher that isn't installed.
    let not_installed = client
        .call("launcher_uninstall_plan", json!({}), true)
        .await;
    assert_eq!(error_code(&not_installed), "PRECONDITION_FAILED");

    // No takeover journal to restore.
    let no_journal = client.call("launcher_restore_plan", json!({}), true).await;
    assert_eq!(error_code(&no_journal), "NO_JOURNAL");

    // No APK given and none built relative to the server's directory.
    let no_apk = client.call("launcher_install_plan", json!({}), true).await;
    assert_eq!(error_code(&no_apk), "INVALID_APK");

    // A well-formed path to a file that doesn't exist.
    let missing = client
        .call(
            "launcher_install_plan",
            json!({"apk": dir.0.join("missing.apk")}),
            true,
        )
        .await;
    assert_eq!(error_code(&missing), "INVALID_APK");

    // A well-formed path to a directory.
    fs::create_dir(dir.0.join("folder.apk")).unwrap();
    let folder = client
        .call(
            "launcher_install_plan",
            json!({"apk": dir.0.join("folder.apk")}),
            true,
        )
        .await;
    assert_eq!(error_code(&folder), "INVALID_APK");

    let several = SharedAdb::new(
        FakeAdb::new()
            .with_device(device("tv-1", DeviceState::Device))
            .with_device(device("tv-2", DeviceState::Device)),
    );
    let mut client = plan_client(&several, &dir).await;
    let ambiguous = client
        .call("launcher_uninstall_plan", json!({}), true)
        .await;
    assert_eq!(error_code(&ambiguous), "MULTIPLE_DEVICES");

    assert_eq!(adb.changes(), []);
    assert_eq!(several.calls(), []);
}

#[tokio::test]
async fn apk_paths_are_checked_before_anything_runs() {
    let dir = Dir::new("apk-shape");
    let adb = SharedAdb::new(google_tv(&dir, false));
    let mut client = plan_client(&adb, &dir).await;

    for apk in [
        "app-arm64-v8a-release.apk".to_owned(),
        "../app.apk".to_owned(),
        "/etc/passwd".to_owned(),
        "/tmp/app.apk; rm -rf ~".to_owned(),
        "/tmp/-r.apk".to_owned(),
        "/tmp/$(reboot).apk".to_owned(),
        "/tmp/app\n.apk".to_owned(),
    ] {
        let rejected = client
            .call("launcher_install_plan", json!({"apk": apk}), true)
            .await;
        assert_eq!(error_code(&rejected), "INVALID_APK", "{apk:?}");
    }
    // The shape check fails before the device is contacted.
    assert_eq!(adb.calls(), [], "a rejected APK reached adb");
}

/// The server never approves or applies: the only way to an `Approval` is
/// `Plan::approve`, and `apply`/`restore` need one.
#[test]
fn server_never_approves_or_applies_a_plan() {
    let source = include_str!("../src/lib.rs");
    for forbidden in [
        "approve(",
        "Approval",
        "rusttv_core::apply",
        "rusttv_core::restore(",
        "FileJournal",
        "JournalStore",
        "SystemAdb",
    ] {
        assert!(
            !source.contains(forbidden),
            "src/lib.rs mentions `{forbidden}`"
        );
    }
}
