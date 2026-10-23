//! Experimental tool: turn on MediaTek's AirPlay receiver where the TV maker
//! ships it installed but disabled (TOOLS.md).
//!
//! `enable` is a [`Plan`] of two steps: set `setup_airplay_enable` to 1 (R2,
//! the value found is journaled), then start the vendor's `BootupService`
//! through RustTV's trigger activity (R1, undone by force-stopping the
//! vendor process). `disable` restores from the journal. The TV maker writes
//! 7 (hidden) at every boot, so a restart undoes it too.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use crate::adb::Adb;
use crate::apps::package_names;
use crate::command::{
    AIRPLAY_SYSTEM_PACKAGE, Component, DeviceCommand, HiddenFeature, SettingKey, SettingState,
    SettingValue,
};
use crate::compat::{DeviceProfile, profile_file_name};
use crate::inspect::parse_properties;
use crate::journal::{Journal, JournalSearch, find_restorable_journal};
use crate::launcher::RUSTTV_LAUNCHER;
use crate::plan::{Check, Plan, PlanError, Step, device_fingerprint, restore_plan};

/// Plan and journal name of `rusttv tools airplay enable`.
pub const AIRPLAY_TOOL: &str = "tools-airplay";

/// What a person types to approve `enable`.
const CONFIRMATION: &str = "airplay";

/// Shown once per run of every `rusttv tools airplay` command.
pub const AIRPLAY_CAVEAT: &str = "EXPERIMENTAL: AirPlay is Apple's technology. This TV carries a \
    vendor AirPlay receiver that its maker ships disabled on this model, and Apple has not \
    certified this model. It may be incomplete or stop working after a firmware update. RustTV \
    only starts the vendor's own installed software; the change lasts until the TV restarts.";

/// The three MediaTek packages the tool needs: system service, daemon, TV input.
pub const AIRPLAY_PACKAGES: [&str; 3] = [
    AIRPLAY_SYSTEM_PACKAGE,
    "com.mediatek.airplaydaemon",
    "com.mediatek.AirplayAPK",
];

/// The service the trigger activity starts.
const BOOTUP_SERVICE: &str = "com.mediatek.partner.airplay_sys/.BootupService";

/// How long `enable` waits for the service to run.
const START_TIMEOUT: Duration = Duration::from_secs(20);

/// Log tags `status` shows: the vendor components and RustTV's trigger.
const LOG_TAG_PREFIXES: [&str; 4] = [
    "AirPlaySystemService",
    "AirPlayDaemon",
    "AirPlayTIS",
    "RustTV.HiddenFeature",
];

/// How many matching log lines `status` keeps, newest last.
const LOG_LINES: usize = 20;

/// One package [`AirplayStatus`] looks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AirplayPackage {
    /// Package name.
    pub package: &'static str,
    /// Installed (enabled or not).
    pub installed: bool,
}

/// Whether this computer sees AirPlay receivers on the network
/// (`--check-bonjour`); filled in by the CLI, which runs the host tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BonjourCheck {
    /// The tool that ran (`dns-sd` or `avahi-browse`), or `None` if neither
    /// is installed.
    pub tool: Option<String>,
    /// Names of the `_airplay._tcp` services found, in the order seen.
    pub instances: Vec<String>,
    /// Why the check was skipped or may be incomplete.
    pub note: Option<String>,
}

/// What `rusttv tools airplay status` reports (CLI.md § AirplayStatus).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AirplayStatus {
    /// adb serial.
    pub serial: String,
    /// `enable` would be planned on this device (its gate passes).
    pub supported: bool,
    /// Why not, when `supported` is false.
    pub reason: Option<String>,
    /// File name of the bundled compat profile matching the device.
    pub profile: Option<String>,
    /// The three MediaTek packages.
    pub packages: Vec<AirplayPackage>,
    /// The RustTV launcher, which carries the trigger activity, is installed.
    pub launcher_installed: bool,
    /// `settings get global setup_airplay_enable`, `None` when unset. The
    /// device reads below are skipped (`None`) unless all three packages
    /// are installed.
    pub setup_airplay_enable: Option<String>,
    /// `settings get global show_airplay_setting_ui`, `None` when unset.
    pub show_airplay_setting_ui: Option<String>,
    /// The vendor `BootupService` runs.
    pub service_running: Option<bool>,
    /// The last log lines of the AirPlay components and RustTV's trigger;
    /// `None` if the log can't be read.
    pub log: Option<Vec<String>>,
    /// Listening TCP ports; `None` if `ss` is missing or fails.
    pub listening_ports: Option<Vec<u16>>,
    /// A restorable `enable` journal for this firmware, if any.
    pub journal: Option<String>,
    /// `--check-bonjour`, or `None`.
    pub bonjour: Option<BonjourCheck>,
}

