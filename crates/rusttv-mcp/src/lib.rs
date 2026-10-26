//! Read-only MCP server for RustTV.
//!
//! Exposes the core's read-only functions as MCP tools. Every tool result is
//! the CLI's `rusttv.v10` JSON envelope (CLI.md § Output), so `data` is exactly
//! what `rusttv <command> --json` prints and errors carry the same stable
//! codes. The `*_plan` tools build a change plan and return its dry run with
//! the `rusttv` command a person runs to apply it. There is deliberately no
//! tool that changes a device, and none that can approve a plan (SECURITY.md
//! § AI agents). See MCP.md.
//!
//! The core is synchronous (ARCHITECTURE.md D10): each tool call runs it on
//! tokio's blocking pool, one call at a time.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, ErrorData, Implementation, ServerCapabilities, ServerConfig,
};
use rmcp::{ServerHandler, schemars, tool, tool_handler, tool_router};
use rusttv_core::{
    Adb, AdbError, ApkPath, DeviceCommand, DeviceProfile, DeviceSelectionError, DryRun,
    HOME_TAKEOVER, LoadedChoices, OptimizeKind, Plan, PlanError, PrivacyCategory, PrivacyData,
};
use serde::{Deserialize, Serialize};

/// Version of the JSON output contract; the same as the CLI's `--json`.
pub const SCHEMA: &str = "rusttv.v10";

/// Default and allowed range of `performance_snapshot`'s `sample_ms`, as in the CLI.
const DEFAULT_SAMPLE_MS: u64 = 1000;
const SAMPLE_MS_RANGE: std::ops::RangeInclusive<u64> = 100..=60_000;

const INSTRUCTIONS: &str = "Read-only tools for Android TVs connected through adb. \
Results are the `rusttv.v10` envelope of the `rusttv` CLI: on success `data` is the \
same as `rusttv <command> --json`; on failure `error.code` is a stable code from \
CLI.md. Device-targeting tools take an optional `serial`; it is required when several \
devices are connected (`MULTIPLE_DEVICES`). Nothing here changes a device or approves \
a change. The `*_plan` tools only plan one: they return the dry run (`data.dry_run`) \
and the `rusttv` command (`data.apply_command`) that a person must run at an \
interactive terminal, where they read the plan and type its confirmation word. Show \
both to the user; do not run the command yourself.";

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

/// Arguments of `launcher_install_plan`.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct InstallPlanParams {
    /// adb serial of the device (see `device_list`); needed when several are connected.
    #[serde(default)]
    pub serial: Option<String>,
    /// Absolute path of the launcher APK on this computer, file name of ASCII letters, digits, `.`, `_`, `-` ending in `.apk`. Default: the APK `android/build.sh` builds for the device's primary ABI, relative to the server's working directory.
    #[serde(default)]
    pub apk: Option<String>,
}

/// Arguments of `launcher_restore_plan`.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct RestorePlanParams {
    /// adb serial of the device (see `device_list`); needed when several are connected.
    #[serde(default)]
    pub serial: Option<String>,
    /// Plan from the device's compatibility profile instead of a takeover journal, for when the journal is lost: re-enable the packages a takeover disables that are disabled now (the CLI's `--from-profile`). Default false.
    #[serde(default)]
    pub from_profile: bool,
}

/// Arguments of `privacy_plan`.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct PrivacyPlanParams {
    /// adb serial of the device (see `device_list`); needed when several are connected.
    #[serde(default)]
    pub serial: Option<String>,
    /// Narrow the categories of the person's choices file to these (the CLI's `--only`); it never adds a category. Default: the choices' categories.
    #[serde(default)]
    pub only: Vec<Category>,
    /// Also plan packages not tested on this model, each marked UNTESTED in the dry run (the CLI's `--allow-untested`). Default false.
    #[serde(default)]
    pub allow_untested: bool,
}

/// Arguments of `optimize_plan`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct OptimizePlanParams {
    /// adb serial of the device (see `device_list`); needed when several are connected.
    #[serde(default)]
    pub serial: Option<String>,
    /// The bundled optimizer profile to plan.
    pub profile: OptimizerProfileName,
    /// Plan it although this model's compatibility profile doesn't record it as tested, marked UNTESTED in the dry run (the CLI's `--allow-untested`). Default false.
    #[serde(default)]
    pub allow_untested: bool,
}

