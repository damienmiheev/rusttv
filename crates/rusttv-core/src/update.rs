//! `rusttv update`: which RustTV release is newest, and whether its files are
//! the ones the owner signed (SECURITY.md § Updates, ARCHITECTURE D17).
//!
//! Everything here is sans-IO except [`launcher_on_device`]: parsing GitHub's
//! release JSON, `SHA256SUMS` and versions, and verifying the minisign
//! signature of `SHA256SUMS` and each file against it. Fetching lives in
//! [`crate::download`].
//!
//! Trust: the release feed and TLS only say which version is newest. Nothing
//! is installed or replaced unless `SHA256SUMS` carries a valid signature by
//! [`RELEASE_SIGNING_KEY`] and the file's size and SHA-256 match it. File
//! names contain the version, so a validly signed `SHA256SUMS` of an older
//! release can't pass for a newer one.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize, Serializer};
use sha2::{Digest, Sha256};

use crate::adb::Adb;
use crate::command::{DeviceCommand, PackageName};
use crate::error::AdbError;
use crate::inspect::parse_properties;

/// GitHub's "latest release" of the RustTV repository. It never returns
/// drafts or pre-releases; [`parse_release`] refuses them anyway.
pub const RELEASES_LATEST_URL: &str =
    "https://api.github.com/repos/damienmiheev/rusttv/releases/latest";

/// The release signing key's public half, in minisign's base64 form (the
/// second line of `~/.rusttv-release/rusttv-release.pub`, starting `RW`).
///
/// Key id `C1DB81DC9704FCB0`, created 2026-10-25 (RELEASE.md § Update signing
/// key). With `None` a build can check for updates but refuses to install or
/// replace anything (`UPDATE_VERIFY_FAILED`). Tests never override it: see
/// [`release_signing_key`].
pub const RELEASE_SIGNING_KEY: Option<&str> =
    Some("RWSw/ASX3IHbwWo2E165vf8VLUVfDBY0Ptf2flvZ8Jna7dWbfYMmFMRX");

/// Environment variable with a test signing key, honoured only by debug
/// builds with the `test-util` feature (the CLI's integration tests), never
/// by a release build.
#[cfg(all(feature = "test-util", debug_assertions))]
pub const TEST_SIGNING_KEY_ENV: &str = "RUSTTV_UPDATE_TEST_KEY";

/// The checksum file every release carries.
pub const SUMS_FILE: &str = "SHA256SUMS";
/// The minisign signature of [`SUMS_FILE`].
pub const SUMS_SIGNATURE_FILE: &str = "SHA256SUMS.minisig";

/// Largest release asset RustTV downloads (the APKs are about 15 MB).
pub const MAX_ASSET_BYTES: u64 = 200 * 1024 * 1024;
/// Largest release JSON, `SHA256SUMS` or signature RustTV reads.
pub const MAX_METADATA_BYTES: u64 = 1024 * 1024;

/// Hosts release files may be downloaded from. GitHub serves
/// `browser_download_url`s on `github.com`, which redirect elsewhere; curl
/// follows redirects over HTTPS only (`crate::Curl`).
const ALLOWED_HOSTS: &[&str] = &["github.com", "objects.githubusercontent.com"];

const NO_SIGNING_KEY: &str = "this build has no release signing key, so it can't verify a release \
     (RELEASE_SIGNING_KEY in rusttv-core is unset until the owner creates the key); install the \
     release by hand from its GitHub page, checking SHA256SUMS";

/// A release version, `<major>.<minor>.<patch>` with no pre-release or build
/// part (RELEASE.md § Version). Ordered by number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    major: u32,
    minor: u32,
    patch: u32,
}

impl Version {
    /// Parses `1.2.3`: three decimal numbers without leading zeros. `None`
    /// for anything else (`1.2`, `1.2.3-rc1`, `01.2.3`, ...).
    pub fn parse(text: &str) -> Option<Self> {
        let number = |part: &str| {
            let plain = !part.is_empty()
                && part.bytes().all(|b| b.is_ascii_digit())
                && (part == "0" || !part.starts_with('0'));
            plain.then(|| part.parse().ok()).flatten()
        };
        let mut parts = text.split('.');
        let version = Self {
            major: number(parts.next()?)?,
            minor: number(parts.next()?)?,
            patch: number(parts.next()?)?,
        };
        parts.next().is_none().then_some(version)
    }

