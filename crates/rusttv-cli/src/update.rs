//! `rusttv update ...` commands (SECURITY.md § Updates, ARCHITECTURE D17).
//!
//! Releases come from GitHub through `curl`, or from `RUSTTV_UPDATE_DIR`
//! without any network. Nothing is installed or replaced unless the
//! release's `SHA256SUMS` is signed with the release signing key and the
//! file matches it (`rusttv_core::download_verified`).

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, IsTerminal};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use rusttv_core::{
    Adb, Curl, GrantScope, LAUNCHER_INSTALL, LocalRelease, PlanError, Release, ReleaseSource,
    UpdateError, Version,
};
use serde::Serialize;

use crate::launcher::{approve_and_apply, approver, now_unix_ms, show_plan};
use crate::{CliError, Target, output};

/// This build's Rust target triple (build.rs); picks the CLI archive.
const TARGET: &str = env!("RUSTTV_TARGET");

/// The word `update cli` asks a person to type.
const CONFIRMATION: &str = "update";

/// The program next to `rusttv` that `update cli` also replaces, if present.
const MCP_PROGRAM: &str = "rusttv-mcp";

/// `RUSTTV_UPDATE_DIR` (a release in a directory, no network), else GitHub
/// through `curl` (`RUSTTV_CURL`, else `curl` on `PATH`).
fn release_source() -> Box<dyn ReleaseSource> {
    match std::env::var_os("RUSTTV_UPDATE_DIR") {
        Some(dir) => Box::new(LocalRelease::new(dir)),
        None => {
            Box::new(std::env::var_os("RUSTTV_CURL").map_or_else(Curl::new, Curl::with_program))
        }
    }
}

/// Result of `update check`.
#[derive(Debug, Serialize)]
struct UpdateCheck {
    /// Version of the latest release.
    latest: Version,
    /// Its GitHub page.
    page: String,
    /// This build has a release signing key, so `update cli` / `update
    /// launcher` can verify downloads; without one they refuse.
    can_verify: bool,
    cli: CliCheck,
    /// Null when no device was checked; `launcher_note` says why.
    launcher: Option<LauncherCheck>,
    /// Why no device was checked, or why there is no launcher update.
    launcher_note: Option<String>,
}

#[derive(Debug, Serialize)]
struct CliCheck {
    current: String,
    target: &'static str,
    update_available: bool,
    /// The release's archive for `target`, or null if it has none.
    archive: Option<String>,
}

#[derive(Debug, Serialize)]
struct LauncherCheck {
    serial: String,
    /// `versionName` of the installed launcher, or null if not installed.
    installed: Option<String>,
    abi: Option<String>,
    update_available: bool,
    /// The APK `update launcher` would install, or null.
    apk: Option<String>,
}

/// `rusttv update check`.
pub fn check(adb: &dyn Adb, target: &Target, json: bool) -> Result<(), CliError> {
    let release = rusttv_core::latest_release(release_source().as_ref())?;
    let latest = release.version();
    let cli_update = Version::current().is_some_and(|current| current < latest);
    let archive = rusttv_core::cli_archive_name(latest, TARGET);
    let cli = CliCheck {
        current: rusttv_core::VERSION.to_owned(),
        target: TARGET,
        update_available: cli_update,
        archive: release.asset(&archive).map(|_| archive),
    };

    // With --serial the device must be there, as for every device command;
    // without it the only device is checked, and none (or several) only
    // skips the launcher part, so the CLI check works without a TV.
    let mut launcher_note = None;
    let serial = match (&target.serial, target.resolve(adb)) {
        (_, Ok(serial)) => Some(serial),
        (Some(_), Err(error)) => return Err(error),
        (None, Err(error)) => {
            launcher_note = Some(format!("no device checked: {error}"));
            None
        }
    };
    let launcher = match serial {
        Some(serial) => {
            let device = rusttv_core::launcher_on_device(adb, &serial)?;
            let apk = match rusttv_core::launcher_update_asset(&device, &release) {
                Ok(asset) => Some(asset.name().to_owned()),
                Err(error) => {
                    launcher_note = Some(error.to_string());
                    None
                }
            };
            Some(LauncherCheck {
                serial,
                installed: device.installed,
                abi: device.abi,
                update_available: apk.is_some(),
                apk,
            })
        }
        None => None,
    };
    let report = UpdateCheck {
        latest,
        page: release.page().to_owned(),
        can_verify: rusttv_core::RELEASE_SIGNING_KEY.is_some(),
        cli,
        launcher,
        launcher_note,
    };
    if json {
        output::print_json(&report);
    } else {
        print_check(&report);
    }
    Ok(())
}

