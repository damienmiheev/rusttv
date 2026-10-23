//! Fetching releases for `rusttv update` (ARCHITECTURE D17).
//!
//! [`ReleaseSource`] is the seam (M-MOCKABLE-SYSCALLS): [`Curl`] fetches
//! from GitHub by spawning `curl` with a fixed argument list (no shell, no
//! TLS stack in RustTV), and [`LocalRelease`] reads a release from a
//! directory (`RUSTTV_UPDATE_DIR`) without any network. [`download_verified`]
//! fetches files through either and checks them against the signed sums.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::update::{
    MAX_ASSET_BYTES, MAX_METADATA_BYTES, RELEASES_LATEST_URL, Release, ReleaseAsset, SUMS_FILE,
    SUMS_SIGNATURE_FILE, Sha256Hasher, UpdateError, parse_release, verify_asset, verify_sums,
};

/// Where releases come from.
pub trait ReleaseSource {
    /// The latest release's JSON, as GitHub's API returns it.
    fn latest(&self) -> Result<String, UpdateError>;

    /// Writes `asset` to `dest`, which must not exist yet.
    fn download(&self, asset: &ReleaseAsset, dest: &Path) -> Result<(), UpdateError>;
}

/// Fetches from GitHub with the `curl` program.
#[derive(Debug, Clone)]
pub struct Curl {
    program: OsString,
}

impl Curl {
    /// Uses the `curl` found on `PATH`.
    pub fn new() -> Self {
        Self::with_program("curl")
    }

