//! Any bytes as what a phone sends after the upgrade: WebSocket frames read
//! with the listener's limits, each message's payload through the parser and
//! a session's `open`.
#![no_main]

use std::io::{Cursor, Read, Write};

use rusttv_remote::protocol::{MAX_PHONE_MESSAGE, parse_phone};
use rusttv_remote::{NONCE_LEN, Session, TAG_LEN};
use tungstenite::protocol::{Role, WebSocketConfig};
use tungstenite::{Message, WebSocket};

struct Wire {
    input: Cursor<Vec<u8>>,
}

impl Read for Wire {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.input.read(buffer)
    }
}

impl Write for Wire {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let limit = MAX_PHONE_MESSAGE + NONCE_LEN + TAG_LEN;
    let config = WebSocketConfig::default()
        .max_message_size(Some(limit))
        .max_frame_size(Some(limit));
    let wire = Wire {
        input: Cursor::new(data.to_vec()),
    };
    let mut socket = WebSocket::from_raw_socket(wire, Role::Server, Some(config));
    let mut session = Session::tv(&[0x55; 32]);
    for _ in 0..64 {
        match socket.read() {
            Ok(Message::Text(text)) => {
                let _ = parse_phone(text.as_bytes());
            }
            Ok(Message::Binary(frame)) => {
                assert!(frame.len() <= limit);
                let _ = session.open(&frame, MAX_PHONE_MESSAGE);
                let _ = parse_phone(&frame);
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
});