    /// Parses a release tag, `v1.2.3`.
    pub fn from_tag(tag: &str) -> Option<Self> {
        tag.strip_prefix('v').and_then(Self::parse)
    }

    /// This build's version ([`crate::VERSION`]); `None` only if the
    /// workspace version isn't a release version.
    pub fn current() -> Option<Self> {
        Self::parse(crate::VERSION)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl Serialize for Version {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// The subset of GitHub's release JSON RustTV reads. Unknown fields are
/// ignored; `RUSTTV_UPDATE_DIR`'s `release.json` has the same shape.
#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    html_url: String,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

/// A published release, checked by [`parse_release`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    version: Version,
    page: String,
    assets: Vec<ReleaseAsset>,
}

impl Release {
    /// The release's version, from its tag.
    pub fn version(&self) -> Version {
        self.version
    }

    /// The release's page on GitHub.
    pub fn page(&self) -> &str {
        &self.page
    }

    /// The file called `name`, if the release has one.
    pub fn asset(&self, name: &str) -> Option<&ReleaseAsset> {
        self.assets.iter().find(|asset| asset.name == name)
    }
}

/// One file of a [`Release`]: a plain file name, an allowed HTTPS URL and a
/// size within [`MAX_ASSET_BYTES`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAsset {
    name: String,
    url: String,
    size: u64,
}

impl ReleaseAsset {
    /// File name: ASCII letters, digits, `.`, `_` and `-`, so it is safe as
    /// a path component.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Download URL, `https://` on an allowed GitHub host.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Size in bytes, as the release states it.
    pub fn size(&self) -> u64 {
        self.size
    }
}

/// Reads GitHub's latest-release JSON. Refuses drafts, pre-releases, tags
/// that aren't `v<major>.<minor>.<patch>`, file names that aren't plain,
/// download URLs off GitHub or not HTTPS, files over [`MAX_ASSET_BYTES`] and
/// duplicate names (`UPDATE_CHECK_FAILED`).
pub fn parse_release(json: &str) -> Result<Release, UpdateError> {
    let release: GithubRelease = serde_json::from_str(json)
        .map_err(|error| UpdateError::check(format!("can't read the release JSON: {error}")))?;
    if release.draft || release.prerelease {
        return Err(UpdateError::check(format!(
            "release `{}` is a draft or pre-release; only published releases count",
            release.tag_name
        )));
    }
    let version = Version::from_tag(&release.tag_name).ok_or_else(|| {
        UpdateError::check(format!(
            "release tag `{}` isn't `v<major>.<minor>.<patch>`",
            release.tag_name
        ))
    })?;
    if !allowed_url(&release.html_url) {
        return Err(UpdateError::check(format!(
            "release page `{}` isn't an https://github.com URL",
            release.html_url
        )));
    }
    let mut assets: Vec<ReleaseAsset> = Vec::with_capacity(release.assets.len());
    for asset in release.assets {
        if !plain_file_name(&asset.name) {
            return Err(UpdateError::check(format!(
                "release file name `{}` isn't a plain file name",
                asset.name
            )));
        }
        if !allowed_url(&asset.browser_download_url) {
            return Err(UpdateError::check(format!(
                "release file `{}` would come from `{}`, which isn't https on {}",
                asset.name,
                asset.browser_download_url,
                ALLOWED_HOSTS.join(" or ")
            )));
        }
        if asset.size > MAX_ASSET_BYTES {
            return Err(UpdateError::check(format!(
                "release file `{}` is {} bytes, more than the {MAX_ASSET_BYTES} RustTV downloads",
                asset.name, asset.size
            )));
        }
        if assets.iter().any(|known| known.name == asset.name) {
            return Err(UpdateError::check(format!(
                "the release lists `{}` twice",
                asset.name
            )));
        }
        assets.push(ReleaseAsset {
            name: asset.name,
            url: asset.browser_download_url,
            size: asset.size,
        });
    }
    Ok(Release {
        version,
        page: release.html_url,
        assets,
    })
}

/// `https://` on an allowed host, with no user, port or whitespace.
fn allowed_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or_default();
    ALLOWED_HOSTS.contains(&host) && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// ASCII letters, digits, `.`, `_` and `-`, not starting with `.` or `-`.
fn plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(['.', '-'])
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The CLI archive of `version` for a Rust target triple (RELEASE.md).
pub fn cli_archive_name(version: Version, target: &str) -> String {
    format!("rusttv-{version}-{target}.tar.gz")
}

/// The directory inside [`cli_archive_name`]'s archive holding the binaries.
pub fn cli_archive_dir(version: Version, target: &str) -> String {
    format!("rusttv-{version}-{target}")
}

/// The launcher APK of `version` for an Android ABI (RELEASE.md).
pub fn launcher_apk_name(version: Version, abi: &str) -> String {
    format!("rusttv-launcher-{version}-{abi}.apk")
}

/// The SHA-256 of each file a release's `SHA256SUMS` lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sums(BTreeMap<String, [u8; 32]>);

impl Sums {
    /// The SHA-256 listed for `name`.
    pub fn get(&self, name: &str) -> Option<&[u8; 32]> {
        self.0.get(name)
    }
}

/// Parses `shasum -a 256` output: `<64 hex digits>  <name>` per line (or
/// ` *<name>`, binary mode). Blank lines are skipped; anything else, a name
/// that isn't plain, or a name listed twice is refused (`UPDATE_VERIFY_FAILED`).
pub fn parse_sums(text: &str) -> Result<Sums, UpdateError> {
    let mut sums = BTreeMap::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let invalid = || UpdateError::verify(format!("{SUMS_FILE} has an invalid line `{line}`"));
        let (hex, name) = line
            .split_once("  ")
            .or_else(|| line.split_once(" *"))
            .ok_or_else(invalid)?;
        let hash = decode_sha256(hex).ok_or_else(invalid)?;
        if !plain_file_name(name) {
            return Err(invalid());
        }
        if sums.insert(name.to_owned(), hash).is_some() {
            return Err(UpdateError::verify(format!(
                "{SUMS_FILE} lists `{name}` twice"
            )));
        }
    }
    Ok(Sums(sums))
}

