//! Fake signed releases for tests (M-TEST-UTIL; feature `test-util`).
//!
//! [`TestSigningKey`] makes minisign signatures in the format `minisign -Sm`
//! writes (pre-hashed, `ED`), from a fixed seed: a throwaway key that only
//! tests trust. Never enable this in a release build.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::Path;

use blake2::{Blake2b512, Digest};
use ed25519_compact::{KeyPair, Seed};

use crate::update::{SUMS_FILE, SUMS_SIGNATURE_FILE, sha256};

/// A minisign key pair made from a fixed seed.
#[derive(Debug)]
pub struct TestSigningKey {
    pair: KeyPair,
    id: [u8; 8],
}

impl TestSigningKey {
    /// The key for `seed`; its key id is `seed` repeated.
    pub fn new(seed: u8) -> Self {
        Self::with_id(seed, [seed; 8])
    }

    /// The key for `seed` with another key id, e.g. to pose as another key.
    pub fn with_id(seed: u8, id: [u8; 8]) -> Self {
        Self {
            pair: KeyPair::from_seed(Seed::new([seed; 32])),
            id,
        }
    }

    /// The key id.
    pub fn id(&self) -> [u8; 8] {
        self.id
    }

    /// The public key as minisign prints it (second line of the `.pub` file).
    pub fn public_key(&self) -> String {
        let mut bytes = b"Ed".to_vec();
        bytes.extend_from_slice(&self.id);
        bytes.extend_from_slice(&*self.pair.pk);
        base64(&bytes)
    }

    /// A `.minisig` file for `data`: pre-hashed (BLAKE2b-512), as `minisign
    /// -Sm` writes by default.
    pub fn sign(&self, data: &[u8]) -> String {
        self.signature(b"ED", &Blake2b512::digest(data))
    }

    /// A legacy (not pre-hashed) `.minisig`, which RustTV refuses.
    pub fn sign_legacy(&self, data: &[u8]) -> String {
        self.signature(b"Ed", data)
    }

    fn signature(&self, algorithm: &[u8; 2], message: &[u8]) -> String {
        let signature = self.pair.sk.sign(message, None);
        let trusted = "timestamp:1761300000\tfile:SHA256SUMS\thashed";
        let mut global = signature.to_vec();
        global.extend_from_slice(trusted.as_bytes());
        let global = self.pair.sk.sign(&global, None);

        let mut first = algorithm.to_vec();
        first.extend_from_slice(&self.id);
        first.extend_from_slice(&*signature);
        format!(
            "untrusted comment: signature from the RustTV test key\n{}\ntrusted comment: {trusted}\n{}\n",
            base64(&first),
            base64(&*global)
        )
    }
}

/// `SHA256SUMS` for these files, as `shasum -a 256` prints it, sorted by name.
pub fn sums_text(assets: &[(&str, &[u8])]) -> String {
    let mut sorted: Vec<&(&str, &[u8])> = assets.iter().collect();
    sorted.sort_by_key(|(name, _)| *name);
    let mut text = String::new();
    for (name, bytes) in sorted {
        for byte in sha256(bytes) {
            let _ = write!(text, "{byte:02x}");
        }
        let _ = writeln!(text, "  {name}");
    }
    text
}

/// A GitHub latest-release JSON for `v<version>` listing these files.
pub fn release_json(version: &str, assets: &[(&str, &[u8])]) -> String {
    let base = "https://github.com/damienmiheev/rusttv/releases";
    let assets: Vec<serde_json::Value> = assets
        .iter()
        .map(|(name, bytes)| {
            serde_json::json!({
                "name": name,
                "browser_download_url": format!("{base}/download/v{version}/{name}"),
                "size": bytes.len(),
            })
        })
        .collect();
    serde_json::json!({
        "tag_name": format!("v{version}"),
        "draft": false,
        "prerelease": false,
        "html_url": format!("{base}/tag/v{version}"),
        "assets": assets,
    })
    .to_string()
}

/// Writes a release for `RUSTTV_UPDATE_DIR` into `dir`: the files, their
/// `SHA256SUMS` signed with `key`, its signature, and `release.json` listing
/// all of them.
pub fn write_test_release(
    dir: &Path,
    version: &str,
    assets: &[(&str, &[u8])],
    key: &TestSigningKey,
) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let sums = sums_text(assets);
    let signature = key.sign(sums.as_bytes());
    let mut all: Vec<(&str, &[u8])> = assets.to_vec();
    all.push((SUMS_FILE, sums.as_bytes()));
    all.push((SUMS_SIGNATURE_FILE, signature.as_bytes()));
    for (name, bytes) in &all {
        fs::write(dir.join(name), bytes)?;
    }
    fs::write(dir.join("release.json"), release_json(version, &all))
}

/// Standard base64 with padding, as minisign writes it.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                text.push(char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                text.push('=');
            }
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard_alphabet_with_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe]), "//4=");
    }

    #[test]
    fn a_test_key_is_a_valid_minisign_key() {
        let key = TestSigningKey::new(7);
        let public = minisign_verify::PublicKey::from_base64(&key.public_key()).unwrap();
        let signature = minisign_verify::Signature::decode(&key.sign(b"data")).unwrap();
        public.verify(b"data", &signature, false).unwrap();
        assert!(signature.trusted_comment().contains("SHA256SUMS"));
    }
}
