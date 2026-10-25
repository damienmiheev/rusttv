//! HTTPS for the phone page (REMOTE.md § 2 "Trusting the TV"): the TV is its
//! own small certificate authority, and a phone that trusts it once opens the
//! page over HTTPS with no warning (and gets what browsers only give secure
//! pages, such as motion sensors).
//!
//! The authority may only vouch for home network addresses (10/8,
//! 172.16/12, 192.168/16, as name constraints that browsers enforce), so
//! even its leaked key couldn't impersonate a real website. It is made once
//! and kept in the launcher's private storage; the certificate for an address
//! is issued when a phone first connects to it, so a new address after a
//! reboot needs nothing from the person.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};

use rcgen::{
    BasicConstraints, CertificateParams, CidrSubnet, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, GeneralSubtree, IsCa, Issuer, KeyPair, KeyUsagePurpose,
    NameConstraints, SanType, date_time_ymd,
};
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha2::{Digest, Sha256};

use crate::RemoteError;

/// The home network ranges the authority may vouch for.
const PRIVATE: [([u8; 4], u8); 3] = [
    ([10, 0, 0, 0], 8),
    ([172, 16, 0, 0], 12),
    ([192, 168, 0, 0], 16),
];
const DAY: u64 = 86_400;
/// How long an address's certificate lasts, in days. Apple accepts at most
/// 825 for a server certificate; a new one is issued when the launcher
/// restarts.
const LEAF_DAYS: u64 = 365;
/// How long the authority lasts, in years: a phone trusts it once.
const AUTHORITY_YEARS: u64 = 20;

/// Whether the TV may serve HTTPS on this address (its own on the home
/// network).
pub fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => PRIVATE.iter().any(|&(net, bits)| {
            let mask = u32::MAX << (32 - u32::from(bits));
            u32::from(ip) & mask == u32::from(Ipv4Addr::from(net))
        }),
        IpAddr::V6(_) => false,
    }
}

/// The TV's certificate authority.
pub struct Authority {
    key: KeyPair,
    certificate: CertificateDer<'static>,
    /// Certificates issued since the start, by address.
    issued: Mutex<HashMap<IpAddr, Arc<ServerConfig>>>,
    /// Seconds since 1970, for certificates' validity.
    now: u64,
}

impl std::fmt::Debug for Authority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Authority")
            .field("name", &self.name())
            .finish_non_exhaustive()
    }
}

impl Authority {
    /// A new authority with a fresh key; `now` is seconds since 1970.
    pub fn create(now: u64) -> Result<Self, RemoteError> {
        let key = KeyPair::generate().map_err(tls)?;
        let certificate = authority_params(&key, now)
            .self_signed(&key)
            .map_err(tls)?
            .der()
            .clone();
        Ok(Self {
            key,
            certificate,
            issued: Mutex::default(),
            now,
        })
    }

    /// The authority saved by [`Authority::key_der`] and
    /// [`Authority::certificate_der`].
    pub fn load(certificate: Vec<u8>, key: &[u8], now: u64) -> Result<Self, RemoteError> {
        let key = KeyPair::try_from(key).map_err(tls)?;
        Ok(Self {
            key,
            certificate: CertificateDer::from(certificate),
            issued: Mutex::default(),
            now,
        })
    }

    /// The certificate phones install to trust the TV (DER).
    pub fn certificate_der(&self) -> &[u8] {
        &self.certificate
    }

    /// The private key, for the launcher's private storage only (PKCS #8 DER).
    pub fn key_der(&self) -> Vec<u8> {
        self.key.serialize_der()
    }

