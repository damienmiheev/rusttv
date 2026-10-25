//! Pairing, authentication and the paired phones (REMOTE.md § 2).
//!
//! Every value is an HMAC-SHA256 under a key the two sides already share,
//! over a fixed domain label, a zero byte, then fixed-length parts (a
//! variable-length part, such as the phone's name, only ever comes last), so
//! no two inputs can be confused. The page's JavaScript computes the same
//! bytes; the test vectors below pin them.
//!
//! - **Pairing:** the QR code carries a 32-byte pairing secret. The phone
//!   sends a fresh nonce, its name and [`pairing_proof`]; the TV checks it
//!   ([`verify_pairing_proof`]) and both show [`match_code`]. After OK on the
//!   TV, both derive the phone's [`PhoneKeys`]; the pairing secret is then
//!   forgotten.
//! - **Each connection:** the phone says who it is, the TV sends a 32-byte
//!   challenge, the phone answers with [`auth_response`] and the TV checks it
//!   ([`verify_auth`]). Both derive [`session_key`] from the challenge.

use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::RemoteError;
use crate::protocol::is_phone_name;

/// Length of every secret, nonce and HMAC here.
pub const SECRET_LEN: usize = 32;
/// Most phones the TV keeps paired.
pub const MAX_PHONES: usize = 16;
/// The phones file's format.
pub const PHONES_FORMAT: u32 = 1;

type HmacSha256 = Hmac<Sha256>;

fn mac(key: &[u8], label: &str, parts: &[&[u8]]) -> HmacSha256 {
    // HMAC takes keys of any length (longer ones are hashed), so this can't
    // fail.
    #[expect(clippy::expect_used, reason = "HMAC accepts any key length")]
    let mut mac =
        <HmacSha256 as KeyInit>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(label.as_bytes());
    mac.update(&[0]);
    for part in parts {
        mac.update(part);
    }
    mac
}

fn compute(key: &[u8], label: &str, parts: &[&[u8]]) -> [u8; 32] {
    mac(key, label, parts).finalize().into_bytes().into()
}

fn verify(key: &[u8], label: &str, parts: &[&[u8]], expected: &[u8; 32]) -> bool {
    // Constant time: `verify_slice` compares without an early exit.
    mac(key, label, parts).verify_slice(expected).is_ok()
}

/// The phone's proof that it read the QR code: binds its nonce and name.
pub fn pairing_proof(pairing_secret: &[u8; 32], phone_nonce: &[u8; 32], name: &str) -> [u8; 32] {
    compute(
        pairing_secret,
        "rusttv-remote/1/pair",
        &[phone_nonce, name.as_bytes()],
    )
}

/// Whether `proof` is [`pairing_proof`] for these values, in constant time.
pub fn verify_pairing_proof(
    pairing_secret: &[u8; 32],
    phone_nonce: &[u8; 32],
    name: &str,
    proof: &[u8; 32],
) -> bool {
    verify(
        pairing_secret,
        "rusttv-remote/1/pair",
        &[phone_nonce, name.as_bytes()],
        proof,
    )
}

/// The 2-digit code the TV and the phone both show while pairing (0–99,
/// shown with two digits).
pub fn match_code(pairing_secret: &[u8; 32], phone_nonce: &[u8; 32]) -> u8 {
    let bytes = compute(pairing_secret, "rusttv-remote/1/code", &[phone_nonce]);
    // u16 % 100: a bias below 0.1 %, irrelevant for a code a person compares.
    (u16::from_be_bytes([bytes[0], bytes[1]]) % 100) as u8
}

/// What a paired phone and the TV share: `auth` answers challenges, `key`
/// seeds each session's encryption key. Neither ever crosses the network.
#[derive(Clone, PartialEq, Eq)]
pub struct PhoneKeys {
    pub auth: [u8; 32],
    pub key: [u8; 32],
}

impl std::fmt::Debug for PhoneKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PhoneKeys(…)")
    }
}

impl PhoneKeys {
    /// Both keys, from the pairing secret and the phone's pairing nonce.
    pub fn derive(pairing_secret: &[u8; 32], phone_nonce: &[u8; 32]) -> Self {
        Self {
            auth: compute(pairing_secret, "rusttv-remote/1/auth-key", &[phone_nonce]),
            key: compute(
                pairing_secret,
                "rusttv-remote/1/session-seed",
                &[phone_nonce],
            ),
        }
    }
}

/// A new Home Screen icon's proof that it holds a handover token
/// (REMOTE.md § 1 "Add to Home Screen"): binds its nonce and the phone.
pub fn handover_proof(token: &[u8; 32], phone_nonce: &[u8; 32], phone_id: &PhoneId) -> [u8; 32] {
    compute(
        token,
        "rusttv-remote/1/handover",
        &[phone_nonce, &phone_id.0],
    )
}

