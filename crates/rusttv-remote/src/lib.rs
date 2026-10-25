//! RustTV Remote: the phone remote's protocol, pairing, authentication and
//! session encryption (REMOTE.md).
//!
//! [`protocol`] is the fixed set of messages between a phone and the TV, with
//! their limits; [`auth`] derives the pairing proof, match code, per-phone
//! secrets and session keys with HMAC-SHA256, and keeps the paired phones
//! ([`Phones`]); [`Session`] seals and opens every message after
//! authentication with XChaCha20-Poly1305.
//!
//! [`Remote`] is the listener: the page and one WebSocket per phone on std
//! sockets, with every limit from REMOTE.md § 3.
//!
//! No clock or randomness of its own: callers pass nonces, secrets and times
//! in (on the TV, Kotlin's `SecureRandom`), so everything is testable and the
//! same bytes can be checked against the page's JavaScript. Used by the
//! launcher; depends on neither the launcher nor `rusttv-core`.

pub mod auth;
mod crypto;
mod hex;
pub mod protocol;
mod server;

pub use auth::{Phone, PhoneId, Phones};
pub use crypto::{Direction, NONCE_LEN, Session, TAG_LEN};
pub use server::{Clock, Event, MAX_CONNECTED, Options, PAIRING_TIME, PORT, Random, Remote};

/// The phone page, served at `/` ([`Options::page`]).
pub const PAGE: &str = include_str!("../page/index.html");

/// Why a remote operation failed. Every variant means "drop this message"
/// (or this connection); none of them is ever echoed to the phone.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RemoteError {
    /// A message or field is longer than its limit.
    #[error("too long: {0}")]
    TooLong(&'static str),
    /// A message, field or file breaks the format.
    #[error("invalid: {0}")]
    Invalid(String),
    /// A sealed message didn't open: wrong key, tampered, replayed or out of
    /// order.
    #[error("a message failed authentication")]
    Decrypt,
    /// The phones file was written by a newer RustTV (format version inside).
    #[error("the phones file was written by a newer RustTV (format {0})")]
    FormatNewer(u32),
    /// Another app already listens on the remote's port; the listener
    /// doesn't start (Settings says so).
    #[error("port {0} is already in use")]
    PortTaken(u16),
    /// A socket error, a timeout, or the phone went away.
    #[error("connection: {0}")]
    Io(String),
}
