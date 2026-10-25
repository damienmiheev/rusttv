//! Session encryption (REMOTE.md § 2): every message after authentication
//! is XChaCha20-Poly1305 under the connection's [`session_key`](crate::auth::session_key).
//!
//! A sealed frame is `nonce (24 bytes) || ciphertext || tag (16 bytes)`.
//! The nonce is random, from the caller. The associated data is the
//! direction byte and the message's number in that direction (u64, big
//! endian), which neither side sends: a message can't be replayed, dropped,
//! reordered or reflected back without failing to open. WebSocket over TCP
//! keeps order, so any gap means the connection is closed.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

use crate::RemoteError;

/// Length of a frame's nonce.
pub const NONCE_LEN: usize = 24;
/// Length of a frame's authentication tag.
pub const TAG_LEN: usize = 16;

/// Which way a message goes; part of every message's associated data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    PhoneToTv,
    TvToPhone,
}

impl Direction {
    fn byte(self) -> u8 {
        match self {
            Self::PhoneToTv => 1,
            Self::TvToPhone => 2,
        }
    }

    fn reverse(self) -> Self {
        match self {
            Self::PhoneToTv => Self::TvToPhone,
            Self::TvToPhone => Self::PhoneToTv,
        }
    }
}

/// One connection's encryption, for one side.
pub struct Session {
    cipher: XChaCha20Poly1305,
    sends: Direction,
    sent: u64,
    received: u64,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("sends", &self.sends)
            .field("sent", &self.sent)
            .field("received", &self.received)
            .finish_non_exhaustive()
    }
}

impl Session {
    /// The TV's side of a connection.
    pub fn tv(key: &[u8; 32]) -> Self {
        Self::new(key, Direction::TvToPhone)
    }

    /// The phone's side (the page does the same in JavaScript; this one is
    /// for tests and the desktop dev mode).
    pub fn phone(key: &[u8; 32]) -> Self {
        Self::new(key, Direction::PhoneToTv)
    }

    fn new(key: &[u8; 32], sends: Direction) -> Self {
        Self {
            cipher: XChaCha20Poly1305::new(&(*key).into()),
            sends,
            sent: 0,
            received: 0,
        }
    }

    /// Seals the next outgoing message with a fresh random `nonce`.
    pub fn seal(
        &mut self,
        plaintext: &[u8],
        nonce: [u8; NONCE_LEN],
    ) -> Result<Vec<u8>, RemoteError> {
        let aad = associated_data(self.sends, self.sent);
        let nonce = XNonce::from(nonce);
        let sealed = self
            .cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| RemoteError::TooLong("sealed message"))?;
        self.sent = self
            .sent
            .checked_add(1)
            .ok_or(RemoteError::TooLong("session"))?;
        let mut frame = Vec::with_capacity(NONCE_LEN + sealed.len());
        frame.extend_from_slice(&nonce);
        frame.extend_from_slice(&sealed);
        Ok(frame)
    }

    /// Opens the next incoming frame. Refused: a frame whose plaintext would
    /// be over `max` bytes (before decrypting), or one that fails
    /// authentication (wrong key, tampered, replayed, out of order, or sent
    /// in the other direction). After a refusal the connection must close.
    pub fn open(&mut self, frame: &[u8], max: usize) -> Result<Vec<u8>, RemoteError> {
        if frame.len() > NONCE_LEN + TAG_LEN + max {
            return Err(RemoteError::TooLong("sealed message"));
        }
        if frame.len() < NONCE_LEN + TAG_LEN {
            return Err(RemoteError::Decrypt);
        }
        let (nonce, sealed) = frame.split_at(NONCE_LEN);
        let nonce: [u8; NONCE_LEN] = nonce.try_into().map_err(|_| RemoteError::Decrypt)?;
        let aad = associated_data(self.sends.reverse(), self.received);
        let plaintext = self
            .cipher
            .decrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: sealed,
                    aad: &aad,
                },
            )
            .map_err(|_| RemoteError::Decrypt)?;
        self.received = self
            .received
            .checked_add(1)
            .ok_or(RemoteError::TooLong("session"))?;
        Ok(plaintext)
    }
}