    /// The code the TV shows while a phone sets up trust: the first six
    /// bytes of the certificate's SHA-256 fingerprint, as iOS and Android
    /// show it in the certificate's details ("C5 8B 5F 12 9A 0E"). The person
    /// compares it there, on the phone's own screen, before installing: a
    /// tampered page can't fake that, and 48 bits can't be matched by
    /// making certificates until one fits.
    pub fn check_code(&self) -> String {
        let digest = Sha256::digest(&self.certificate);
        digest[..6]
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The name phones show for the certificate: "RustTV" and a short
    /// fingerprint, so two TVs are told apart.
    pub fn name(&self) -> String {
        name(&self.key)
    }

    /// The TLS setup for connections to `ip`, issuing its certificate on
    /// first use. Refused for addresses outside the home network ranges.
    pub fn config(&self, ip: IpAddr) -> Result<Arc<ServerConfig>, RemoteError> {
        if !is_private(ip) {
            return Err(RemoteError::Invalid(
                "not a home network address".to_owned(),
            ));
        }
        let mut issued = self
            .issued
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(config) = issued.get(&ip) {
            return Ok(Arc::clone(config));
        }
        let config = Arc::new(self.server_config(ip)?);
        issued.insert(ip, Arc::clone(&config));
        Ok(config)
    }

    fn server_config(&self, ip: IpAddr) -> Result<ServerConfig, RemoteError> {
        let key = KeyPair::generate().map_err(tls)?;
        let mut params = CertificateParams::default();
        params.distinguished_name = DistinguishedName::new();
        params
            .distinguished_name
            .push(DnType::CommonName, ip.to_string());
        params.subject_alt_names = vec![SanType::IpAddress(ip)];
        params.is_ca = IsCa::ExplicitNoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        valid(
            &mut params,
            self.now.saturating_sub(DAY),
            self.now + LEAF_DAYS * DAY,
        );
        let issuer = Issuer::new(authority_params(&self.key, self.now), &self.key);
        let leaf = params.signed_by(&key, &issuer).map_err(tls)?;
        let chain = vec![leaf.der().clone(), self.certificate.clone()];
        let private = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(tls)?
            .with_no_client_auth()
            .with_single_cert(chain, private)
            .map_err(tls)
    }
}

/// The authority's name, made from its key.
fn name(key: &KeyPair) -> String {
    let digest = Sha256::digest(key.public_key_raw());
    format!("RustTV {}", crate::hex::encode(&digest[..3]).to_uppercase())
}

/// The authority's certificate fields. Rebuilt the same way from the key for
/// signing: only the name and key identifier must match the saved
/// certificate.
fn authority_params(key: &KeyPair, now: u64) -> CertificateParams {
    let mut params = CertificateParams::default();
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(DnType::CommonName, name(key));
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params.name_constraints = Some(NameConstraints {
        permitted_subtrees: PRIVATE
            .iter()
            .map(|&(net, bits)| GeneralSubtree::IpAddress(CidrSubnet::V4(net, mask(bits))))
            .collect(),
        excluded_subtrees: Vec::new(),
    });
    valid(
        &mut params,
        now.saturating_sub(DAY),
        now + AUTHORITY_YEARS * 365 * DAY,
    );
    params
}

fn mask(bits: u8) -> [u8; 4] {
    (u32::MAX << (32 - u32::from(bits))).to_be_bytes()
}

/// Makes the certificate valid from the day of `from` to the day of `until`
/// (seconds since 1970, UTC).
fn valid(params: &mut CertificateParams, from: u64, until: u64) {
    let (year, month, day) = civil(from);
    params.not_before = date_time_ymd(year, month, day);
    let (year, month, day) = civil(until);
    params.not_after = date_time_ymd(year, month, day);
}

/// Year, month and day of `seconds` since 1970 (UTC), by Howard Hinnant's
/// days-to-civil algorithm.
fn civil(seconds: u64) -> (i32, u8, u8) {
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    // Month 1..=12 and day 1..=31 by construction; years fit far beyond 2100.
    (
        i32::try_from(year).unwrap_or(i32::MAX),
        u8::try_from(month).unwrap_or(1),
        u8::try_from(day).unwrap_or(1),
    )
}

fn tls(error: impl std::fmt::Display) -> RemoteError {
    RemoteError::Invalid(format!("tls: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::pki_types::ServerName;
    use rustls::{ClientConfig, ClientConnection, RootCertStore, ServerConnection};

    /// Now: the TLS client checks validity against the real clock.
    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    /// A TLS 1.3 handshake in memory, by a client that trusts `root`, to
    /// `server` presented as `ip`.
    fn handshake(root: &[u8], server: Arc<ServerConfig>, ip: [u8; 4]) -> Result<(), rustls::Error> {
        let mut roots = RootCertStore::empty();
        roots.add(CertificateDer::from(root.to_vec())).unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let client = ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let name = ServerName::IpAddress(Ipv4Addr::from(ip).into());
        let mut client = ClientConnection::new(Arc::new(client), name).unwrap();
        let mut server = ServerConnection::new(server).unwrap();
        for _ in 0..10 {
            let mut bytes = Vec::new();
            client.write_tls(&mut bytes).unwrap();
            server.read_tls(&mut bytes.as_slice()).unwrap();
            server.process_new_packets()?;
            let mut bytes = Vec::new();
            server.write_tls(&mut bytes).unwrap();
            client.read_tls(&mut bytes.as_slice()).unwrap();
            client.process_new_packets()?;
            if !client.is_handshaking() && !server.is_handshaking() {
                return Ok(());
            }
        }
        panic!("the handshake didn't finish");
    }

    #[test]
    fn only_home_network_addresses_are_private() {
        for ip in ["10.0.0.1", "172.16.0.1", "172.31.255.254", "192.168.1.203"] {
            assert!(is_private(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "8.8.8.8",
            "172.32.0.1",
            "192.169.0.1",
            "127.0.0.1",
            "::1",
            "fd00::1",
        ] {
            assert!(!is_private(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn dates_are_right() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(1_792_958_103), (2026, 10, 25));
        assert_eq!(civil(951_782_400), (2000, 2, 29));
    }

    #[test]
    fn a_phone_that_trusts_the_tv_connects_to_its_address_only() {
        let authority = Authority::create(now()).unwrap();
        let config = authority.config(IpAddr::from([192, 168, 1, 203])).unwrap();
        handshake(
            authority.certificate_der(),
            Arc::clone(&config),
            [192, 168, 1, 203],
        )
        .unwrap();
        // The same certificate presented for another address fails.
        assert!(handshake(authority.certificate_der(), config, [192, 168, 1, 204]).is_err());
        // Issued once per address.
        let again = authority.config(IpAddr::from([192, 168, 1, 203])).unwrap();
        let first = authority.config(IpAddr::from([192, 168, 1, 203])).unwrap();
        assert!(Arc::ptr_eq(&again, &first));
        // A phone that trusts another TV doesn't connect.
        let other = Authority::create(now()).unwrap();
        let config = authority.config(IpAddr::from([192, 168, 1, 203])).unwrap();
        assert!(handshake(other.certificate_der(), config, [192, 168, 1, 203]).is_err());
    }

    #[test]
    fn the_authority_can_never_vouch_for_a_website() {
        let authority = Authority::create(now()).unwrap();
        assert!(authority.config(IpAddr::from([8, 8, 8, 8])).is_err());
        // Even a certificate it made for an internet address anyway is
        // refused by the phone: the name constraints say so.
        let config = Arc::new(authority.server_config(IpAddr::from([8, 8, 8, 8])).unwrap());
        assert!(handshake(authority.certificate_der(), config, [8, 8, 8, 8]).is_err());
    }

    #[test]
    fn a_saved_authority_signs_for_the_certificate_phones_trust() {
        let authority = Authority::create(now()).unwrap();
        let loaded = Authority::load(
            authority.certificate_der().to_vec(),
            &authority.key_der(),
            now(),
        )
        .unwrap();
        assert_eq!(loaded.name(), authority.name());
        assert!(loaded.name().starts_with("RustTV "));
        let config = loaded.config(IpAddr::from([10, 0, 0, 7])).unwrap();
        handshake(authority.certificate_der(), config, [10, 0, 0, 7]).unwrap();
        assert!(Authority::load(vec![1, 2, 3], b"not a key", now()).is_err());
        // The check code is the fingerprint's start, the same after loading.
        let code = authority.check_code();
        assert_eq!(code, loaded.check_code());
        let fingerprint = Sha256::digest(authority.certificate_der());
        assert_eq!(
            code.replace(' ', ""),
            crate::hex::encode(&fingerprint[..6]).to_uppercase()
        );
        assert_eq!(code.len(), 17);
    }
}