fn decode_sha256(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 || !hex.is_ascii() {
        return None;
    }
    let mut hash = [0; 32];
    for (byte, pair) in hash.iter_mut().zip(hex.as_bytes().chunks(2)) {
        // ASCII checked above, so every pair is a valid str.
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(hash)
}

/// Checks the minisign `signature` (the contents of `SHA256SUMS.minisig`)
/// of `sums` against `public_key` (minisign base64), then parses `sums`.
/// Only pre-hashed signatures, minisign's default, are accepted.
pub fn verify_sums(sums: &[u8], signature: &str, public_key: &str) -> Result<Sums, UpdateError> {
    let key = minisign_verify::PublicKey::from_base64(public_key.trim()).map_err(|_| {
        UpdateError::verify("the release signing key in this build isn't a minisign public key")
    })?;
    let signature = minisign_verify::Signature::decode(signature).map_err(|_| {
        UpdateError::verify(format!("{SUMS_SIGNATURE_FILE} isn't a minisign signature"))
    })?;
    key.verify(sums, &signature, false).map_err(|error| match error {
        minisign_verify::Error::UnexpectedKeyId => UpdateError::verify(format!(
            "{SUMS_FILE} is signed with another key than the release signing key this build trusts"
        )),
        _ => UpdateError::verify(format!(
            "{SUMS_SIGNATURE_FILE} isn't a valid signature of this {SUMS_FILE}"
        )),
    })?;
    let text = std::str::from_utf8(sums)
        .map_err(|_| UpdateError::verify(format!("{SUMS_FILE} isn't UTF-8")))?;
    parse_sums(text)
}

/// Checks a downloaded file against the release and the verified sums: it
/// must have the size the release states and the SHA-256 `SHA256SUMS` lists.
pub fn verify_asset(
    sums: &Sums,
    asset: &ReleaseAsset,
    size: u64,
    sha256: &[u8; 32],
) -> Result<(), UpdateError> {
    let expected = sums
        .get(&asset.name)
        .ok_or_else(|| UpdateError::verify(format!("{SUMS_FILE} doesn't list `{}`", asset.name)))?;
    if size != asset.size {
        return Err(UpdateError::verify(format!(
            "`{}` is {size} bytes, but the release says {}",
            asset.name, asset.size
        )));
    }
    if sha256 != expected {
        return Err(UpdateError::verify(format!(
            "the SHA-256 of `{}` doesn't match {SUMS_FILE}",
            asset.name
        )));
    }
    Ok(())
}

/// SHA-256 of `bytes`.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// A streaming SHA-256, for files too big to read at once.
#[derive(Debug, Default)]
pub(crate) struct Sha256Hasher(Sha256);

impl Sha256Hasher {
    pub(crate) fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    pub(crate) fn finish(self) -> [u8; 32] {
        self.0.finalize().into()
    }
}

/// The key that verifies releases: [`RELEASE_SIGNING_KEY`], or
/// `UPDATE_VERIFY_FAILED` while it is unset.
///
/// Debug builds with the `test-util` feature (only the CLI's integration
/// tests build that) also honour `RUSTTV_UPDATE_TEST_KEY`, so they can
/// verify a fake release. A release build has neither: the release workflow
/// builds `--release` without dev-dependencies, so the variable is never read.
pub fn release_signing_key() -> Result<String, UpdateError> {
    #[cfg(all(feature = "test-util", debug_assertions))]
    if let Some(key) = std::env::var_os(TEST_SIGNING_KEY_ENV) {
        return key
            .into_string()
            .map_err(|_| UpdateError::verify(format!("{TEST_SIGNING_KEY_ENV} isn't UTF-8")));
    }
    RELEASE_SIGNING_KEY
        .map(str::to_owned)
        .ok_or_else(|| UpdateError::verify(NO_SIGNING_KEY))
}

/// The RustTV launcher on a device, as `rusttv update` needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LauncherOnDevice {
    /// `versionName` of `ai.rusttv.launcher`, or `None` when it isn't installed.
    pub installed: Option<String>,
    /// The device's primary ABI, the first of `ro.product.cpu.abilist`.
    pub abi: Option<String>,
}