/// Whether `proof` is [`handover_proof`] for these values, in constant time.
pub fn verify_handover_proof(
    token: &[u8; 32],
    phone_nonce: &[u8; 32],
    phone_id: &PhoneId,
    proof: &[u8; 32],
) -> bool {
    verify(
        token,
        "rusttv-remote/1/handover",
        &[phone_nonce, &phone_id.0],
        proof,
    )
}

/// The key the TV seals a phone's keys with for a redeemed handover token.
pub fn handover_key(token: &[u8; 32], phone_nonce: &[u8; 32]) -> [u8; 32] {
    compute(token, "rusttv-remote/1/handover-key", &[phone_nonce])
}

/// The phone's answer to a challenge.
pub fn auth_response(auth: &[u8; 32], challenge: &[u8; 32], phone_id: &PhoneId) -> [u8; 32] {
    compute(auth, "rusttv-remote/1/auth", &[challenge, &phone_id.0])
}

/// Whether `response` is [`auth_response`] for these values, in constant
/// time.
pub fn verify_auth(
    auth: &[u8; 32],
    challenge: &[u8; 32],
    phone_id: &PhoneId,
    response: &[u8; 32],
) -> bool {
    verify(
        auth,
        "rusttv-remote/1/auth",
        &[challenge, &phone_id.0],
        response,
    )
}

/// This connection's encryption key ([`Session`](crate::Session)).
pub fn session_key(key: &[u8; 32], challenge: &[u8; 32]) -> [u8; 32] {
    compute(key, "rusttv-remote/1/session", &[challenge])
}

/// A paired phone's id: 16 random bytes, 32 lowercase hex digits.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PhoneId(pub [u8; 16]);

impl std::fmt::Debug for PhoneId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PhoneId({self})")
    }
}

impl std::fmt::Display for PhoneId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::hex::encode(&self.0))
    }
}

impl TryFrom<String> for PhoneId {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        crate::hex::decode(&text)
            .map(Self)
            .ok_or_else(|| "expected 32 lowercase hex digits".to_owned())
    }
}

impl From<PhoneId> for String {
    fn from(id: PhoneId) -> String {
        id.to_string()
    }
}

/// A paired phone, as the TV keeps it.
#[derive(Clone, PartialEq, Eq)]
pub struct Phone {
    pub id: PhoneId,
    /// A clean name ([`clean_phone_name`](crate::protocol::clean_phone_name)).
    pub name: String,
    pub keys: PhoneKeys,
    /// Unix seconds.
    pub paired_at: u64,
    /// Unix seconds of the last authenticated connection.
    pub last_seen: Option<u64>,
}

impl std::fmt::Debug for Phone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Phone")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("paired_at", &self.paired_at)
            .field("last_seen", &self.last_seen)
            .finish_non_exhaustive()
    }
}

/// The paired phones (`files/remote/phones.toml` on the TV).
///
/// A file that doesn't parse is an error; the caller keeps it as `.bak` and
/// starts with no phones, so a damaged file never lets anyone in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Phones(Vec<Phone>);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PhonesFile {
    format: u32,
    #[serde(default, rename = "phone")]
    phones: Vec<PhoneEntry>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PhoneEntry {
    id: PhoneId,
    name: String,
    auth: String,
    key: String,
    paired_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_seen: Option<u64>,
}

impl Phones {
    /// Reads a phones file.
    pub fn parse(text: &str) -> Result<Self, RemoteError> {
        let file: PhonesFile =
            toml::from_str(text).map_err(|error| RemoteError::Invalid(error.to_string()))?;
        if file.format > PHONES_FORMAT {
            return Err(RemoteError::FormatNewer(file.format));
        }
        if file.format != PHONES_FORMAT {
            return Err(RemoteError::Invalid(format!(
                "phones format {}",
                file.format
            )));
        }
        let mut phones = Self::default();
        for entry in file.phones {
            let secret = |text: &str| {
                crate::hex::decode::<SECRET_LEN>(text)
                    .ok_or_else(|| RemoteError::Invalid("phone secret".to_owned()))
            };
            phones.add(Phone {
                id: entry.id,
                name: entry.name,
                keys: PhoneKeys {
                    auth: secret(&entry.auth)?,
                    key: secret(&entry.key)?,
                },
                paired_at: entry.paired_at,
                last_seen: entry.last_seen,
            })?;
        }
        Ok(phones)
    }