/// A bundled optimizer profile; a closed list, so no setting can be named.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "kebab-case")]
pub enum OptimizerProfileName {
    /// Window, transition and animator animation scales at 0.5.
    ReducedAnimations,
}

impl OptimizerProfileName {
    fn as_str(self) -> &'static str {
        match self {
            Self::ReducedAnimations => rusttv_core::REDUCED_ANIMATIONS,
        }
    }
}

/// Arguments of `optimize_restore_plan`.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct OptimizeRestorePlanParams {
    /// adb serial of the device (see `device_list`); needed when several are connected.
    #[serde(default)]
    pub serial: Option<String>,
    /// Which change to undo; needed only when both kinds can be restored on the device. Default: the only kind that can be.
    #[serde(default)]
    pub profile: Option<RestoreKind>,
}

/// What `optimize_restore_plan` undoes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "kebab-case")]
pub enum RestoreKind {
    /// The newest restorable privacy journal.
    Privacy,
    /// The newest restorable `reduced-animations` journal.
    ReducedAnimations,
}

impl From<RestoreKind> for OptimizeKind {
    fn from(kind: RestoreKind) -> Self {
        match kind {
            RestoreKind::Privacy => Self::Privacy,
            RestoreKind::ReducedAnimations => Self::ReducedAnimations,
        }
    }
}

/// A catalog category; a closed list, so no package can be named.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    /// Packages that collect usage data and send it off the TV (default).
    Telemetry,
    /// Packages that serve or personalise ads (default).
    Ads,
    /// Packages that run for nothing on a TV (default).
    Extras,
    /// Casting and screen mirroring (opt-in).
    Casting,
    /// Control by home-automation systems and phone apps (opt-in).
    SmartHome,
    /// Voice assistants other than Google's own (opt-in).
    Voice,
    /// Preinstalled streaming apps (opt-in).
    StreamingApps,
}

impl From<Category> for PrivacyCategory {
    fn from(category: Category) -> Self {
        match category {
            Category::Telemetry => Self::Telemetry,
            Category::Ads => Self::Ads,
            Category::Extras => Self::Extras,
            Category::Casting => Self::Casting,
            Category::SmartHome => Self::SmartHome,
            Category::Voice => Self::Voice,
            Category::StreamingApps => Self::StreamingApps,
        }
    }
}

/// `data` of the `*_plan` tools: the plan's dry run and how a person applies it.
#[derive(Debug, Serialize)]
struct PlanProposal {
    /// Same as `rusttv launcher <command> --dry-run --json`.
    dry_run: DryRun,
    /// The `rusttv` command line a person runs at a terminal to apply this plan.
    apply_command: String,
}