fn print_check(report: &UpdateCheck) {
    println!("Latest release:  {} ({})", report.latest, report.page);
    let cli = &report.cli;
    let state = match (&cli.archive, cli.update_available) {
        (_, false) => "up to date".to_owned(),
        (Some(_), true) => format!("{} available: run `rusttv update cli`", report.latest),
        (None, true) => format!("{} has no build for {}", report.latest, cli.target),
    };
    println!("rusttv:          {} ({state})", cli.current);
    if let Some(launcher) = &report.launcher {
        let installed = launcher.installed.as_deref().unwrap_or("not installed");
        let state = if launcher.update_available {
            format!("{} available: run `rusttv update launcher`", report.latest)
        } else {
            "no update".to_owned()
        };
        println!(
            "RustTV launcher: {installed} on {} ({state})",
            launcher.serial
        );
    }
    if let Some(note) = &report.launcher_note {
        output::print_note(note);
    }
    if !report.can_verify {
        output::print_note(
            "This build has no release signing key, so `rusttv update cli` and `rusttv update \
             launcher` refuse to install anything; install the release by hand from its page.",
        );
    }
}

/// `rusttv update launcher`: the normal `launcher install` plan, with the
/// release's APK for the device's primary ABI, downloaded and verified.
pub fn launcher(adb: &dyn Adb, target: &Target, dry_run: bool, json: bool) -> Result<(), CliError> {
    let approver = (!dry_run)
        .then(|| approver(GrantScope::Launcher))
        .transpose()?;
    let serial = target.resolve(adb)?;
    let source = release_source();
    let release = rusttv_core::latest_release(source.as_ref())?;
    let device = rusttv_core::launcher_on_device(adb, &serial)?;
    let asset = rusttv_core::launcher_update_asset(&device, &release)?;
    let key = rusttv_core::release_signing_key()?;

    let work = WorkDir::new()?;
    let apk = download_one(source.as_ref(), &release, asset.name(), work.path(), &key)?;
    let plan = rusttv_core::release_install_plan(
        adb,
        &serial,
        &apk,
        release.version(),
        &rusttv_core::journals_dir()?,
    )?;
    show_plan(&plan, dry_run, json);
    let Some(approver) = approver else {
        return Ok(()); // --dry-run
    };
    let path = approve_and_apply(adb, &plan, &approver, LAUNCHER_INSTALL, json)?;
    if !json {
        println!(
            "\nDone: the RustTV launcher {} is installed.",
            release.version()
        );
        println!("Journal: {}", path.display());
    }
    Ok(())
}

/// Downloads `name` and the signed sums into `dir` and verifies them.
fn download_one(
    source: &dyn ReleaseSource,
    release: &Release,
    name: &str,
    dir: &Path,
    key: &str,
) -> Result<PathBuf, UpdateError> {
    rusttv_core::download_verified(source, release, &[name], dir, key)?
        .into_iter()
        .next()
        .ok_or_else(|| UpdateError::check(format!("`{name}` wasn't downloaded")))
}

/// What `update cli` replaces, and its result in `--json` output.
#[derive(Debug, Serialize)]
struct CliUpdate {
    from: String,
    to: Version,
    /// The verified archive the new files come from.
    archive: String,
    /// The directory of the running `rusttv`.
    dir: String,
    files: Vec<FileReport>,
    confirmation: &'static str,
    /// False for `--dry-run`.
    applied: bool,
}

#[derive(Debug, Serialize)]
struct FileReport {
    path: String,
    /// Where the file being replaced is kept.
    previous: String,
}