/// Whether the gate passes: a bundled profile matches and all packages are
/// installed. Returns the profile's file name, or why not.
struct Gate {
    profile: Option<String>,
    packages: Vec<AirplayPackage>,
    launcher_installed: bool,
}

impl Gate {
    fn read(
        adb: &dyn Adb,
        serial: &str,
        properties: &HashMap<String, String>,
        devices: &[DeviceProfile],
    ) -> Result<Self, PlanError> {
        let output = adb.run(serial, &DeviceCommand::ListPackages)?;
        let installed = package_names(&output)?;
        Ok(Self {
            profile: devices
                .iter()
                .find(|device| device.matches(properties))
                .map(profile_file_name),
            packages: AIRPLAY_PACKAGES
                .into_iter()
                .map(|package| AirplayPackage {
                    package,
                    installed: installed.contains(package),
                })
                .collect(),
            launcher_installed: installed.contains(RUSTTV_LAUNCHER),
        })
    }

    fn has_packages(&self) -> bool {
        self.packages.iter().all(|package| package.installed)
    }

    /// Why `enable` is refused, if it is.
    fn refusal(&self, properties: &HashMap<String, String>) -> Option<String> {
        let missing: Vec<&str> = self
            .packages
            .iter()
            .filter(|package| !package.installed)
            .map(|package| package.package)
            .collect();
        if !missing.is_empty() {
            return Some(format!(
                "this device doesn't have MediaTek's AirPlay stack (missing {})",
                missing.join(", ")
            ));
        }
        if self.profile.is_none() {
            let property = |name: &str| properties.get(name).map_or("unknown", String::as_str);
            return Some(format!(
                "no profile in compat/devices/ matches this device ({} {}, `{}`, firmware `{}`); \
                 experimental tools run only on known models (SECURITY.md: unknown devices get \
                 read-only commands)",
                property("ro.product.manufacturer"),
                property("ro.product.model"),
                property("ro.product.device"),
                property("ro.build.fingerprint"),
            ));
        }
        None
    }
}

/// `rusttv tools airplay status`: read-only.
pub fn airplay_status(
    adb: &dyn Adb,
    serial: &str,
    devices: &[DeviceProfile],
    journals: &Path,
) -> Result<AirplayStatus, PlanError> {
    let properties = parse_properties(&adb.run(serial, &DeviceCommand::GetProperties)?);
    let gate = Gate::read(adb, serial, &properties, devices)?;
    let reason = gate.refusal(&properties);
    let mut status = AirplayStatus {
        serial: serial.to_owned(),
        supported: reason.is_none(),
        reason,
        profile: gate.profile.clone(),
        packages: gate.packages.clone(),
        launcher_installed: gate.launcher_installed,
        setup_airplay_enable: None,
        show_airplay_setting_ui: None,
        service_running: None,
        log: None,
        listening_ports: None,
        journal: None,
        bonjour: None,
    };
    if !gate.has_packages() {
        return Ok(status);
    }
    status.setup_airplay_enable = read_setting(adb, serial, SettingKey::SetupAirplayEnable)?;
    status.show_airplay_setting_ui = read_setting(adb, serial, SettingKey::ShowAirplaySettingUi)?;
    status.service_running = Some(Check::ServiceRunning(bootup_service()).holds(adb, serial)?);
    status.log = adb
        .run(serial, &DeviceCommand::RecentLog)
        .ok()
        .map(|output| airplay_log_lines(&output));
    status.listening_ports = adb
        .run(serial, &DeviceCommand::ListeningSockets)
        .ok()
        .map(|output| listening_ports(&output));
    if let Some(fingerprint) = properties.get("ro.build.fingerprint")
        && let Ok((path, _)) = find_airplay_journal(journals, fingerprint).found
    {
        status.journal = Some(path.display().to_string());
    }
    Ok(status)
}

/// `settings get global <key>`, trimmed; `None` for `null` (unset).
fn read_setting(adb: &dyn Adb, serial: &str, key: SettingKey) -> Result<Option<String>, PlanError> {
    let output = adb.run(serial, &DeviceCommand::GetSetting(key))?;
    let value = output.trim();
    Ok((value != "null").then(|| value.to_owned()))
}