/// Reads the installed launcher's version (`dumpsys package
/// ai.rusttv.launcher`) and the primary ABI (`getprop`). Read-only.
pub fn launcher_on_device(adb: &dyn Adb, serial: &str) -> Result<LauncherOnDevice, AdbError> {
    let properties = parse_properties(&adb.run(serial, &DeviceCommand::GetProperties)?);
    let abi = properties
        .get("ro.product.cpu.abilist")
        .and_then(|list| list.split(',').next())
        .filter(|abi| !abi.is_empty())
        .map(str::to_owned);
    let dump = adb.run(
        serial,
        &DeviceCommand::PackageInfo(PackageName::rusttv_launcher()),
    )?;
    Ok(LauncherOnDevice {
        installed: parse_version_name(&dump).map(str::to_owned),
        abi,
    })
}

/// The first `versionName=` in `dumpsys package <package>` output; `None`
/// when the package isn't installed (no package section, so no such line).
pub fn parse_version_name(dumpsys: &str) -> Option<&str> {
    dumpsys
        .lines()
        .find_map(|line| line.trim().strip_prefix("versionName="))
        .map(str::trim)
        .filter(|name| !name.is_empty())
}

/// The APK `rusttv update launcher` installs from `release`: the one for
/// the device's primary ABI. Refuses (`PRECONDITION_FAILED`) when the
/// installed launcher is already that version or newer, or reports a
/// version that can't be compared: an update never downgrades.
pub fn launcher_update_asset<'a>(
    device: &LauncherOnDevice,
    release: &'a Release,
) -> Result<&'a ReleaseAsset, UpdateError> {
    let latest = release.version;
    if let Some(installed) = &device.installed {
        let Some(version) = Version::parse(installed) else {
            return Err(UpdateError::precondition(format!(
                "the installed RustTV launcher reports version `{installed}`, which isn't a \
                 release version, so RustTV can't tell whether {latest} is newer; install the \
                 APK with `rusttv launcher install --apk <file>` instead"
            )));
        };
        if version >= latest {
            return Err(UpdateError::precondition(format!(
                "the RustTV launcher is up to date: {version} is installed, the latest release \
                 is {latest}"
            )));
        }
    }
    let abi = device.abi.as_deref().ok_or_else(|| {
        UpdateError::check("the device doesn't report `ro.product.cpu.abilist`".to_owned())
    })?;
    let name = launcher_apk_name(latest, abi);
    release.asset(&name).ok_or_else(|| {
        UpdateError::check(format!(
            "release {latest} has no launcher APK for the device's ABI `{abi}` (no `{name}`)"
        ))
    })
}