/// Everything a tool can fail with; codes are the CLI's (CLI.md § Error codes).
#[derive(Debug, thiserror::Error)]
enum ToolError {
    #[error(transparent)]
    Adb(#[from] AdbError),
    #[error(transparent)]
    Selection(#[from] DeviceSelectionError),
    #[error(transparent)]
    Plan(#[from] PlanError),
}

impl ToolError {
    fn code(&self) -> &'static str {
        match self {
            Self::Adb(error) => error.code(),
            Self::Selection(error) => error.code(),
            Self::Plan(error) => error.code(),
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
    /// Journal directory the plan tools read; `None`: the CLI's, [`rusttv_core::journals_dir`].
    journals: Option<PathBuf>,
    /// Compat profiles the optimizer tools use; `None`: the bundled ones, as the CLI.
    profiles: Option<Vec<DeviceProfile>>,
    /// Privacy choices the optimizer tools use; `None`: the CLI's choices
    /// file (`<config dir>/privacy.toml`), read on every call.
    choices: Option<LoadedChoices>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl<A: Adb + Send + 'static> RustTvServer<A> {
    /// Creates a server that reaches devices through `adb`.
    pub fn new(adb: A) -> Self {
        Self {
            adb: Arc::new(Mutex::new(adb)),
            journals: None,
            profiles: None,
            choices: None,
            tool_router: Self::tool_router(),
        }
    }

    /// Reads journals from `dir` instead of the CLI's state directory (for tests).
    #[must_use]
    pub fn with_journals_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.journals = Some(dir.into());
        self
    }

    /// Uses `profiles` instead of the bundled compat profiles (for tests).
    #[must_use]
    pub fn with_profiles(mut self, profiles: Vec<DeviceProfile>) -> Self {
        self.profiles = Some(profiles);
        self
    }

    /// Uses `choices` instead of reading the choices file (for tests).
    #[must_use]
    pub fn with_choices(mut self, choices: LoadedChoices) -> Self {
        self.choices = Some(choices);
        self
    }

    /// The catalog, profiles and choices, as the CLI reads them.
    fn privacy_data(&self) -> impl FnOnce() -> Result<PrivacyData, PlanError> + Send + 'static {
        let profiles = self.profiles.clone();
        let choices = self.choices.clone();
        move || {
            let choices = match choices {
                Some(choices) => choices,
                None => rusttv_core::load_choices(None)?,
            };
            let mut data = PrivacyData::bundled(choices);
            if let Some(profiles) = profiles {
                data.profiles = profiles;
            }
            Ok(data)
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

    #[tool(
        description = "Plan installing (or upgrading) the RustTV launcher, ai.rusttv.launcher. Changes nothing: returns the plan's dry run (`data.dry_run`, same as `rusttv launcher install --dry-run --json`) and the command a person runs at a terminal to apply it (`data.apply_command`). Show both to the user; you cannot apply or approve it.",
        annotations(read_only_hint = true)
    )]
    async fn launcher_install_plan(
        &self,
        Parameters(params): Parameters<InstallPlanParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let journals = self.journals.clone();
        self.run(move |adb| {
            // Shape first (absolute, plain `.apk` file name), so an agent can't
            // probe arbitrary files; the core then checks the file itself.
            let apk = params
                .apk
                .as_deref()
                .map(ApkPath::new)
                .transpose()
                .map_err(PlanError::from)?;
            let serial = resolve(adb, params.serial.as_deref())?;
            let plan = rusttv_core::install_plan(
                adb,
                &serial,
                apk.as_ref().map(|apk| Path::new(apk.as_str())),
                &journals_dir(journals)?,
            )?;
            // The APK the plan resolved, so the person applies this same file.
            let planned_apk = plan.steps().iter().find_map(|step| match step.command() {
                DeviceCommand::InstallLauncher(apk) | DeviceCommand::UpgradeLauncher(apk) => {
                    Some(apk.as_str())
                }
                _ => None,
            });
            let mut args = vec!["launcher", "install", "--serial", plan.serial()];
            if let Some(apk) = planned_apk {
                args.extend(["--apk", apk]);
            }
            let apply_command = command_line(&args);
            Ok(proposal(&plan, apply_command))
        })
        .await
    }

    #[tool(
        description = "Plan uninstalling the RustTV launcher, ai.rusttv.launcher (no other package). Changes nothing: returns the plan's dry run (`data.dry_run`, same as `rusttv launcher uninstall --dry-run --json`) and the command a person runs at a terminal to apply it (`data.apply_command`). Show both to the user; you cannot apply or approve it.",
        annotations(read_only_hint = true)
    )]
    async fn launcher_uninstall_plan(
        &self,
        Parameters(params): Parameters<TargetParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let journals = self.journals.clone();
        self.run(move |adb| {
            let serial = resolve(adb, params.serial.as_deref())?;
            let plan = rusttv_core::uninstall_plan(adb, &serial, &journals_dir(journals)?)?;
            let apply_command = command_line(&["launcher", "uninstall", "--serial", plan.serial()]);
            Ok(proposal(&plan, apply_command))
        })
        .await
    }