/// Plans `rusttv tools airplay enable`. Read-only on the device.
///
/// Refused (`FEATURE_NOT_SUPPORTED`) unless a bundled compat profile matches
/// the device and all three MediaTek packages are installed; refused
/// (`PRECONDITION_FAILED`) without the RustTV launcher, while an `enable`
/// journal for this firmware isn't restored, when the flag isn't 0 or 7, or
/// when the service already runs.
pub fn airplay_plan(
    adb: &dyn Adb,
    serial: &str,
    devices: &[DeviceProfile],
    journals: &Path,
) -> Result<Plan, PlanError> {
    let properties = parse_properties(&adb.run(serial, &DeviceCommand::GetProperties)?);
    let fingerprint = device_fingerprint(adb, serial)?;
    let gate = Gate::read(adb, serial, &properties, devices)?;
    if let Some(reason) = gate.refusal(&properties) {
        return Err(PlanError::not_supported(reason));
    }
    if !gate.launcher_installed {
        return Err(PlanError::precondition(
            "the RustTV launcher isn't installed; its trigger activity starts the AirPlay \
             service. Install it with `rusttv launcher install`, then try again"
                .to_owned(),
        ));
    }
    if let Ok((path, journal)) = find_airplay_journal(journals, &fingerprint).found {
        return Err(PlanError::precondition(format!(
            "an earlier `enable` ({}) isn't restored; run `rusttv tools airplay disable --journal \
             {}` first (also after a restart of the TV, which turned AirPlay off already)",
            journal.status,
            path.display()
        )));
    }

    let key = SettingKey::SetupAirplayEnable;
    let output = adb.run(serial, &DeviceCommand::GetSetting(key))?;
    let from = SettingState::from_output(&output)
        .ok()
        .filter(|state| key.accepts(state))
        .ok_or_else(|| {
            PlanError::precondition(format!(
                "{key} is `{}`, not 0 or 7; RustTV only changes it from a value it knows",
                output.trim()
            ))
        })?;
    let on = SettingState::Value(
        SettingValue::new("1").map_err(|error| PlanError::invalid_plan(error.to_string()))?,
    );
    if from == on {
        return Err(PlanError::precondition(format!(
            "{key} is 1 already; see `rusttv tools airplay status`"
        )));
    }
    let service = bootup_service();
    if Check::ServiceRunning(service.clone()).holds(adb, serial)? {
        return Err(PlanError::precondition(format!(
            "{service} runs already; nothing to do"
        )));
    }

    let steps = vec![
        Step::new(
            DeviceCommand::SetSetting {
                key,
                to: on,
                from: from.clone(),
            },
            Vec::new(),
        ),
        Step::new(
            DeviceCommand::HiddenFeature(HiddenFeature::AirplayStart),
            vec![Check::ServiceRunning(service)],
        )
        .requiring(vec![Check::PackageInstalled(
            crate::command::PackageName::rusttv_launcher(),
        )])
        .polling_for(START_TIMEOUT),
    ];
    let notes = vec![
        AIRPLAY_CAVEAT.to_owned(),
        format!(
            "{key}: {from} → 1. The TV writes 7 again at the next restart, so AirPlay is on \
             until then."
        ),
        "The trigger activity needs a RustTV launcher with `.tools.HiddenFeatureActivity`; an \
         older one makes step 2 fail and everything is undone."
            .to_owned(),
        "Undo with `rusttv tools airplay disable`: it puts the value found now back and \
         force-stops the AirPlay system service (Android restarts it without AirPlay). Restarting \
         the TV has the same effect."
            .to_owned(),
        "Recovery is never guaranteed (SECURITY.md).".to_owned(),
    ];
    Plan::new(
        AIRPLAY_TOOL,
        serial,
        &fingerprint,
        CONFIRMATION,
        steps,
        notes,
    )
}