/// `rusttv update cli`.
pub fn cli(dry_run: bool, json: bool) -> Result<(), CliError> {
    if !dry_run && !io::stdin().is_terminal() {
        return Err(PlanError::not_approved(
            "this replaces programs on this computer and needs a person to approve it at an \
             interactive terminal; nothing changed (use --dry-run to only see the plan)",
        )
        .into());
    }
    let exe = std::env::current_exe()
        .and_then(fs::canonicalize)
        .map_err(|error| {
            UpdateError::precondition(format!("can't find the running rusttv's path: {error}"))
        })?;
    let dir = exe
        .parent()
        .ok_or_else(|| UpdateError::precondition("the running rusttv has no directory"))?;
    refuse_dev_build(&exe)?;
    check_writable(dir)?;

    let source = release_source();
    let release = rusttv_core::latest_release(source.as_ref())?;
    let version = release.version();
    let current = Version::current().ok_or_else(|| {
        UpdateError::precondition(format!(
            "this build's version `{}` isn't a release version",
            rusttv_core::VERSION
        ))
    })?;
    if version <= current {
        return Err(UpdateError::precondition(format!(
            "rusttv is up to date: this is {current}, the latest release is {version}"
        ))
        .into());
    }
    let key = rusttv_core::release_signing_key()?;
    let archive_name = rusttv_core::cli_archive_name(version, TARGET);

    let work = WorkDir::new()?;
    let downloads = work.subdir("download")?;
    let extracted = work.subdir("extract")?;
    let archive = download_one(source.as_ref(), &release, &archive_name, &downloads, &key)?;
    extract(&archive, &extracted)?;
    let new_files = extracted.join(rusttv_core::cli_archive_dir(version, TARGET));

    let mut files = vec![Replacement::new(&exe, &new_files.join("rusttv"))?];
    let mcp = dir.join(MCP_PROGRAM);
    if mcp.exists() {
        files.push(Replacement::new(&mcp, &new_files.join(MCP_PROGRAM))?);
    }
    let mut report = CliUpdate {
        from: current.to_string(),
        to: version,
        archive: archive_name,
        dir: dir.display().to_string(),
        files: files.iter().map(Replacement::report).collect(),
        confirmation: CONFIRMATION,
        applied: false,
    };
    match (json, dry_run) {
        (true, true) => output::print_json(&report),
        (true, false) => output::print_note(&plan_text(&report)),
        (false, _) => print!("{}", plan_text(&report)),
    }
    if dry_run {
        return Ok(());
    }
    confirm()?;
    replace_and_verify(&files, &exe, version)?;
    report.applied = true;
    if json {
        output::print_json(&report);
    } else {
        println!("\nDone: rusttv {version} is installed.");
        println!(
            "The previous version is kept next to it as `{}.previous`.",
            file_name(&exe)
        );
    }
    Ok(())
}

fn plan_text(report: &CliUpdate) -> String {
    let mut text = format!(
        "Update rusttv {} to {} from {}\n(verified: SHA256SUMS signed with the release signing \
         key, SHA-256 and size)\n\nReplaces, in {}:\n",
        report.from, report.to, report.archive, report.dir
    );
    for (index, file) in report.files.iter().enumerate() {
        text.push_str(&format!(
            "  {}. {}  (the current one is kept as {})\n",
            index + 1,
            file.path,
            file.previous
        ));
    }
    text.push_str(&format!(
        "\nThen runs `rusttv --version`, which must print `rusttv {}`; otherwise every file is \
         put back.\nConfirmation: {}\n",
        report.to, report.confirmation
    ));
    text
}

/// A development build is updated with git and cargo, not from a release.
fn refuse_dev_build(exe: &Path) -> Result<(), UpdateError> {
    let parts: Vec<Component<'_>> = exe.components().collect();
    // `target/debug`, `target/release`, or `target/<triple>/release`.
    let in_target = parts.iter().enumerate().any(|(index, part)| {
        part.as_os_str() == "target"
            && parts[index + 1..]
                .iter()
                .take(2)
                .any(|next| next.as_os_str() == "debug" || next.as_os_str() == "release")
    });
    if in_target {
        return Err(UpdateError::precondition(format!(
            "{} is a development build (in a cargo `target` directory); update it with `git \
             pull` and `cargo build` instead",
            exe.display()
        )));
    }
    Ok(())
}

/// The new files are written next to the old ones, so the directory must be
/// writable by this user.
fn check_writable(dir: &Path) -> Result<(), UpdateError> {
    let probe = dir.join(format!(".rusttv-update-probe-{}", std::process::id()));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .and_then(|_| fs::remove_file(&probe))
        .map_err(|error| {
            UpdateError::precondition(format!(
                "can't write to {} ({error}); rusttv was installed by another user or a \
                 package manager, so update it the same way",
                dir.display()
            ))
        })
}