/// Checking for, downloading, verifying or installing an update failed.
///
/// Like [`crate::PlanError`] (M-ERRORS-CANONICAL-STRUCTS): the kind is
/// private; callers use [`UpdateError::code`].
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct UpdateError {
    kind: UpdateErrorKind,
}

#[derive(Debug, thiserror::Error)]
enum UpdateErrorKind {
    #[error(transparent)]
    Adb(#[from] AdbError),
    #[error("update check failed: {0}")]
    Check(String),
    #[error("update not verified: {0}; nothing was installed or replaced")]
    Verify(String),
    #[error("{0}")]
    Precondition(String),
    #[error("{0}; every file replaced was put back")]
    RolledBack(String),
    #[error(
        "{reason}; putting the previous files back ALSO failed: {}. Run those commands by hand",
        problems.join("; ")
    )]
    RollbackFailed {
        reason: String,
        problems: Vec<String>,
    },
}

impl UpdateError {
    /// Stable machine-readable code (CLI.md).
    pub fn code(&self) -> &'static str {
        match &self.kind {
            UpdateErrorKind::Adb(error) => error.code(),
            UpdateErrorKind::Check(_) => "UPDATE_CHECK_FAILED",
            UpdateErrorKind::Verify(_) => "UPDATE_VERIFY_FAILED",
            UpdateErrorKind::Precondition(_) => "PRECONDITION_FAILED",
            UpdateErrorKind::RolledBack(_) => "ROLLED_BACK",
            UpdateErrorKind::RollbackFailed { .. } => "ROLLBACK_FAILED",
        }
    }

    /// The device could not be reached.
    pub fn is_device_unreachable(&self) -> bool {
        matches!(&self.kind, UpdateErrorKind::Adb(error) if error.is_device_unreachable())
    }

    /// The release or its files couldn't be fetched or read (`UPDATE_CHECK_FAILED`).
    pub fn check(reason: impl Into<String>) -> Self {
        UpdateErrorKind::Check(reason.into()).into()
    }

    /// A file isn't what the owner signed (`UPDATE_VERIFY_FAILED`).
    pub fn verify(reason: impl Into<String>) -> Self {
        UpdateErrorKind::Verify(reason.into()).into()
    }

    /// Nothing to do, or this computer or device can't be updated as it is
    /// (`PRECONDITION_FAILED`); nothing changed.
    pub fn precondition(reason: impl Into<String>) -> Self {
        UpdateErrorKind::Precondition(reason.into()).into()
    }

    /// Replacing files failed and every replaced file was put back (`ROLLED_BACK`).
    pub fn rolled_back(reason: impl Into<String>) -> Self {
        UpdateErrorKind::RolledBack(reason.into()).into()
    }

    /// Replacing files failed and putting them back failed too; `problems`
    /// are the commands to run by hand (`ROLLBACK_FAILED`).
    pub fn rollback_failed(reason: impl Into<String>, problems: Vec<String>) -> Self {
        UpdateErrorKind::RollbackFailed {
            reason: reason.into(),
            problems,
        }
        .into()
    }
}

impl<E: Into<UpdateErrorKind>> From<E> for UpdateError {
    fn from(kind: E) -> Self {
        Self { kind: kind.into() }
    }
}