    /// The phones file's text.
    pub fn to_toml(&self) -> Result<String, RemoteError> {
        let file = PhonesFile {
            format: PHONES_FORMAT,
            phones: self
                .0
                .iter()
                .map(|phone| PhoneEntry {
                    id: phone.id,
                    name: phone.name.clone(),
                    auth: crate::hex::encode(&phone.keys.auth),
                    key: crate::hex::encode(&phone.keys.key),
                    paired_at: phone.paired_at,
                    last_seen: phone.last_seen,
                })
                .collect(),
        };
        toml::to_string(&file).map_err(|error| RemoteError::Invalid(error.to_string()))
    }

    /// Adds a phone. Refused: a name that isn't clean, an id already paired,
    /// or more than [`MAX_PHONES`].
    pub fn add(&mut self, phone: Phone) -> Result<(), RemoteError> {
        if !is_phone_name(&phone.name) {
            return Err(RemoteError::Invalid("phone name".to_owned()));
        }
        if self.get(&phone.id).is_some() {
            return Err(RemoteError::Invalid("phone already paired".to_owned()));
        }
        if self.0.len() >= MAX_PHONES {
            return Err(RemoteError::TooLong("paired phones"));
        }
        self.0.push(phone);
        Ok(())
    }

    /// Unpairs a phone; the removed phone, if it was paired.
    pub fn remove(&mut self, id: &PhoneId) -> Option<Phone> {
        let index = self.0.iter().position(|phone| phone.id == *id)?;
        Some(self.0.remove(index))
    }

    /// Renames a phone. Refused if the name isn't clean or the phone isn't
    /// paired.
    pub fn rename(&mut self, id: &PhoneId, name: &str) -> Result<(), RemoteError> {
        if !is_phone_name(name) {
            return Err(RemoteError::Invalid("phone name".to_owned()));
        }
        let phone = self
            .0
            .iter_mut()
            .find(|phone| phone.id == *id)
            .ok_or_else(|| RemoteError::Invalid("no such phone".to_owned()))?;
        name.clone_into(&mut phone.name);
        Ok(())
    }

    /// Records an authenticated connection.
    pub fn seen(&mut self, id: &PhoneId, now: u64) {
        if let Some(phone) = self.0.iter_mut().find(|phone| phone.id == *id) {
            phone.last_seen = Some(now);
        }
    }