/// Plans `rusttv tools airplay disable` from `journal`. Read-only.
///
/// The journal is untrusted: refused unless it is an [`AIRPLAY_TOOL`]
/// journal whose undo only puts `setup_airplay_enable` back and
/// force-stops the AirPlay system service.
pub fn airplay_restore_plan(
    adb: &dyn Adb,
    serial: &str,
    journal: &Journal,
) -> Result<Plan, PlanError> {
    if journal.plan != AIRPLAY_TOOL {
        return Err(PlanError::precondition(format!(
            "this is a `{}` journal, not a {AIRPLAY_TOOL} journal",
            journal.plan
        )));
    }
    let fingerprint = device_fingerprint(adb, serial)?;
    let plan = restore_plan(journal, serial, &fingerprint)?;
    let allowed = |command: &DeviceCommand| match command {
        DeviceCommand::SetSetting { key, .. } => *key == SettingKey::SetupAirplayEnable,
        DeviceCommand::ForceStop(package) => package.as_str() == AIRPLAY_SYSTEM_PACKAGE,
        _ => false,
    };
    if !plan.steps().iter().all(|step| allowed(step.command())) {
        return Err(PlanError::precondition(
            "the journal asks for more than putting the AirPlay switch back; refusing to restore \
             from it"
                .to_owned(),
        ));
    }
    Ok(plan.with_notes([
        "A setting changed since `enable` is left as is.".to_owned(),
        "Without ADB: restart the TV; it turns AirPlay off again.".to_owned(),
    ]))
}

/// The newest restorable `enable` journal for `fingerprint` in `dir`.
pub fn find_airplay_journal(dir: &Path, fingerprint: &str) -> JournalSearch {
    find_restorable_journal(dir, AIRPLAY_TOOL, fingerprint)
}

fn bootup_service() -> Component {
    // A valid component literal; `bootup_service_is_valid` checks it.
    BOOTUP_SERVICE
        .parse()
        .unwrap_or_else(|_| unreachable!("BOOTUP_SERVICE is a valid component"))
}

/// Services in `dumpsys activity services` output, as fully qualified
/// `package/class` (from lines `* ServiceRecord{<id> u<user> <component> ...}`).
pub(crate) fn running_services(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| line.trim().strip_prefix("* ServiceRecord{"))
        .filter_map(|record| record.split_whitespace().nth(2))
        .filter_map(|component| component.trim_end_matches('}').parse::<Component>().ok())
        .map(|component| component.full_name())
        .collect()
}

/// The last [`LOG_LINES`] lines of `logcat` output (threadtime format) whose
/// tag starts with one of [`LOG_TAG_PREFIXES`].
pub(crate) fn airplay_log_lines(output: &str) -> Vec<String> {
    let tag = |line: &str| {
        // `MM-DD HH:MM:SS.mmm  PID  TID L TAG: message`
        let mut fields = line.split_whitespace();
        let tag = fields.nth(5)?;
        Some(tag.trim_end_matches(':').to_owned())
    };
    let lines: Vec<String> = output
        .lines()
        .filter(|line| {
            tag(line).is_some_and(|tag| LOG_TAG_PREFIXES.iter().any(|p| tag.starts_with(p)))
        })
        .map(|line| line.trim_end().to_owned())
        .collect();
    let skip = lines.len().saturating_sub(LOG_LINES);
    lines.into_iter().skip(skip).collect()
}

/// Listening ports in `ss -ltn[p]` output, sorted, without duplicates.
pub(crate) fn listening_ports(output: &str) -> Vec<u16> {
    let mut ports: Vec<u16> = output
        .lines()
        .filter(|line| line.starts_with("LISTEN"))
        .filter_map(|line| line.split_whitespace().nth(3))
        .filter_map(|local| local.rsplit_once(':'))
        .filter_map(|(_, port)| port.parse().ok())
        .collect();
    ports.sort_unstable();
    ports.dedup();
    ports
}