/// One error of every kind; see `crate::error::every_adb_error`.
#[cfg(test)]
pub(crate) fn every_update_error() -> Vec<UpdateError> {
    let all = vec![
        UpdateError::check(""),
        UpdateError::verify(""),
        UpdateError::precondition(""),
        UpdateError::rolled_back(""),
        UpdateError::rollback_failed("", Vec::new()),
    ];
    for error in &all {
        match error.kind {
            UpdateErrorKind::Adb(_)
            | UpdateErrorKind::Check(_)
            | UpdateErrorKind::Verify(_)
            | UpdateErrorKind::Precondition(_)
            | UpdateErrorKind::RolledBack(_)
            | UpdateErrorKind::RollbackFailed { .. } => {}
        }
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adb::FakeAdb;
    use crate::test_release::{TestSigningKey, release_json, sums_text};

    fn version(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn versions_parse_only_major_minor_patch() {
        assert_eq!(version("0.1.0").to_string(), "0.1.0");
        assert_eq!(version("10.20.30").to_string(), "10.20.30");
        for bad in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "1.2.3-rc1",
            "1.2.3+build",
            "01.2.3",
            "1.02.3",
            "1.2.-3",
            " 1.2.3",
            "1..3",
            "a.b.c",
            "4294967296.0.0",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
        assert_eq!(Version::from_tag("v1.2.3"), Some(version("1.2.3")));
        for bad in ["1.2.3", "V1.2.3", "v1.2", "release-1.2.3", "v1.2.3-beta"] {
            assert_eq!(Version::from_tag(bad), None, "{bad}");
        }
        assert!(
            Version::current().is_some(),
            "the workspace version is a release version"
        );
    }

    #[test]
    fn versions_compare_by_number_not_text() {
        assert!(version("0.10.0") > version("0.9.9"));
        assert!(version("1.0.0") > version("0.99.99"));
        assert!(version("0.1.10") > version("0.1.9"));
        assert_eq!(version("0.1.0"), version("0.1.0"));
        assert!(version("0.1.0") < version("0.1.1"));
    }

    const URL: &str = "https://github.com/damienmiheev/rusttv/releases/download/v0.2.0";

    fn release(assets: &[(&str, &[u8])]) -> Release {
        parse_release(&release_json("0.2.0", assets)).unwrap()
    }

    #[test]
    fn a_github_release_is_read_with_its_assets() {
        let release = release(&[
            ("SHA256SUMS", b"sums"),
            ("rusttv-0.2.0-x86_64.tar.gz", b"x"),
        ]);
        assert_eq!(release.version(), version("0.2.0"));
        assert_eq!(
            release.page(),
            "https://github.com/damienmiheev/rusttv/releases/tag/v0.2.0"
        );
        let asset = release.asset("SHA256SUMS").unwrap();
        assert_eq!(asset.url(), format!("{URL}/SHA256SUMS"));
        assert_eq!(asset.size(), 4);
        assert!(release.asset("missing").is_none());
    }

    /// A release JSON with GitHub's fields, one asset, and these overrides.
    fn github_json(tag: &str, draft: bool, prerelease: bool, name: &str, url: &str) -> String {
        serde_json::json!({
            "url": "https://api.github.com/repos/damienmiheev/rusttv/releases/1",
            "tag_name": tag,
            "name": "RustTV",
            "draft": draft,
            "prerelease": prerelease,
            "html_url": "https://github.com/damienmiheev/rusttv/releases/tag/v0.2.0",
            "assets": [{"name": name, "browser_download_url": url, "size": 10, "state": "uploaded"}],
        })
        .to_string()
    }

    #[test]
    fn drafts_pre_releases_and_bad_tags_are_refused() {
        let url = format!("{URL}/SHA256SUMS");
        assert!(parse_release(&github_json("v0.2.0", false, false, "SHA256SUMS", &url)).is_ok());
        for (tag, draft, prerelease) in [
            ("v0.2.0", true, false),
            ("v0.2.0", false, true),
            ("0.2.0", false, false),
            ("v0.2", false, false),
            ("v0.2.0-rc.1", false, false),
            ("nightly", false, false),
        ] {
            let error = parse_release(&github_json(tag, draft, prerelease, "SHA256SUMS", &url))
                .unwrap_err();
            assert_eq!(
                error.code(),
                "UPDATE_CHECK_FAILED",
                "{tag} {draft} {prerelease}"
            );
        }
        assert_eq!(
            parse_release("{\"message\": \"Not Found\"}")
                .unwrap_err()
                .code(),
            "UPDATE_CHECK_FAILED"
        );
    }

    #[test]
    fn asset_urls_must_be_https_on_github() {
        for url in [
            format!("{URL}/a"),
            "https://objects.githubusercontent.com/github-production-release-asset/1".to_owned(),
        ] {
            assert!(
                parse_release(&github_json("v0.2.0", false, false, "a", &url)).is_ok(),
                "{url}"
            );
        }
        for url in [
            "http://github.com/damienmiheev/rusttv/releases/download/v0.2.0/a",
            "https://evil.example/a",
            "https://github.com.evil.example/a",
            "https://user@github.com/a",
            "https://github.com:8443/a",
            "https://github.com/a b",
            "file:///etc/passwd",
            "",
        ] {
            let error = parse_release(&github_json("v0.2.0", false, false, "a", url)).unwrap_err();
            assert_eq!(error.code(), "UPDATE_CHECK_FAILED", "{url}");
        }
    }

    #[test]
    fn asset_names_must_be_plain_file_names() {
        for name in ["../a", "a/b", ".hidden", "-rf", "a b", "", "a\nb"] {
            let json = github_json("v0.2.0", false, false, name, &format!("{URL}/a"));
            assert!(parse_release(&json).is_err(), "{name:?}");
        }
    }

    #[test]
    fn duplicate_and_oversized_assets_are_refused() {
        let mut json: serde_json::Value = serde_json::from_str(&github_json(
            "v0.2.0",
            false,
            false,
            "a",
            &format!("{URL}/a"),
        ))
        .unwrap();
        let asset = json["assets"][0].clone();
        json["assets"].as_array_mut().unwrap().push(asset);
        assert!(parse_release(&json.to_string()).is_err());

        json["assets"].as_array_mut().unwrap().pop();
        json["assets"][0]["size"] = (MAX_ASSET_BYTES + 1).into();
        assert!(parse_release(&json.to_string()).is_err());
    }

    #[test]
    fn sums_parse_shasum_output() {
        let text = sums_text(&[("b.apk", b"two"), ("a.tar.gz", b"one")]);
        let sums = parse_sums(&format!("{text}\n")).unwrap();
        assert_eq!(sums.get("a.tar.gz"), Some(&sha256(b"one")));
        assert_eq!(sums.get("b.apk"), Some(&sha256(b"two")));
        assert_eq!(sums.get("c"), None);

        // Binary mode (`shasum -b`) and upper-case hex.
        let hex = text.lines().next().unwrap().split_once("  ").unwrap().0;
        let binary = format!("{} *a.tar.gz\n", hex.to_uppercase());
        assert_eq!(
            parse_sums(&binary).unwrap().get("a.tar.gz"),
            Some(&sha256(b"one"))
        );
    }

    #[test]
    fn malformed_sums_are_refused() {
        let hash = "0".repeat(64);
        for text in [
            format!("{hash} a"),
            format!("{}  a", "0".repeat(63)),
            format!("{}  a", "g".repeat(64)),
            format!("{hash}  ../a"),
            format!("{hash}  a\n{hash}  a"),
            "Not Found".to_owned(),
        ] {
            assert_eq!(
                parse_sums(&text).unwrap_err().code(),
                "UPDATE_VERIFY_FAILED",
                "{text}"
            );
        }
    }

    #[test]
    fn signed_sums_verify_with_the_right_key_only() {
        let key = TestSigningKey::new(1);
        let sums = sums_text(&[("a.apk", b"apk")]);
        let signature = key.sign(sums.as_bytes());
        let verified = verify_sums(sums.as_bytes(), &signature, &key.public_key()).unwrap();
        assert_eq!(verified.get("a.apk"), Some(&sha256(b"apk")));

        // Tampered sums.
        let tampered = sums.replace("a.apk", "b.apk");
        let error = verify_sums(tampered.as_bytes(), &signature, &key.public_key()).unwrap_err();
        assert_eq!(error.code(), "UPDATE_VERIFY_FAILED");
        assert!(
            error.to_string().contains("isn't a valid signature"),
            "{error}"
        );

        // Signed by another key (another key id).
        let other = TestSigningKey::new(2);
        let error = verify_sums(
            sums.as_bytes(),
            &other.sign(sums.as_bytes()),
            &key.public_key(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("another key"), "{error}");

        // Same key id, other key: the signature itself fails.
        let impostor = TestSigningKey::with_id(2, key.id());
        let error = verify_sums(
            sums.as_bytes(),
            &impostor.sign(sums.as_bytes()),
            &key.public_key(),
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("isn't a valid signature"),
            "{error}"
        );

        // Garbage signature and key.
        assert!(verify_sums(sums.as_bytes(), "not a signature", &key.public_key()).is_err());
        let error = verify_sums(sums.as_bytes(), &signature, "RWQ").unwrap_err();
        assert!(error.to_string().contains("signing key"), "{error}");
    }

    #[test]
    fn legacy_signatures_are_refused() {
        let key = TestSigningKey::new(1);
        let sums = sums_text(&[("a.apk", b"apk")]);
        let error = verify_sums(
            sums.as_bytes(),
            &key.sign_legacy(sums.as_bytes()),
            &key.public_key(),
        )
        .unwrap_err();
        assert_eq!(error.code(), "UPDATE_VERIFY_FAILED");
    }

    #[test]
    fn a_downloaded_file_must_match_size_and_sum() {
        let release = release(&[("a.apk", b"apk")]);
        let asset = release.asset("a.apk").unwrap();
        let sums = parse_sums(&sums_text(&[("a.apk", b"apk")])).unwrap();
        assert!(verify_asset(&sums, asset, 3, &sha256(b"apk")).is_ok());

        let error = verify_asset(&sums, asset, 3, &sha256(b"apx")).unwrap_err();
        assert!(error.to_string().contains("SHA-256"), "{error}");
        let error = verify_asset(&sums, asset, 4, &sha256(b"apk")).unwrap_err();
        assert!(error.to_string().contains("4 bytes"), "{error}");
        let unlisted = parse_sums(&sums_text(&[("b.apk", b"apk")])).unwrap();
        let error = verify_asset(&unlisted, asset, 3, &sha256(b"apk")).unwrap_err();
        assert!(error.to_string().contains("doesn't list"), "{error}");
        assert_eq!(error.code(), "UPDATE_VERIFY_FAILED");
    }

    #[test]
    fn without_a_compiled_in_key_nothing_verifies() {
        // RELEASE_SIGNING_KEY is unset until the owner creates the key; unit
        // tests never set the test key variable.
        if RELEASE_SIGNING_KEY.is_none() {
            let error = release_signing_key().unwrap_err();
            assert_eq!(error.code(), "UPDATE_VERIFY_FAILED");
            assert!(
                error.to_string().contains("no release signing key"),
                "{error}"
            );
        }
    }

    #[test]
    fn the_version_name_comes_from_dumpsys_package() {
        let dump = include_str!("fixtures/synthetic-update/dumpsys-package-rusttv.txt");
        assert_eq!(parse_version_name(dump), Some("0.1.0"));
        let missing = include_str!("fixtures/synthetic-update/dumpsys-package-missing.txt");
        assert_eq!(parse_version_name(missing), None);
        assert_eq!(parse_version_name(""), None);
    }

    fn device(installed: Option<&str>) -> LauncherOnDevice {
        LauncherOnDevice {
            installed: installed.map(str::to_owned),
            abi: Some("arm64-v8a".to_owned()),
        }
    }

    #[test]
    fn the_launcher_is_only_updated_to_a_strictly_newer_version() {
        let release = release(&[("rusttv-launcher-0.2.0-arm64-v8a.apk", b"apk")]);
        for installed in [None, Some("0.1.0"), Some("0.1.9")] {
            let asset = launcher_update_asset(&device(installed), &release).unwrap();
            assert_eq!(asset.name(), "rusttv-launcher-0.2.0-arm64-v8a.apk");
        }
        for installed in ["0.2.0", "0.10.0", "1.0.0"] {
            let error = launcher_update_asset(&device(Some(installed)), &release).unwrap_err();
            assert_eq!(error.code(), "PRECONDITION_FAILED", "{installed}");
            assert!(error.to_string().contains("up to date"), "{error}");
        }
        let error = launcher_update_asset(&device(Some("0.1.0-dev")), &release).unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        assert!(error.to_string().contains("--apk"), "{error}");
    }

    #[test]
    fn a_release_without_the_devices_abi_is_a_check_failure() {
        let release = release(&[("rusttv-launcher-0.2.0-x86_64.apk", b"apk")]);
        let error = launcher_update_asset(&device(Some("0.1.0")), &release).unwrap_err();
        assert_eq!(error.code(), "UPDATE_CHECK_FAILED");
        assert!(error.to_string().contains("arm64-v8a"), "{error}");
        let no_abi = LauncherOnDevice {
            installed: None,
            abi: None,
        };
        assert!(launcher_update_asset(&no_abi, &release).is_err());
    }

    #[test]
    fn launcher_on_device_reads_version_and_primary_abi() {
        let adb = FakeAdb::new()
            .respond(
                DeviceCommand::GetProperties,
                "[ro.product.cpu.abilist]: [armeabi-v7a,armeabi]\n",
            )
            .respond(
                DeviceCommand::PackageInfo(PackageName::rusttv_launcher()),
                include_str!("fixtures/synthetic-update/dumpsys-package-rusttv.txt"),
            );
        let launcher = launcher_on_device(&adb, "tv").unwrap();
        assert_eq!(launcher.installed.as_deref(), Some("0.1.0"));
        assert_eq!(launcher.abi.as_deref(), Some("armeabi-v7a"));
    }
}