/// Unpacks the verified archive with `tar` (fixed arguments, no shell).
fn extract(archive: &Path, into: &Path) -> Result<(), UpdateError> {
    let output = Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(into)
        .output()
        .map_err(|error| UpdateError::check(format!("could not run `tar`: {error}")))?;
    if !output.status.success() {
        return Err(UpdateError::check(format!(
            "`tar` couldn't unpack {}: {}",
            archive.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

fn confirm() -> Result<(), CliError> {
    output::print_note(&format!(
        "\nType `{CONFIRMATION}` and press Enter to replace these files; anything else cancels:"
    ));
    let mut typed = String::new();
    io::stdin().read_line(&mut typed).map_err(|error| {
        PlanError::not_approved(&format!("could not read the confirmation: {error}"))
    })?;
    if typed.trim() == CONFIRMATION {
        Ok(())
    } else {
        Err(PlanError::not_approved(&format!(
            "the confirmation `{CONFIRMATION}` was not typed; nothing changed"
        ))
        .into())
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

/// One program `update cli` replaces.
#[derive(Debug)]
struct Replacement {
    /// The installed file.
    path: PathBuf,
    /// The new file, from the unpacked archive.
    new: PathBuf,
    /// `<path>.previous`, where the installed file is kept.
    previous: PathBuf,
    /// `.<name>.rusttv-update`, the new file copied next to `path` so the
    /// final rename stays in one directory (atomic).
    staged: PathBuf,
}

impl Replacement {
    fn new(path: &Path, new: &Path) -> Result<Self, UpdateError> {
        let regular = fs::symlink_metadata(new).is_ok_and(|meta| meta.file_type().is_file());
        if !regular {
            return Err(UpdateError::check(format!(
                "the release archive has no file {}",
                new.display()
            )));
        }
        let name = file_name(path);
        Ok(Self {
            path: path.to_owned(),
            new: new.to_owned(),
            previous: path.with_file_name(format!("{name}.previous")),
            staged: path.with_file_name(format!(".{name}.rusttv-update")),
        })
    }

    fn report(&self) -> FileReport {
        FileReport {
            path: self.path.display().to_string(),
            previous: self.previous.display().to_string(),
        }
    }

    /// Keeps the installed file as `.previous` and renames the new one over
    /// it. If this fails, the installed file is unchanged.
    fn apply(&self) -> io::Result<()> {
        let result = self.stage().and_then(|()| {
            remove_if_present(&self.previous)?;
            // A hard link keeps the installed file in place until the rename.
            fs::hard_link(&self.path, &self.previous)
                .or_else(|_| fs::copy(&self.path, &self.previous).map(|_| ()))?;
            fs::rename(&self.staged, &self.path)
        });
        if result.is_err() {
            let _ = fs::remove_file(&self.staged);
        }
        result
    }

    fn stage(&self) -> io::Result<()> {
        remove_if_present(&self.staged)?;
        fs::copy(&self.new, &self.staged)?;
        fs::set_permissions(&self.staged, fs::Permissions::from_mode(0o755))
    }

    /// Puts the `.previous` file back.
    fn undo(&self) -> Result<(), String> {
        fs::rename(&self.previous, &self.path).map_err(|error| {
            format!(
                "mv '{}' '{}' ({error})",
                self.previous.display(),
                self.path.display()
            )
        })
    }
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Replaces each file in order, then checks `<exe> --version` prints
/// `rusttv <version>`. On any failure every file replaced is put back
/// (`ROLLED_BACK`, or `ROLLBACK_FAILED` with the commands to run by hand).
fn replace_and_verify(
    files: &[Replacement],
    exe: &Path,
    version: Version,
) -> Result<(), UpdateError> {
    let mut done: Vec<&Replacement> = Vec::new();
    for file in files {
        if let Err(error) = file.apply() {
            let reason = format!("replacing {} failed: {error}", file.path.display());
            return Err(roll_back(&done, reason));
        }
        done.push(file);
    }
    let expected = format!("rusttv {version}");
    let reported = Command::new(exe)
        .arg("--version")
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned());
    match reported {
        Ok(reported) if reported == expected => Ok(()),
        Ok(reported) => Err(roll_back(
            &done,
            format!("the new rusttv printed `{reported}` for --version, not `{expected}`"),
        )),
        Err(error) => Err(roll_back(
            &done,
            format!("the new rusttv didn't run: {error}"),
        )),
    }
}

fn roll_back(done: &[&Replacement], reason: String) -> UpdateError {
    let problems: Vec<String> = done
        .iter()
        .rev()
        .filter_map(|file| file.undo().err())
        .collect();
    if problems.is_empty() {
        UpdateError::rolled_back(reason)
    } else {
        UpdateError::rollback_failed(reason, problems)
    }
}

/// A private directory for one run under `<state dir>/updates/`, removed
/// with everything in it when dropped.
#[derive(Debug)]
struct WorkDir(PathBuf);

impl WorkDir {
    fn new() -> Result<Self, CliError> {
        let parent = rusttv_core::state_dir()?.join("updates");
        let dir = parent.join(format!("{}-{}", now_unix_ms(), std::process::id()));
        let create = |path: &Path| {
            DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(path)
                .and_then(|()| fs::canonicalize(path))
                .map_err(|error| {
                    UpdateError::check(format!("can't create {}: {error}", path.display()))
                })
        };
        // Absolute: `launcher install` takes only absolute APK paths.
        Ok(Self(create(&dir)?))
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn subdir(&self, name: &str) -> Result<PathBuf, UpdateError> {
        let dir = self.0.join(name);
        fs::create_dir(&dir).map_err(|error| {
            UpdateError::check(format!("can't create {}: {error}", dir.display()))
        })?;
        Ok(dir)
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp directory for one test, removed afterwards.
    struct Dir(PathBuf);

    impl Dir {
        fn new(test: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("rusttv-cli-update-{}-{test}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("bin")).unwrap();
            fs::create_dir_all(dir.join("new")).unwrap();
            Self(dir)
        }

        /// A script printing `rusttv <version>` at `<dir>/<place>/<name>`.
        fn program(&self, place: &str, name: &str, version: &str) -> PathBuf {
            let path = self.0.join(place).join(name);
            fs::write(&path, format!("#!/bin/sh\necho 'rusttv {version}'\n")).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
            path
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn files_are_replaced_and_the_previous_ones_kept() {
        let dir = Dir::new("replace");
        let exe = dir.program("bin", "rusttv", "0.1.0");
        let mcp = dir.program("bin", "rusttv-mcp", "0.1.0");
        let files = [
            Replacement::new(&exe, &dir.program("new", "rusttv", "0.2.0")).unwrap(),
            Replacement::new(&mcp, &dir.program("new", "rusttv-mcp", "0.2.0")).unwrap(),
        ];

        replace_and_verify(&files, &exe, v("0.2.0")).unwrap();

        assert!(fs::read_to_string(&exe).unwrap().contains("0.2.0"));
        assert!(fs::read_to_string(&mcp).unwrap().contains("0.2.0"));
        let previous = fs::read_to_string(dir.0.join("bin/rusttv.previous")).unwrap();
        assert!(previous.contains("0.1.0"));
        assert!(dir.0.join("bin/rusttv-mcp.previous").exists());
        assert!(!dir.0.join("bin/.rusttv.rusttv-update").exists());
        let mode = fs::metadata(&exe).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }

    #[test]
    fn a_wrong_version_puts_every_file_back() {
        let dir = Dir::new("wrong-version");
        let exe = dir.program("bin", "rusttv", "0.1.0");
        let mcp = dir.program("bin", "rusttv-mcp", "0.1.0");
        let files = [
            Replacement::new(&exe, &dir.program("new", "rusttv", "0.1.5")).unwrap(),
            Replacement::new(&mcp, &dir.program("new", "rusttv-mcp", "0.2.0")).unwrap(),
        ];

        let error = replace_and_verify(&files, &exe, v("0.2.0")).unwrap_err();

        assert_eq!(error.code(), "ROLLED_BACK", "{error}");
        assert!(error.to_string().contains("rusttv 0.1.5"), "{error}");
        assert!(fs::read_to_string(&exe).unwrap().contains("0.1.0"));
        assert!(fs::read_to_string(&mcp).unwrap().contains("0.1.0"));
        assert!(!dir.0.join("bin/rusttv.previous").exists());
    }

    #[test]
    fn a_missing_file_in_the_archive_is_refused_before_anything_changes() {
        let dir = Dir::new("missing");
        let exe = dir.program("bin", "rusttv", "0.1.0");
        let error = Replacement::new(&exe, &dir.0.join("new/rusttv")).unwrap_err();
        assert_eq!(error.code(), "UPDATE_CHECK_FAILED");
    }

    #[test]
    fn development_builds_are_refused() {
        for path in [
            "/src/rusttv/target/debug/rusttv",
            "/src/rusttv/target/release/rusttv",
            "/src/rusttv/target/aarch64-apple-darwin/release/rusttv",
        ] {
            let error = refuse_dev_build(Path::new(path)).unwrap_err();
            assert!(error.to_string().contains("git pull"), "{error}");
        }
        for path in ["/usr/local/bin/rusttv", "/home/me/target/bin/rusttv"] {
            assert!(refuse_dev_build(Path::new(path)).is_ok(), "{path}");
        }
    }

    #[test]
    fn a_directory_that_isnt_writable_is_refused() {
        let error = check_writable(Path::new("/nonexistent/rusttv-test")).unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        let dir = Dir::new("writable");
        check_writable(&dir.0).unwrap();
        assert_eq!(
            fs::read_dir(&dir.0).unwrap().count(),
            2,
            "the probe is removed"
        );
    }
}