    /// Uses a specific `curl` executable (`RUSTTV_CURL`).
    pub fn with_program(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
        }
    }

    /// The fixed arguments: fail on HTTP errors, follow redirects but over
    /// HTTPS only, TLS 1.2 or newer, at most `max_bytes`, errors on stderr.
    fn args(max_bytes: u64) -> Vec<String> {
        [
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--tlsv1.2",
            "--connect-timeout",
            "30",
            "--max-filesize",
        ]
        .into_iter()
        .map(str::to_owned)
        .chain([max_bytes.to_string()])
        .collect()
    }

    /// Runs curl with [`Curl::args`] plus `extra`, returning its stdout.
    fn run(
        &self,
        max_bytes: u64,
        extra: &[&std::ffi::OsStr],
        url: &str,
    ) -> Result<Vec<u8>, UpdateError> {
        let output = Command::new(&self.program)
            .args(Self::args(max_bytes))
            .args(extra)
            .arg(url)
            .output()
            .map_err(|error| {
                UpdateError::check(if error.kind() == io::ErrorKind::NotFound {
                    "`curl` was not found; install curl or set RUSTTV_CURL".to_owned()
                } else {
                    format!("could not run `curl`: {error}")
                })
            })?;
        if !output.status.success() {
            return Err(UpdateError::check(format!(
                "`curl {url}` failed with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(output.stdout)
    }
}

impl Default for Curl {
    fn default() -> Self {
        Self::new()
    }
}

impl ReleaseSource for Curl {
    fn latest(&self) -> Result<String, UpdateError> {
        let accept = std::ffi::OsStr::new("Accept: application/vnd.github+json");
        let stdout = self.run(
            MAX_METADATA_BYTES,
            &["--header".as_ref(), accept],
            RELEASES_LATEST_URL,
        )?;
        String::from_utf8(stdout)
            .map_err(|_| UpdateError::check("GitHub's release JSON isn't UTF-8"))
    }

    fn download(&self, asset: &ReleaseAsset, dest: &Path) -> Result<(), UpdateError> {
        // `--output` creates the file; the size check after download catches
        // a server that ignores `--max-filesize`.
        self.run(
            asset.size().min(MAX_ASSET_BYTES),
            &["--output".as_ref(), dest.as_os_str()],
            asset.url(),
        )?;
        Ok(())
    }
}

/// A release in a local directory (`RUSTTV_UPDATE_DIR`): `release.json` in
/// the shape of GitHub's response, and each file under its asset name. No
/// network at all; the files are verified exactly like downloads.
#[derive(Debug, Clone)]
pub struct LocalRelease {
    dir: PathBuf,
}

impl LocalRelease {
    /// The release in `dir`.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl ReleaseSource for LocalRelease {
    fn latest(&self) -> Result<String, UpdateError> {
        let path = self.dir.join("release.json");
        let bytes = read_capped(&path, MAX_METADATA_BYTES)
            .map_err(|error| UpdateError::check(format!("{}: {error}", path.display())))?;
        String::from_utf8(bytes)
            .map_err(|_| UpdateError::check(format!("{} isn't UTF-8", path.display())))
    }

    fn download(&self, asset: &ReleaseAsset, dest: &Path) -> Result<(), UpdateError> {
        // Asset names are plain file names (`parse_release`), so this stays in `dir`.
        let source = self.dir.join(asset.name());
        fs::copy(&source, dest)
            .map(|_| ())
            .map_err(|error| UpdateError::check(format!("{}: {error}", source.display())))
    }
}

/// Fetches and parses the latest release.
pub fn latest_release(source: &dyn ReleaseSource) -> Result<Release, UpdateError> {
    parse_release(&source.latest()?)
}

/// Downloads `SHA256SUMS` and its signature into `dir`, verifies the
/// signature with `public_key`, then downloads each of `names` into `dir`
/// and checks its size and SHA-256. Returns the paths of `names`, in order.
///
/// `dir` should be empty and private to this run; the caller removes it.
pub fn download_verified(
    source: &dyn ReleaseSource,
    release: &Release,
    names: &[&str],
    dir: &Path,
    public_key: &str,
) -> Result<Vec<PathBuf>, UpdateError> {
    let asset = |name: &str| {
        release.asset(name).ok_or_else(|| {
            UpdateError::check(format!("release {} has no `{name}`", release.version()))
        })
    };
    let fetch_small = |name: &str| -> Result<Vec<u8>, UpdateError> {
        let path = fetch(source, asset(name)?, dir)?;
        read_capped(&path, MAX_METADATA_BYTES)
            .map_err(|error| UpdateError::check(format!("{}: {error}", path.display())))
    };
    let sums = fetch_small(SUMS_FILE)?;
    let signature = String::from_utf8(fetch_small(SUMS_SIGNATURE_FILE)?)
        .map_err(|_| UpdateError::verify(format!("{SUMS_SIGNATURE_FILE} isn't UTF-8")))?;
    let sums = verify_sums(&sums, &signature, public_key)?;

    let mut paths = Vec::with_capacity(names.len());
    for name in names {
        let asset = asset(name)?;
        let path = fetch(source, asset, dir)?;
        let (size, hash) = sha256_file(&path)
            .map_err(|error| UpdateError::check(format!("{}: {error}", path.display())))?;
        verify_asset(&sums, asset, size, &hash)?;
        paths.push(path);
    }
    Ok(paths)
}

fn fetch(
    source: &dyn ReleaseSource,
    asset: &ReleaseAsset,
    dir: &Path,
) -> Result<PathBuf, UpdateError> {
    let path = dir.join(asset.name());
    if path.exists() {
        return Err(UpdateError::check(format!(
            "{} already exists; the download directory must be empty",
            path.display()
        )));
    }
    source.download(asset, &path)?;
    Ok(path)
}

/// Reads at most `max` bytes of `path`; more is an error.
fn read_capped(path: &Path, max: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(io::Error::other(format!("larger than {max} bytes")));
    }
    Ok(bytes)
}

/// Size and SHA-256 of a file, read in chunks.
fn sha256_file(path: &Path) -> io::Result<(u64, [u8; 32])> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256Hasher::default();
    let mut buffer = vec![0; 64 * 1024];
    let mut size = 0;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            return Ok((size, hasher.finish()));
        }
        size += read as u64;
        hasher.update(&buffer[..read]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_release::{TestSigningKey, write_test_release};

    /// A temp directory with a release in `release/` and an empty `downloads/`.
    struct Dir(PathBuf);

    impl Dir {
        fn new(test: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("rusttv-download-{}-{test}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("downloads")).unwrap();
            Self(dir)
        }
        fn release(&self) -> PathBuf {
            self.0.join("release")
        }
        fn downloads(&self) -> PathBuf {
            self.0.join("downloads")
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const APK: &str = "rusttv-launcher-0.2.0-arm64-v8a.apk";

    fn signed_release(dir: &Dir, key: &TestSigningKey) -> LocalRelease {
        write_test_release(&dir.release(), "0.2.0", &[(APK, b"apk bytes")], key).unwrap();
        LocalRelease::new(dir.release())
    }

    #[test]
    fn a_signed_release_downloads_and_verifies() {
        let dir = Dir::new("ok");
        let key = TestSigningKey::new(1);
        let source = signed_release(&dir, &key);
        let release = latest_release(&source).unwrap();
        let paths = download_verified(
            &source,
            &release,
            &[APK],
            &dir.downloads(),
            &key.public_key(),
        )
        .unwrap();
        assert_eq!(paths, [dir.downloads().join(APK)]);
        assert_eq!(fs::read(&paths[0]).unwrap(), b"apk bytes");
    }

    #[test]
    fn a_tampered_file_is_refused() {
        let dir = Dir::new("tampered");
        let key = TestSigningKey::new(1);
        let source = signed_release(&dir, &key);
        let release = latest_release(&source).unwrap();
        // Same size, other bytes.
        fs::write(dir.release().join(APK), b"apk bytez").unwrap();
        let error = download_verified(
            &source,
            &release,
            &[APK],
            &dir.downloads(),
            &key.public_key(),
        )
        .unwrap_err();
        assert_eq!(error.code(), "UPDATE_VERIFY_FAILED");
        assert!(error.to_string().contains("SHA-256"), "{error}");
    }

    #[test]
    fn a_file_of_another_size_is_refused() {
        let dir = Dir::new("size");
        let key = TestSigningKey::new(1);
        let source = signed_release(&dir, &key);
        let release = latest_release(&source).unwrap();
        fs::write(dir.release().join(APK), b"apk bytes and more").unwrap();
        let error = download_verified(
            &source,
            &release,
            &[APK],
            &dir.downloads(),
            &key.public_key(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("bytes"), "{error}");
    }

    #[test]
    fn sums_signed_by_another_key_are_refused_before_any_file() {
        let dir = Dir::new("wrong-key");
        let source = signed_release(&dir, &TestSigningKey::new(2));
        let release = latest_release(&source).unwrap();
        let trusted = TestSigningKey::new(1).public_key();
        let error =
            download_verified(&source, &release, &[APK], &dir.downloads(), &trusted).unwrap_err();
        assert_eq!(error.code(), "UPDATE_VERIFY_FAILED");
        assert!(!dir.downloads().join(APK).exists(), "the APK isn't fetched");
    }

    #[test]
    fn tampered_sums_are_refused() {
        let dir = Dir::new("tampered-sums");
        let key = TestSigningKey::new(1);
        let source = signed_release(&dir, &key);
        let sums = dir.release().join(SUMS_FILE);
        let text = fs::read_to_string(&sums).unwrap();
        // Keep the size (release.json states it); change one hex digit.
        let flipped = if text.starts_with('0') { "1" } else { "0" };
        fs::write(&sums, format!("{flipped}{}", &text[1..])).unwrap();
        let release = latest_release(&source).unwrap();
        let error = download_verified(
            &source,
            &release,
            &[APK],
            &dir.downloads(),
            &key.public_key(),
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("isn't a valid signature"),
            "{error}"
        );
    }

    #[test]
    fn a_missing_file_is_a_check_failure() {
        let dir = Dir::new("missing");
        let key = TestSigningKey::new(1);
        let source = signed_release(&dir, &key);
        let release = latest_release(&source).unwrap();
        let error = download_verified(
            &source,
            &release,
            &["rusttv-launcher-0.2.0-x86_64.apk"],
            &dir.downloads(),
            &key.public_key(),
        )
        .unwrap_err();
        assert_eq!(error.code(), "UPDATE_CHECK_FAILED");
    }

    #[test]
    fn curl_gets_a_fixed_argument_list() {
        assert_eq!(
            Curl::args(1024).join(" "),
            "--fail --location --silent --show-error --proto =https --proto-redir =https \
             --tlsv1.2 --connect-timeout 30 --max-filesize 1024"
        );
    }

    #[test]
    fn missing_curl_is_a_check_failure() {
        let error = Curl::with_program("/nonexistent/rusttv-test/curl")
            .latest()
            .unwrap_err();
        assert_eq!(error.code(), "UPDATE_CHECK_FAILED");
        assert!(error.to_string().contains("RUSTTV_CURL"), "{error}");
    }

    #[test]
    fn large_metadata_is_refused() {
        let dir = Dir::new("large");
        let path = dir.0.join("big");
        fs::write(&path, vec![b'x'; 11]).unwrap();
        assert!(read_capped(&path, 10).is_err());
        assert_eq!(read_capped(&path, 11).unwrap().len(), 11);
    }
}