    #[tool(
        description = "Plan undoing a Home takeover (re-enabling the stock launcher) from the newest takeover journal for the device's firmware, or with `from_profile` from the device's compatibility profile when the journal is lost. Changes nothing: returns the plan's dry run (`data.dry_run`, same as `rusttv launcher restore --dry-run --json`) and the command a person runs at a terminal to apply it (`data.apply_command`). Show both to the user; you cannot apply or approve it. Journal files that can't be read are reported in a second text content item, or in the `NO_JOURNAL` message.",
        annotations(read_only_hint = true)
    )]
    async fn launcher_restore_plan(
        &self,
        Parameters(params): Parameters<RestorePlanParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let journals = self.journals.clone();
        self.run_with_notes(move |adb| {
            let serial = resolve(adb, params.serial.as_deref())?;
            if params.from_profile {
                let journal = rusttv_core::takeover_journal_from_profile(
                    adb,
                    &serial,
                    &rusttv_core::bundled_profiles(),
                )?;
                let plan = rusttv_core::takeover_restore_plan(adb, &serial, &journal)?;
                let apply_command = command_line(&[
                    "launcher",
                    "restore",
                    "--serial",
                    plan.serial(),
                    "--from-profile",
                ]);
                return Ok((proposal(&plan, apply_command), Vec::new()));
            }
            let fingerprint = rusttv_core::device_fingerprint(adb, &serial)?;
            let search = rusttv_core::find_restorable_journal(
                &journals_dir(journals)?,
                HOME_TAKEOVER,
                &fingerprint,
            );
            // As the CLI's warnings: any of them may still need restoring.
            let notes: Vec<String> = search
                .unreadable
                .iter()
                .map(|file| {
                    format!(
                        "warning: skipped {}, which can't be read. If it is a takeover that \
                         still needs restoring, it must be fixed and passed as `--journal`, or \
                         planned with `from_profile`. Reason: {}",
                        file.path.display(),
                        file.error
                    )
                })
                .collect();
            let (path, journal) = search.found.map_err(|error| {
                if notes.is_empty() || error.code() != "NO_JOURNAL" {
                    error
                } else {
                    PlanError::no_journal(format!("{error}; {}", notes.join("; ")))
                }
            })?;
            let plan = rusttv_core::takeover_restore_plan(adb, &serial, &journal)?;
            let journal = path.to_str().ok_or_else(|| {
                PlanError::journal(format!("{} is not valid UTF-8", path.display()))
            })?;
            let apply_command = command_line(&[
                "launcher",
                "restore",
                "--serial",
                plan.serial(),
                "--journal",
                journal,
            ]);
            Ok((proposal(&plan, apply_command), notes))
        })
        .await
    }

    #[tool(
        description = "Every package of RustTV's catalog installed on the device, and every `extra` of the person's choices file, with its category (telemetry, ads, extras are on by default; casting, smart-home, voice, streaming-apps are opt-in), what it does, side effects, whether it was tested on this model, whether the choices select it, whether it is enabled / disabled, and whether RustTV refuses to touch it (protected). Also every optimizer profile (`optimizer`: allowlisted settings such as the animation scales) with its status on the device (suggested, available, applied, drifted, stale, not_tested, already_set, unknown) and the current value of each of its settings (null `current` with a `read_error` when a setting can't be read; that never fails the analysis). Same as `rusttv optimize analyze --json`.",
        annotations(read_only_hint = true)
    )]
    async fn optimize_analyze(
        &self,
        Parameters(params): Parameters<TargetParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let data = self.privacy_data();
        let journals = self.journals.clone();
        self.run(move |adb| {
            let data = data()?;
            let serial = resolve(adb, params.serial.as_deref())?;
            Ok(rusttv_core::optimize_analyze(
                adb,
                &serial,
                &data,
                &journals_dir(journals)?,
            )?)
        })
        .await
    }

    #[tool(
        description = "Plan applying an optimizer profile: `reduced-animations` puts the window, transition and animator animation scales (global settings) at 0.5. Refused with `OPTIMIZE_NOT_TESTED` unless this model's compatibility profile records it as tested, or `allow_untested` (the dry run then says UNTESTED); refused while an optimizer change is still applied (`PRECONDITION_FAILED`). Changes nothing: returns the plan's dry run (`data.dry_run`, same as `rusttv optimize apply <profile> --dry-run --json`) and the command a person runs at a terminal to apply it (`data.apply_command`). Show both to the user; you cannot apply or approve it, and you cannot name settings or values.",
        annotations(read_only_hint = true)
    )]
    async fn optimize_plan(
        &self,
        Parameters(params): Parameters<OptimizePlanParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let profiles = self.profiles.clone();
        let journals = self.journals.clone();
        self.run(move |adb| {
            let serial = resolve(adb, params.serial.as_deref())?;
            let name = params.profile.as_str();
            let profile = rusttv_core::bundled_optimizer_profile(name)?;
            let plan = rusttv_core::optimizer_plan(
                adb,
                &serial,
                &profile,
                &profiles.unwrap_or_else(rusttv_core::bundled_profiles),
                &journals_dir(journals)?,
                params.allow_untested,
            )?;
            let mut args = vec!["optimize", "apply", name, "--serial", plan.serial()];
            if params.allow_untested {
                args.push("--allow-untested");
            }
            let apply_command = command_line(&args);
            Ok(proposal(&plan, apply_command))
        })
        .await
    }

    #[tool(
        description = "Plan undoing an `optimize apply` from its journal: re-enabling the packages a privacy journal disabled, or putting back the settings an optimizer journal changed (settings changed since are left as they are, and named in the notes). `profile` (`privacy` or `reduced-animations`) is needed only when both kinds can be restored. Changes nothing: returns the plan's dry run (`data.dry_run`, same as `rusttv optimize restore [<profile>] --dry-run --json`) and the command a person runs at a terminal to apply it (`data.apply_command`). Show both to the user; you cannot apply or approve it. Journal files that can't be read are reported in a second text content item, or in the `NO_JOURNAL` message.",
        annotations(read_only_hint = true)
    )]
    async fn optimize_restore_plan(
        &self,
        Parameters(params): Parameters<OptimizeRestorePlanParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let journals = self.journals.clone();
        self.run_with_notes(move |adb| {
            let serial = resolve(adb, params.serial.as_deref())?;
            let fingerprint = rusttv_core::device_fingerprint(adb, &serial)?;
            let search = rusttv_core::find_optimize_journal(
                &journals_dir(journals)?,
                &fingerprint,
                params.profile.map(OptimizeKind::from),
            );
            // As the CLI's warnings: any of them may still need restoring.
            let notes: Vec<String> = search
                .unreadable
                .iter()
                .map(|file| {
                    format!(
                        "warning: skipped {}, which can't be read. If it is a privacy or \
                         optimizer plan that still needs restoring, it must be fixed and passed \
                         as `--journal`. Reason: {}",
                        file.path.display(),
                        file.error
                    )
                })
                .collect();
            let (kind, path, mut journal) = search.found.map_err(|error| {
                if notes.is_empty() || error.code() != "NO_JOURNAL" {
                    error
                } else {
                    PlanError::no_journal(format!("{error}; {}", notes.join("; ")))
                }
            })?;
            // The journal is changed in memory only (steps to skip); this
            // tool never writes it.
            let plan = rusttv_core::optimize_restore_plan(adb, &serial, kind, &mut journal)?;
            let journal = path.to_str().ok_or_else(|| {
                PlanError::journal(format!("{} is not valid UTF-8", path.display()))
            })?;
            let apply_command = command_line(&[
                "optimize",
                "restore",
                kind.as_str(),
                "--serial",
                plan.serial(),
                "--journal",
                journal,
            ]);
            Ok((proposal(&plan, apply_command), notes))
        })
        .await
    }

    #[tool(
        description = "Plan disabling (never uninstalling) the catalog packages the person's choices file selects (by default telemetry, ads and extras) that are enabled on the device; `only` narrows the categories. Packages not tested on this model are left out unless `allow_untested`, then each is marked UNTESTED in the notes; with nothing tested or allowed left, `PROFILE_NOT_TESTED`. Refused if a chosen package is protected (`PACKAGE_PROTECTED`). Changes nothing: returns the plan's dry run (`data.dry_run`, same as `rusttv optimize apply privacy --dry-run --json`) and the command a person runs at a terminal to apply it (`data.apply_command`). Show both to the user; you cannot apply or approve it, and you cannot name packages: the person edits their choices file.",
        annotations(read_only_hint = true)
    )]
    async fn privacy_plan(
        &self,
        Parameters(params): Parameters<PrivacyPlanParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let data = self.privacy_data();
        self.run(move |adb| {
            let data = data()?;
            let serial = resolve(adb, params.serial.as_deref())?;
            let only: Vec<PrivacyCategory> = params.only.into_iter().map(Into::into).collect();
            let plan =
                rusttv_core::privacy_plan(adb, &serial, &data, &only, params.allow_untested)?;
            let only = only
                .iter()
                .map(|category| category.as_str())
                .collect::<Vec<_>>()
                .join(",");
            let mut args = vec!["optimize", "apply", "privacy", "--serial", plan.serial()];
            if !only.is_empty() {
                args.extend(["--only", &only]);
            }
            if params.allow_untested {
                args.push("--allow-untested");
            }
            let apply_command = command_line(&args);
            Ok(proposal(&plan, apply_command))
        })
        .await
    }

    /// Runs a blocking core call and wraps its outcome in the `rusttv.v10` envelope.
    ///
    /// Core failures are tool results with `isError` (the agent sees the code);
    /// only a crashed task or unserializable data is a protocol error.
    async fn run<T, F>(&self, call: F) -> Result<CallToolResult, ErrorData>
    where
        T: Serialize,
        F: FnOnce(&dyn Adb) -> Result<T, ToolError> + Send + 'static,
        T: Send + 'static,
    {
        self.run_with_notes(move |adb| call(adb).map(|data| (data, Vec::new())))
            .await
    }

    /// [`Self::run`] for a call that may also return notes for the agent. Each
    /// note is one more text content item after the envelope, which stays the
    /// same as the CLI's.
    async fn run_with_notes<T, F>(&self, call: F) -> Result<CallToolResult, ErrorData>
    where
        T: Serialize,
        F: FnOnce(&dyn Adb) -> Result<(T, Vec<String>), ToolError> + Send + 'static,
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

        let (data, notes) = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                return Ok(CallToolResult::structured_error(failure_value(&error)));
            }
        };
        let envelope = serde_json::to_value(Success {
            schema: SCHEMA,
            ok: true,
            data,
        })
        .map_err(|error| ErrorData::internal_error(format!("serializing result: {error}"), None))?;
        let mut result = CallToolResult::structured(envelope);
        result
            .content
            .extend(notes.into_iter().map(ContentBlock::text));
        Ok(result)
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

