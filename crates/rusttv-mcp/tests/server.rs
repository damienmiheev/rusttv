//! Drives the MCP server over an in-memory stdio pipe with `FakeAdb`, speaking
//! raw newline-delimited JSON-RPC like an MCP client does.

#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test helpers should panic on unexpected failures; clippy only exempts #[test] fns"
)]

use std::time::Duration;

use rmcp::ServiceExt;
use rusttv_core::{
    DeviceCommand, DeviceEntry, DeviceState, FakeAdb, PackageName, RiskClass, inspect,
    launcher_status, list_apps, parse_device_list, snapshot,
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
        let (client_io, server_io) = tokio::io::duplex(1 << 20);
        tokio::spawn(async move {
            let running = RustTvServer::new(adb).serve(server_io).await.unwrap();
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

/// `data` of a successful `rusttv.v1` envelope.
fn data(envelope: Value) -> Value {
    assert_eq!(envelope["schema"], "rusttv.v1");
    assert_eq!(envelope["ok"], true, "{envelope}");
    envelope["data"].clone()
}

/// `error.code` of a failed `rusttv.v1` envelope.
fn error_code(envelope: &Value) -> &str {
    assert_eq!(envelope["schema"], "rusttv.v1");
    assert_eq!(envelope["ok"], false, "{envelope}");
    assert!(
        envelope["error"]["message"]
            .as_str()
            .is_some_and(|m| !m.is_empty())
    );
    envelope["error"]["code"].as_str().unwrap()
}

#[tokio::test]
async fn lists_exactly_the_read_only_tools() {
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
            "launcher_status",
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
    ] {
        let response = client
            .request("tools/call", json!({"name": tool, "arguments": arguments}))
            .await;
        // A JSON-RPC error, or (for arguments rmcp can't deserialize) a tool
        // error without a `rusttv.v1` envelope: either way nothing ran.
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
