//! Read-only MCP server for RustTV.
//!
//! Exposes the core's read-only functions as MCP tools. Every tool result is
//! the CLI's `rusttv.v1` JSON envelope (CLI.md § Output), so `data` is exactly
//! what `rusttv <command> --json` prints and errors carry the same stable
//! codes. There is deliberately no tool that changes a device, and none that
//! can approve a plan (SECURITY.md § AI agents). See MCP.md.
//!
//! The core is synchronous (ARCHITECTURE.md D10): each tool call runs it on
//! tokio's blocking pool, one call at a time.

use std::error::Error;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ErrorData, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ServerHandler, schemars, tool, tool_handler, tool_router};
use rusttv_core::{Adb, AdbError, DeviceSelectionError};
use serde::{Deserialize, Serialize};

/// Version of the JSON output contract; the same as the CLI's `--json`.
pub const SCHEMA: &str = "rusttv.v1";

/// Default and allowed range of `performance_snapshot`'s `sample_ms`, as in the CLI.
const DEFAULT_SAMPLE_MS: u64 = 1000;
const SAMPLE_MS_RANGE: std::ops::RangeInclusive<u64> = 100..=60_000;

const INSTRUCTIONS: &str = "Read-only tools for Android TVs connected through adb. \
Results are the `rusttv.v1` envelope of the `rusttv` CLI: on success `data` is the \
same as `rusttv <command> --json`; on failure `error.code` is a stable code from \
CLI.md. Device-targeting tools take an optional `serial`; it is required when several \
devices are connected (`MULTIPLE_DEVICES`). Nothing here changes a device or approves \
a change: changes are planned and approved by a person with the `rusttv` CLI.";

/// Arguments of tools that target one device.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct TargetParams {
    /// adb serial of the device (see `device_list`); needed when several are connected.
    #[serde(default)]
    pub serial: Option<String>,
}

/// Arguments of `performance_snapshot`.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct SnapshotParams {
    /// adb serial of the device (see `device_list`); needed when several are connected.
    #[serde(default)]
    pub serial: Option<String>,
    /// How long to sample CPU usage, in milliseconds (default 1000).
    #[serde(default)]
    #[schemars(range(min = 100, max = 60_000))]
    pub sample_ms: Option<u64>,
}

/// Everything a tool can fail with; codes are the CLI's (CLI.md § Error codes).
#[derive(Debug, thiserror::Error)]
enum ToolError {
    #[error(transparent)]
    Adb(#[from] AdbError),
    #[error(transparent)]
    Selection(#[from] DeviceSelectionError),
}

impl ToolError {
    fn code(&self) -> &'static str {
        match self {
            Self::Adb(error) => error.code(),
            Self::Selection(error) => error.code(),
        }
    }
}

#[derive(Debug, Serialize)]
struct Success<T> {
    schema: &'static str,
    ok: bool,
    data: T,
}

#[derive(Debug, Serialize)]
struct Failure {
    schema: &'static str,
    ok: bool,
    error: ErrorBody,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

/// The MCP server. Generic over [`Adb`] so tests can use `FakeAdb`.
#[derive(Debug)]
pub struct RustTvServer<A> {
    /// Locked for the whole tool call: calls run one at a time, and `A` need
    /// not be `Sync` (`FakeAdb` isn't).
    adb: Arc<Mutex<A>>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl<A: Adb + Send + 'static> RustTvServer<A> {
    /// Creates a server that reaches devices through `adb`.
    pub fn new(adb: A) -> Self {
        Self {
            adb: Arc::new(Mutex::new(adb)),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "List devices known to the local adb server, with their state. Same as `rusttv device list --json`.",
        annotations(read_only_hint = true)
    )]
    async fn device_list(&self) -> Result<CallToolResult, ErrorData> {
        self.run(|adb| Ok(adb.devices()?)).await
    }

    #[tool(
        description = "Manufacturer, model, Android version, build, CPU ABIs, memory and storage of a device. Same as `rusttv device inspect --json`.",
        annotations(read_only_hint = true)
    )]
    async fn device_inspect(
        &self,
        Parameters(params): Parameters<TargetParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(move |adb| {
            let serial = resolve(adb, params.serial.as_deref())?;
            Ok(rusttv_core::inspect(adb, &serial)?)
        })
        .await
    }