/// The journal directory: the one the server was given, else the CLI's.
fn journals_dir(dir: Option<PathBuf>) -> Result<PathBuf, PlanError> {
    dir.map_or_else(rusttv_core::journals_dir, Ok)
}

fn proposal(plan: &Plan, apply_command: String) -> PlanProposal {
    PlanProposal {
        dry_run: plan.dry_run(),
        apply_command,
    }
}

/// `rusttv <args>` as a POSIX shell command line, each argument quoted when
/// needed: the person may paste it, and paths can contain spaces or `$`.
fn command_line(args: &[&str]) -> String {
    let mut line = String::from("rusttv");
    for arg in args {
        line.push(' ');
        let plain = !arg.is_empty()
            && arg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_./:@%+=,".contains(c));
        if plain {
            line.push_str(arg);
        } else {
            line.push('\'');
            line.push_str(&arg.replace('\'', r"'\''"));
            line.push('\'');
        }
    }
    line
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_lines_quote_arguments_a_shell_would_interpret() {
        assert_eq!(
            command_line(&["launcher", "uninstall", "--serial", "192.168.1.20:5555"]),
            "rusttv launcher uninstall --serial 192.168.1.20:5555"
        );
        assert_eq!(
            command_line(&["--apk", "/Users/me/My APKs/$(rm -rf ~)/it's.apk"]),
            r"rusttv --apk '/Users/me/My APKs/$(rm -rf ~)/it'\''s.apk'"
        );
        assert_eq!(command_line(&[""]), "rusttv ''");
    }
}