    pub fn get(&self, id: &PhoneId) -> Option<&Phone> {
        self.0.iter().find(|phone| phone.id == *id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Phone> {
        self.0.iter()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hex;

    const SECRET: [u8; 32] = [0x11; 32];
    const NONCE: [u8; 32] = [0x22; 32];
    const CHALLENGE: [u8; 32] = [0x33; 32];
    const ID: PhoneId = PhoneId([0x44; 16]);

    /// Pins the bytes the page's JavaScript must produce. The expected values
    /// were computed with Python's `hmac` module, independently of this code.
    #[test]
    fn test_vectors() {
        assert_eq!(
            hex::encode(&pairing_proof(&SECRET, &NONCE, "Pixel 7")),
            "a98e0ad09f9b1d9403972c024af0bfd9c3a18cf431547dcba65c845d4705d94e"
        );
        assert_eq!(match_code(&SECRET, &NONCE), 99);
        let keys = PhoneKeys::derive(&SECRET, &NONCE);
        assert_eq!(
            hex::encode(&keys.auth),
            "022c6e5321ecdd6d5f573853b9b8d58e16320a6cdb45c0fedeeba28c86d0a867"
        );
        assert_eq!(
            hex::encode(&keys.key),
            "2cccd5037566ae24a4921e84b37296e87e59a923f26fdd40b1143b08fd1d5268"
        );
        assert_eq!(
            hex::encode(&auth_response(&keys.auth, &CHALLENGE, &ID)),
            "8619ebc32fc9c6ffdd6e2c4f1eba95bef2dad871104daaab7513ef70798aa80d"
        );
        assert_eq!(
            hex::encode(&session_key(&keys.key, &CHALLENGE)),
            "0d42934d0f8d09a23873d11d8a56cd1f8a9bf3d9604f0c2e96f18074661d735f"
        );
        // A handover token's proof and key (the token in SECRET's place).
        let proof = handover_proof(&SECRET, &NONCE, &ID);
        assert_eq!(
            hex::encode(&proof),
            "24fe519bc7c508e33d69669a79d3824fc9b03837fd75866f8806175b53fb621c"
        );
        assert!(verify_handover_proof(&SECRET, &NONCE, &ID, &proof));
        assert!(!verify_handover_proof(
            &SECRET,
            &NONCE,
            &PhoneId([0x45; 16]),
            &proof
        ));
        assert_eq!(
            hex::encode(&handover_key(&SECRET, &NONCE)),
            "3db785d4f42218527494c55f3b5543ba7fa14668388f011e92a62d6dbaeb74cc"
        );
    }

    #[test]
    fn pairing_proof_binds_nonce_and_name() {
        let proof = pairing_proof(&SECRET, &NONCE, "Pixel 7");
        assert!(verify_pairing_proof(&SECRET, &NONCE, "Pixel 7", &proof));
        assert!(!verify_pairing_proof(&SECRET, &NONCE, "Pixel 8", &proof));
        assert!(!verify_pairing_proof(
            &SECRET,
            &[0x23; 32],
            "Pixel 7",
            &proof
        ));
        assert!(!verify_pairing_proof(
            &[0x12; 32],
            &NONCE,
            "Pixel 7",
            &proof
        ));
    }

    #[test]
    fn auth_binds_challenge_and_phone() {
        let keys = PhoneKeys::derive(&SECRET, &NONCE);
        let response = auth_response(&keys.auth, &CHALLENGE, &ID);
        assert!(verify_auth(&keys.auth, &CHALLENGE, &ID, &response));
        assert!(!verify_auth(&keys.auth, &[0x34; 32], &ID, &response));
        assert!(!verify_auth(
            &keys.auth,
            &CHALLENGE,
            &PhoneId([0x45; 16]),
            &response
        ));
        assert!(!verify_auth(&keys.key, &CHALLENGE, &ID, &response));
    }

    #[test]
    fn keys_and_labels_are_distinct() {
        let keys = PhoneKeys::derive(&SECRET, &NONCE);
        assert_ne!(keys.auth, keys.key);
        assert_ne!(session_key(&keys.key, &CHALLENGE), keys.key);
        assert!(match_code(&SECRET, &NONCE) < 100);
    }

    fn phone(byte: u8, name: &str) -> Phone {
        Phone {
            id: PhoneId([byte; 16]),
            name: name.to_owned(),
            keys: PhoneKeys::derive(&SECRET, &[byte; 32]),
            paired_at: 1_792_938_447,
            last_seen: None,
        }
    }

    #[test]
    fn phones_file_round_trips() {
        let mut phones = Phones::default();
        phones.add(phone(1, "iPhone (Safari)")).unwrap();
        phones.add(phone(2, "Pixel 7")).unwrap();
        phones.seen(&PhoneId([2; 16]), 1_792_940_000);
        let text = phones.to_toml().unwrap();
        assert!(text.starts_with("format = 1\n"));
        assert_eq!(Phones::parse(&text).unwrap(), phones);
        assert_eq!(Phones::parse("format = 1\n").unwrap(), Phones::default());
    }

    #[test]
    fn phones_file_refuses_damage() {
        assert!(matches!(
            Phones::parse("format = 2\n"),
            Err(RemoteError::FormatNewer(2))
        ));
        assert!(Phones::parse("format = 0\n").is_err());
        assert!(Phones::parse("").is_err());
        assert!(Phones::parse("format = 1\nextra = true\n").is_err());
        let mut phones = Phones::default();
        phones.add(phone(1, "Pixel 7")).unwrap();
        let text = phones.to_toml().unwrap();
        assert!(Phones::parse(&text.replace("Pixel 7", "\u{202E}Pixel")).is_err());
        let short_key = text.replacen(&hex::encode(&phones.0[0].keys.auth), "abcd", 1);
        assert!(Phones::parse(&short_key).is_err());
        let twice = format!("{text}{}", &text[text.find("[[phone]]").unwrap()..]);
        assert!(Phones::parse(&twice).is_err());
    }

    #[test]
    fn phones_limits_and_edits() {
        let mut phones = Phones::default();
        for byte in 0..u8::try_from(MAX_PHONES).unwrap() {
            phones.add(phone(byte, "Phone")).unwrap();
        }
        assert!(matches!(
            phones.add(phone(200, "One more")),
            Err(RemoteError::TooLong(_))
        ));
        assert!(phones.rename(&PhoneId([3; 16]), "Kitchen iPad").is_ok());
        assert!(phones.rename(&PhoneId([3; 16]), " ").is_err());
        assert_eq!(phones.get(&PhoneId([3; 16])).unwrap().name, "Kitchen iPad");
        assert!(phones.remove(&PhoneId([3; 16])).is_some());
        assert!(phones.remove(&PhoneId([3; 16])).is_none());
        assert_eq!(phones.len(), MAX_PHONES - 1);
    }

    #[test]
    fn debug_hides_secrets() {
        let text = format!("{:?}", phone(1, "Pixel 7"));
        assert!(!text.contains(&hex::encode(&phone(1, "Pixel 7").keys.auth)));
        assert!(text.contains("Pixel 7"));
    }
}