    #[tool(
        description = "Installed packages: system or third-party, enabled or disabled. Same as `rusttv apps list --json`.",
        annotations(read_only_hint = true)
    )]
    async fn apps_list(
        &self,
        Parameters(params): Parameters<TargetParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(move |adb| {
            let serial = resolve(adb, params.serial.as_deref())?;
            Ok(rusttv_core::list_apps(adb, &serial)?)
        })
        .await
    }

    #[tool(
        description = "Which activity the Home button opens, every Home candidate with its priority, and whether the RustTV launcher is installed / is Home. Same as `rusttv launcher status --json`.",
        annotations(read_only_hint = true)
    )]
    async fn launcher_status(
        &self,
        Parameters(params): Parameters<TargetParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(move |adb| {
            let serial = resolve(adb, params.serial.as_deref())?;
            Ok(rusttv_core::launcher_status(adb, &serial)?)
        })
        .await
    }

    #[tool(
        description = "CPU usage over `sample_ms`, memory, launcher memory and the biggest processes right now. Same as `rusttv performance snapshot --json`.",
        annotations(read_only_hint = true)
    )]
    async fn performance_snapshot(
        &self,
        Parameters(params): Parameters<SnapshotParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let sample_ms = params.sample_ms.unwrap_or(DEFAULT_SAMPLE_MS);
        if !SAMPLE_MS_RANGE.contains(&sample_ms) {
            return Err(ErrorData::invalid_params(
                format!(
                    "sample_ms must be between {} and {}",
                    SAMPLE_MS_RANGE.start(),
                    SAMPLE_MS_RANGE.end()
                ),
                None,
            ));
        }
        self.run(move |adb| {
            let serial = resolve(adb, params.serial.as_deref())?;
            Ok(rusttv_core::snapshot(
                adb,
                &serial,
                Duration::from_millis(sample_ms),
            )?)
        })
        .await
    }

    /// Runs a blocking core call and wraps its outcome in the `rusttv.v1` envelope.
    ///
    /// Core failures are tool results with `isError` (the agent sees the code);
    /// only a crashed task or unserializable data is a protocol error.
    async fn run<T, F>(&self, call: F) -> Result<CallToolResult, ErrorData>
    where
        T: Serialize,
        F: FnOnce(&dyn Adb) -> Result<T, ToolError> + Send + 'static,
        T: Send + 'static,
    {
        let adb = Arc::clone(&self.adb);
        let outcome = tokio::task::spawn_blocking(move || {
            // A poisoned lock only means an earlier call panicked; the adb
            // handle itself holds no state that a panic could leave broken.
            let adb = adb.lock().unwrap_or_else(PoisonError::into_inner);
            call(&*adb)
        })
        .await
        .map_err(|error| ErrorData::internal_error(format!("tool task failed: {error}"), None))?;

        let envelope = match outcome {
            Ok(data) => serde_json::to_value(Success {
                schema: SCHEMA,
                ok: true,
                data,
            }),
            Err(error) => {
                return Ok(CallToolResult::structured_error(failure_value(&error)));
            }
        };
        envelope.map(CallToolResult::structured).map_err(|error| {
            ErrorData::internal_error(format!("serializing result: {error}"), None)
        })
    }
}

#[tool_handler(router = self.tool_router)]
impl<A: Adb + Send + 'static> ServerHandler for RustTvServer<A> {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("rusttv-mcp", rusttv_core::VERSION))
            .with_instructions(INSTRUCTIONS)
    }
}

/// Resolves to the serial of a single ready device, like the CLI's `--serial`.
fn resolve(adb: &dyn Adb, serial: Option<&str>) -> Result<String, ToolError> {
    let devices = adb.devices()?;
    let device = rusttv_core::select_device(&devices, serial)?;
    Ok(device.serial.clone())
}

fn failure_value(error: &ToolError) -> serde_json::Value {
    let failure = Failure {
        schema: SCHEMA,
        ok: false,
        error: ErrorBody {
            code: error.code(),
            message: message_with_causes(error),
        },
    };
    // Only strings and bools: serializing cannot fail.
    serde_json::to_value(failure).unwrap_or_default()
}

/// Joins an error with its chain of causes: "outer: inner: root", as the CLI does.
fn message_with_causes(error: &dyn Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}