fn associated_data(direction: Direction, number: u64) -> [u8; 9] {
    let mut aad = [0; 9];
    aad[0] = direction.byte();
    aad[1..].copy_from_slice(&number.to_be_bytes());
    aad
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const KEY: [u8; 32] = [0x55; 32];

    fn nonce(byte: u8) -> [u8; NONCE_LEN] {
        [byte; NONCE_LEN]
    }

    /// Pins the frame the page's JavaScript must produce. Computed
    /// independently in Python (HChaCha20 written out, checked against the
    /// XChaCha draft's § 2.2.1 vector, plus ChaCha20-Poly1305 from
    /// `cryptography`).
    #[test]
    fn test_vector() {
        let frame = Session::phone(&KEY)
            .seal(br#"{"type":"key","name":"ok","action":"press"}"#, nonce(1))
            .unwrap();
        assert_eq!(
            crate::hex::encode(&frame),
            "010101010101010101010101010101010101010101010101\
             e6b154a858658e5a5d1750775536fe971a08c1139d4e3b06d6fd6f104e473012\
             c9818219f884d938b3952f96b3912fd7be73a75ccfc98a9283641a"
        );
    }

    #[test]
    fn round_trips_both_ways() {
        let mut tv = Session::tv(&KEY);
        let mut phone = Session::phone(&KEY);
        for i in 0..3 {
            let frame = phone.seal(b"{\"type\":\"key\"}", nonce(i)).unwrap();
            assert_eq!(frame.len(), NONCE_LEN + 14 + TAG_LEN);
            assert_eq!(tv.open(&frame, 4096).unwrap(), b"{\"type\":\"key\"}");
            let frame = tv.seal(b"{\"type\":\"ok\"}", nonce(100 + i)).unwrap();
            assert_eq!(phone.open(&frame, 4096).unwrap(), b"{\"type\":\"ok\"}");
        }
    }

    #[test]
    fn refuses_replay_reorder_and_reflection() {
        let mut tv = Session::tv(&KEY);
        let mut phone = Session::phone(&KEY);
        let first = phone.seal(b"one", nonce(1)).unwrap();
        let second = phone.seal(b"two", nonce(2)).unwrap();
        // Out of order.
        assert!(matches!(tv.open(&second, 64), Err(RemoteError::Decrypt)));
        assert_eq!(tv.open(&first, 64).unwrap(), b"one");
        // Replayed.
        assert!(matches!(tv.open(&first, 64), Err(RemoteError::Decrypt)));
        // The TV's own message sent back to it.
        let mut tv2 = Session::tv(&KEY);
        let own = tv2.seal(b"ok", nonce(3)).unwrap();
        assert!(matches!(tv2.open(&own, 64), Err(RemoteError::Decrypt)));
    }

    #[test]
    fn refuses_wrong_key_tampering_and_sizes() {
        let mut phone = Session::phone(&KEY);
        let mut frame = phone.seal(b"hello", nonce(1)).unwrap();
        assert!(matches!(
            Session::tv(&[0x56; 32]).open(&frame, 64),
            Err(RemoteError::Decrypt)
        ));
        let last = frame.len() - 1;
        frame[last] ^= 1;
        assert!(matches!(
            Session::tv(&KEY).open(&frame, 64),
            Err(RemoteError::Decrypt)
        ));
        assert!(matches!(
            Session::tv(&KEY).open(&[0; 39], 64),
            Err(RemoteError::Decrypt)
        ));
        let big = Session::phone(&KEY).seal(&[b'a'; 65], nonce(2)).unwrap();
        assert!(matches!(
            Session::tv(&KEY).open(&big, 64),
            Err(RemoteError::TooLong(_))
        ));
    }

    proptest! {
        #[test]
        fn open_never_panics(frame in proptest::collection::vec(any::<u8>(), 0..200)) {
            prop_assert!(Session::tv(&KEY).open(&frame, 128).is_err());
        }
    }
}