/// Service names in the output of `dns-sd -B _airplay._tcp local.` (macOS),
/// each once (dns-sd lists a service per network interface).
pub fn parse_dns_sd_browse(output: &str) -> Vec<String> {
    // `Timestamp  A/R  Flags  if  Domain  Service Type  Instance Name`
    let mut names: Vec<String> = Vec::new();
    for line in output.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() > 6 && fields[1] == "Add" {
            let name = fields[6..].join(" ");
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// Service names in the output of `avahi-browse -t -p _airplay._tcp`
/// (Linux): `+;<interface>;<protocol>;<name>;<type>;<domain>`, with avahi's
/// `\DDD` decimal escapes decoded. One name per service, though avahi lists
/// it per interface and protocol.
pub fn parse_avahi_browse(output: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for line in output.lines() {
        let fields: Vec<&str> = line.split(';').collect();
        if fields.len() >= 6 && fields[0] == "+" {
            let name = unescape_avahi(fields[3]);
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

fn unescape_avahi(text: &str) -> String {
    let mut bytes = Vec::with_capacity(text.len());
    let raw = text.as_bytes();
    let mut index = 0;
    while index < raw.len() {
        let digits = raw.get(index + 1..index + 4);
        match (raw[index], digits) {
            (b'\\', Some(digits)) if digits.iter().all(u8::is_ascii_digit) => {
                let value = digits
                    .iter()
                    .fold(0u32, |sum, digit| sum * 10 + u32::from(digit - b'0'));
                bytes.push(u8::try_from(value).unwrap_or(b'?'));
                index += 4;
            }
            (byte, _) => {
                bytes.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;
    use crate::RiskClass;
    use crate::adb::FakeAdb;
    use crate::command::PackageName;
    use crate::compat::bundled_profiles;
    use crate::journal::tests::MemoryJournal;
    use crate::journal::{FileJournal, JournalStatus, JournalStore, StepStatus, journal_file_name};
    use crate::plan::{apply, restore};

    const SERIAL: &str = "tv";
    const GETPROP: &str = include_str!("fixtures/philips-65oled818-api34/getprop.txt");

    macro_rules! fixture {
        ($file:literal) => {
            include_str!(concat!("fixtures/synthetic-airplay/", $file))
        };
    }

    fn setting(key: SettingKey) -> DeviceCommand {
        DeviceCommand::GetSetting(key)
    }

    fn services() -> DeviceCommand {
        DeviceCommand::RunningServices(PackageName::new(AIRPLAY_SYSTEM_PACKAGE).unwrap())
    }

    fn value(text: &str) -> SettingState {
        SettingState::Value(SettingValue::new(text).unwrap())
    }

    /// The Philips with the AirPlay stack, the flag at 7 and nothing running.
    fn philips() -> FakeAdb {
        FakeAdb::new()
            .respond(DeviceCommand::GetProperties, GETPROP)
            .respond(DeviceCommand::ListPackages, fixture!("packages-f.txt"))
            .respond(
                setting(SettingKey::SetupAirplayEnable),
                fixture!("setting-setup-airplay-enable-7.txt"),
            )
            .respond(
                setting(SettingKey::ShowAirplaySettingUi),
                fixture!("setting-show-airplay-setting-ui-null.txt"),
            )
            .respond(services(), fixture!("services-not-running.txt"))
            .respond(DeviceCommand::RecentLog, fixture!("logcat-not-ready.txt"))
            .respond(
                DeviceCommand::ListeningSockets,
                fixture!("ss-ltnp-before.txt"),
            )
    }

    /// A fresh, empty journal directory.
    fn journals_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rusttv-airplay-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn bootup_service_is_valid() {
        assert!(BOOTUP_SERVICE.parse::<Component>().is_ok());
        assert_eq!(
            bootup_service().full_name(),
            "com.mediatek.partner.airplay_sys/com.mediatek.partner.airplay_sys.BootupService"
        );
    }

    #[test]
    fn status_on_the_philips_reports_the_flag_service_log_and_ports() {
        let dir = journals_dir("status");
        let status = airplay_status(&philips(), SERIAL, &bundled_profiles(), &dir).unwrap();
        assert!(status.supported, "{:?}", status.reason);
        assert_eq!(status.profile.as_deref(), Some("tpv-ph1m-ww-9972.toml"));
        assert!(status.packages.iter().all(|package| package.installed));
        assert!(status.launcher_installed);
        assert_eq!(status.setup_airplay_enable.as_deref(), Some("7"));
        assert_eq!(status.show_airplay_setting_ui, None);
        assert_eq!(status.service_running, Some(false));
        let log = status.log.unwrap();
        assert_eq!(log.len(), 3);
        assert!(log[1].ends_with("Luna is not ready"));
        assert_eq!(status.listening_ports, Some(vec![5555, 8008]));
        assert_eq!(status.journal, None);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn status_without_ss_or_logcat_still_reports_the_rest() {
        let dir = journals_dir("status-no-ss");
        let adb = FakeAdb::new()
            .respond(DeviceCommand::GetProperties, GETPROP)
            .respond(DeviceCommand::ListPackages, fixture!("packages-f.txt"))
            .respond(setting(SettingKey::SetupAirplayEnable), "1\n")
            .respond(setting(SettingKey::ShowAirplaySettingUi), "1\n")
            .respond(services(), fixture!("services-running.txt"));
        let status = airplay_status(&adb, SERIAL, &bundled_profiles(), &dir).unwrap();
        assert_eq!(status.service_running, Some(true));
        assert_eq!(status.show_airplay_setting_ui.as_deref(), Some("1"));
        assert_eq!(status.log, None);
        assert_eq!(status.listening_ports, None);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn status_without_the_packages_reads_nothing_else() {
        let dir = journals_dir("status-missing");
        let adb = FakeAdb::new()
            .respond(DeviceCommand::GetProperties, GETPROP)
            .respond(
                DeviceCommand::ListPackages,
                fixture!("packages-f-no-airplayapk.txt"),
            );
        let status = airplay_status(&adb, SERIAL, &bundled_profiles(), &dir).unwrap();
        assert!(!status.supported);
        assert!(status.reason.unwrap().contains("com.mediatek.AirplayAPK"));
        assert_eq!(status.setup_airplay_enable, None);
        assert_eq!(status.service_running, None);
        assert_eq!(
            adb.calls(),
            [DeviceCommand::GetProperties, DeviceCommand::ListPackages]
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn enable_is_refused_without_a_matching_profile_or_the_packages() {
        let dir = journals_dir("gate");
        let unknown = philips().respond(
            DeviceCommand::GetProperties,
            GETPROP.replace("Philips Google TV TA1", "Another TV"),
        );
        let error = airplay_plan(&unknown, SERIAL, &bundled_profiles(), &dir).unwrap_err();
        assert_eq!(error.code(), "FEATURE_NOT_SUPPORTED");
        assert!(error.to_string().contains("no profile"), "{error}");

        let missing = philips().respond(
            DeviceCommand::ListPackages,
            fixture!("packages-f-no-airplayapk.txt"),
        );
        let error = airplay_plan(&missing, SERIAL, &bundled_profiles(), &dir).unwrap_err();
        assert_eq!(error.code(), "FEATURE_NOT_SUPPORTED");
        assert!(
            error.to_string().contains("com.mediatek.AirplayAPK"),
            "{error}"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn enable_plans_the_flag_then_the_trigger_and_waits_for_the_service() {
        let dir = journals_dir("plan");
        let plan = airplay_plan(&philips(), SERIAL, &bundled_profiles(), &dir).unwrap();
        assert_eq!(plan.name(), AIRPLAY_TOOL);
        assert_eq!(plan.confirmation(), "airplay");
        assert_eq!(plan.max_risk(), RiskClass::Setting);
        let steps = plan.steps();
        assert_eq!(
            steps[0].command(),
            &DeviceCommand::SetSetting {
                key: SettingKey::SetupAirplayEnable,
                to: value("1"),
                from: value("7"),
            }
        );
        assert_eq!(
            steps[1].command(),
            &DeviceCommand::HiddenFeature(HiddenFeature::AirplayStart)
        );
        assert_eq!(
            steps[1].verification(),
            [Check::ServiceRunning(bootup_service())]
        );
        assert_eq!(steps[1].poll(), Some(START_TIMEOUT));
        assert_eq!(
            steps[1].undo(),
            Some(DeviceCommand::ForceStop(
                PackageName::new(AIRPLAY_SYSTEM_PACKAGE).unwrap()
            ))
        );
        assert!(plan.dry_run().notes[0].starts_with("EXPERIMENTAL"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn enable_is_refused_from_an_unknown_flag_value_or_when_already_on() {
        let dir = journals_dir("flag");
        for (output, expected) in [
            ("2\n", "not 0 or 7"),
            ("null\n", "not 0 or 7"),
            ("on\n", "not 0 or 7"),
            ("1\n", "1 already"),
        ] {
            let adb = philips().respond(setting(SettingKey::SetupAirplayEnable), output);
            let error = airplay_plan(&adb, SERIAL, &bundled_profiles(), &dir).unwrap_err();
            assert_eq!(error.code(), "PRECONDITION_FAILED", "{output:?}");
            assert!(error.to_string().contains(expected), "{output:?}: {error}");
        }
        let running = philips().respond(services(), fixture!("services-running.txt"));
        let error = airplay_plan(&running, SERIAL, &bundled_profiles(), &dir).unwrap_err();
        assert!(error.to_string().contains("runs already"), "{error}");

        let no_launcher = philips().respond(
            DeviceCommand::ListPackages,
            fixture!("packages-f.txt").replace("ai.rusttv.launcher", "com.example.other"),
        );
        let error = airplay_plan(&no_launcher, SERIAL, &bundled_profiles(), &dir).unwrap_err();
        assert!(error.to_string().contains("launcher install"), "{error}");
        fs::remove_dir_all(dir).unwrap();
    }

    /// The Philips during `enable`: the flag reads 7 until it is put, the
    /// service runs after the trigger.
    fn philips_enabling() -> FakeAdb {
        philips()
            .respond_sequence(
                setting(SettingKey::SetupAirplayEnable),
                [
                    // planning, apply's original-state check
                    "7\n".to_owned(),
                    "7\n".to_owned(),
                    // step 1's verification
                    "1\n".to_owned(),
                ],
            )
            .respond(
                DeviceCommand::SetSetting {
                    key: SettingKey::SetupAirplayEnable,
                    to: value("1"),
                    from: value("7"),
                },
                "",
            )
            .respond(
                DeviceCommand::HiddenFeature(HiddenFeature::AirplayStart),
                fixture!("am-start-trigger.txt"),
            )
    }

    #[test]
    fn enable_applies_and_journals_both_steps() {
        let dir = journals_dir("apply");
        let adb = philips_enabling().respond_sequence(
            services(),
            [
                fixture!("services-not-running.txt").to_owned(),
                fixture!("services-not-running.txt").to_owned(),
                fixture!("services-running.txt").to_owned(),
            ],
        );
        let plan = airplay_plan(&adb, SERIAL, &bundled_profiles(), &dir).unwrap();
        let approval = plan.approve("airplay").unwrap();
        let mut store = MemoryJournal::default();
        let journal = apply(&adb, &plan, &approval, &mut store, &|_| {}).unwrap();
        assert_eq!(journal.status, JournalStatus::Applied);
        assert!(
            journal
                .steps
                .iter()
                .all(|step| step.status == StepStatus::Done)
        );
        assert_eq!(
            journal.steps[0].original_state,
            Some(Check::SettingIs {
                key: SettingKey::SetupAirplayEnable,
                state: value("7"),
            })
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_service_that_never_starts_rolls_back_the_flag_and_force_stops() {
        let dir = journals_dir("rollback");
        let stop = DeviceCommand::ForceStop(PackageName::new(AIRPLAY_SYSTEM_PACKAGE).unwrap());
        let put_back = DeviceCommand::SetSetting {
            key: SettingKey::SetupAirplayEnable,
            to: value("7"),
            from: value("1"),
        };
        let adb = philips_enabling()
            .respond_sequence(
                setting(SettingKey::SetupAirplayEnable),
                ["7\n", "7\n", "1\n", "7\n"].map(str::to_owned),
            )
            .respond(stop.clone(), fixture!("am-force-stop.txt"))
            .respond(put_back.clone(), "");
        let plan = airplay_plan(&adb, SERIAL, &bundled_profiles(), &dir).unwrap();
        let approval = plan.approve("airplay").unwrap();
        let mut store = MemoryJournal::default();
        let error = apply(&adb, &plan, &approval, &mut store, &|_| {}).unwrap_err();
        assert_eq!(error.code(), "ROLLED_BACK", "{error}");
        assert!(error.to_string().contains("within 20 s"), "{error}");
        let calls = adb.calls();
        let stopped = calls.iter().position(|call| *call == stop).unwrap();
        let restored = calls.iter().position(|call| *call == put_back).unwrap();
        assert!(stopped < restored, "undo runs last step first");
        assert_eq!(store.last().status, JournalStatus::RolledBack);
        fs::remove_dir_all(dir).unwrap();
    }

    fn applied_journal(name: &str) -> Journal {
        let dir = journals_dir(name);
        let adb = philips_enabling().respond_sequence(
            services(),
            [
                fixture!("services-not-running.txt").to_owned(),
                fixture!("services-not-running.txt").to_owned(),
                fixture!("services-running.txt").to_owned(),
            ],
        );
        let plan = airplay_plan(&adb, SERIAL, &bundled_profiles(), &dir).unwrap();
        let approval = plan.approve("airplay").unwrap();
        let mut store = MemoryJournal::default();
        fs::remove_dir_all(dir).unwrap();
        apply(&adb, &plan, &approval, &mut store, &|_| {}).unwrap()
    }

    #[test]
    fn disable_force_stops_then_puts_the_original_back() {
        let mut journal = applied_journal("j-disable");
        let stop = DeviceCommand::ForceStop(PackageName::new(AIRPLAY_SYSTEM_PACKAGE).unwrap());
        let put_back = DeviceCommand::SetSetting {
            key: SettingKey::SetupAirplayEnable,
            to: value("7"),
            from: value("1"),
        };
        let adb = philips()
            .respond_sequence(
                setting(SettingKey::SetupAirplayEnable),
                ["1\n", "7\n"].map(str::to_owned),
            )
            .respond(stop.clone(), "")
            .respond(put_back.clone(), "");
        let plan = airplay_restore_plan(&adb, SERIAL, &journal).unwrap();
        let commands: Vec<&DeviceCommand> = plan.steps().iter().map(Step::command).collect();
        assert_eq!(commands, [&stop, &put_back]);
        assert_eq!(plan.confirmation(), "restore");
        let approval = plan.approve("restore").unwrap();
        let mut store = MemoryJournal::default();
        let left = restore(&adb, &plan, &approval, &mut journal, &mut store).unwrap();
        assert!(left.is_empty(), "{left:?}");
        assert_eq!(journal.status, JournalStatus::Restored);
    }

    #[test]
    fn disable_refuses_other_or_tampered_journals() {
        let adb = philips();
        let mut other = applied_journal("j-other");
        other.plan = "privacy".to_owned();
        let error = airplay_restore_plan(&adb, SERIAL, &other).unwrap_err();
        assert!(error.to_string().contains("not a tools-airplay"), "{error}");

        let mut tampered = applied_journal("j-tampered");
        tampered.steps[1].command = DeviceCommand::DisablePackage(
            PackageName::new("com.google.android.tvlauncher").unwrap(),
        );
        let error = airplay_restore_plan(&adb, SERIAL, &tampered).unwrap_err();
        assert!(error.to_string().contains("refusing"), "{error}");

        // A value the key doesn't take is refused by the engine itself.
        let mut bad_value = applied_journal("j-bad");
        bad_value.steps[0].command = DeviceCommand::SetSetting {
            key: SettingKey::SetupAirplayEnable,
            to: value("1"),
            from: value("2"),
        };
        let error = airplay_restore_plan(&adb, SERIAL, &bad_value).unwrap_err();
        assert_eq!(error.code(), "INVALID_PLAN", "{error}");
    }

    #[test]
    fn enable_is_refused_while_an_earlier_one_is_not_restored() {
        let dir = journals_dir("open");
        let journal = applied_journal("j-open");
        let path = dir.join(journal_file_name(1, AIRPLAY_TOOL));
        FileJournal::new(&path).save(&journal).unwrap();
        let error = airplay_plan(&philips(), SERIAL, &bundled_profiles(), &dir).unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        assert!(error.to_string().contains("disable --journal"), "{error}");
        let status = airplay_status(&philips(), SERIAL, &bundled_profiles(), &dir).unwrap();
        assert_eq!(status.journal, Some(path.display().to_string()));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parsers_read_services_log_ports_and_bonjour() {
        assert_eq!(
            running_services(fixture!("services-not-running.txt")),
            Vec::<String>::new()
        );
        assert_eq!(
            running_services(fixture!("services-running.txt")),
            ["com.mediatek.partner.airplay_sys/com.mediatek.partner.airplay_sys.BootupService"]
        );
        let log = airplay_log_lines(fixture!("logcat-started.txt"));
        assert_eq!(log.len(), 5);
        assert!(log[0].contains("RustTV.HiddenFeature: airplay start"));
        assert!(airplay_log_lines("10-23 12:00:00.000 1 1 I OtherTag: AirPlayDaemon\n").is_empty());
        assert_eq!(
            listening_ports(fixture!("ss-ltnp-after.txt")),
            [5555, 7000, 7100, 8008, 49152]
        );
        assert_eq!(
            parse_dns_sd_browse(fixture!("dns-sd-airplay.txt")),
            ["Example TV"]
        );
        assert!(parse_dns_sd_browse(fixture!("dns-sd-none.txt")).is_empty());
        let twice = format!(
            "{}12:10:05.130  Add        2   4 local.               _airplay._tcp.       Example TV\n",
            fixture!("dns-sd-airplay.txt")
        );
        assert_eq!(parse_dns_sd_browse(&twice), ["Example TV"]);
        assert_eq!(
            parse_avahi_browse(fixture!("avahi-browse-airplay.txt")),
            ["Example TV"]
        );
        assert_eq!(
            parse_avahi_browse(
                "+;wlan0;IPv4;Living\\032Room;_airplay._tcp;local\n+;wlan0;IPv6;Living\\032Room;_airplay._tcp;local\n"
            ),
            ["Living Room"]
        );
    }

    #[test]
    fn log_keeps_only_the_newest_lines() {
        let many: String = (0..30)
            .map(|n| format!("10-23 12:00:00.000 1 1 I AirPlayDaemon: line {n}\n"))
            .collect();
        let log = airplay_log_lines(&many);
        assert_eq!(log.len(), LOG_LINES);
        assert!(log.last().unwrap().ends_with("line 29"));
    }
}
