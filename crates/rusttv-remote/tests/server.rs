//! The listener over real loopback sockets, with a client that does what the
//! page will do.

#![expect(
    clippy::unwrap_used,
    reason = "test helpers should panic on unexpected failures; clippy only exempts #[test] fns"
)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use rusttv_remote::auth::{self, PhoneKeys};
use rusttv_remote::protocol::{Bytes32, HomeView, Key, KeyAction, PhoneMessage, TvMessage};
use rusttv_remote::{Event, Options, PAGE, PhoneId, Phones, Remote, RemoteError, Session};
use tungstenite::protocol::Role;
use tungstenite::{Message, WebSocket};

const WAIT: Duration = Duration::from_secs(5);
const SECRET: [u8; 32] = [0x11; 32];
const NONCE: [u8; 32] = [0x22; 32];

/// A deterministic "random" source: every call fills with a new byte.
fn start(phones: Phones) -> Remote {
    let counter = Arc::new(AtomicU8::new(1));
    Remote::start(
        Options {
            port: 0,
            page: PAGE,
            random: Arc::new(move |bytes: &mut [u8]| {
                bytes.fill(counter.fetch_add(1, Ordering::Relaxed));
            }),
            clock: Arc::new(|| 1_792_938_447),
        },
        phones,
    )
    .unwrap()
}

fn local(remote: &Remote) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], remote.address().port()))
}

fn http(remote: &Remote, request: &str) -> String {
    let mut stream = TcpStream::connect(local(remote)).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    stream.set_read_timeout(Some(WAIT)).unwrap();
    let _ = stream.read_to_string(&mut response);
    response
}

type Client = WebSocket<TcpStream>;

fn connect(remote: &Remote) -> Client {
    let address = local(remote);
    let mut stream = TcpStream::connect(address).unwrap();
    stream.set_read_timeout(Some(WAIT)).unwrap();
    write!(
        stream,
        "GET /ws HTTP/1.1\r\nHost: {address}\r\nOrigin: http://{address}\r\nUpgrade: websocket\r\n\
         Connection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
    )
    .unwrap();
    let mut head = Vec::new();
    let mut byte = [0];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
    }
    let head = String::from_utf8(head).unwrap();
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    assert!(head.contains("Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo="));
    WebSocket::from_raw_socket(stream, Role::Client, None)
}

fn send(client: &mut Client, message: &PhoneMessage) {
    let json = serde_json::to_string(message).unwrap();
    client.send(Message::text(json)).unwrap();
}

fn receive(client: &mut Client) -> TvMessage {
    loop {
        match client.read().unwrap() {
            Message::Text(text) => return serde_json::from_str(text.as_str()).unwrap(),
            Message::Ping(_) | Message::Pong(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
}

fn receive_sealed(client: &mut Client, session: &mut Session) -> TvMessage {
    loop {
        match client.read().unwrap() {
            Message::Binary(frame) => {
                let plain = session.open(&frame, 64 * 1024).unwrap();
                return serde_json::from_slice(&plain).unwrap();
            }
            Message::Ping(_) | Message::Pong(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
}

/// Pairs a phone the way the page will, answering OK on the TV.
fn pair(remote: &Remote, client: &mut Client) -> PhoneId {
    remote.open_pairing(SECRET);
    send(
        client,
        &PhoneMessage::Pair {
            phone_nonce: Bytes32(NONCE),
            name: "Pixel 7".into(),
            proof: Bytes32(auth::pairing_proof(&SECRET, &NONCE, "Pixel 7")),
        },
    );
    let Some(Event::PairRequest {
        request,
        name,
        code,
    }) = remote.wait_event(WAIT)
    else {
        panic!("no pair request");
    };
    assert_eq!(name, "Pixel 7");
    assert_eq!(code, auth::match_code(&SECRET, &NONCE));
    remote.answer_pairing(request, true);
    let TvMessage::Paired { phone_id } = receive(client) else {
        panic!("not paired");
    };
    assert_eq!(remote.wait_event(WAIT), Some(Event::Paired(phone_id)));
    phone_id
}

/// Authenticates; the phone's side of the session.
fn authenticate(remote: &Remote, client: &mut Client, id: PhoneId) -> Session {
    let keys = PhoneKeys::derive(&SECRET, &NONCE);
    send(client, &PhoneMessage::Hello { phone_id: id });
    let TvMessage::Challenge { nonce } = receive(client) else {
        panic!("no challenge");
    };
    send(
        client,
        &PhoneMessage::Auth {
            hmac: Bytes32(auth::auth_response(&keys.auth, &nonce.0, &id)),
        },
    );
    assert_eq!(receive(client), TvMessage::Ok);
    assert_eq!(remote.wait_event(WAIT), Some(Event::Connected(id)));
    Session::phone(&auth::session_key(&keys.key, &nonce.0))
}

#[test]
fn serves_the_page_and_refuses_the_rest() {
    let remote = start(Phones::default());
    let page = http(&remote, "GET / HTTP/1.1\r\nHost: tv\r\n\r\n");
    assert!(page.starts_with("HTTP/1.1 200 OK"), "{page}");
    assert!(page.contains("Content-Security-Policy"));
    assert!(page.contains("RustTV Remote"));
    assert!(http(&remote, "GET /secret HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 404"));
    assert!(http(&remote, "POST / HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 405"));
    // Another site's page can't open the socket.
    let cross = http(
        &remote,
        "GET /ws HTTP/1.1\r\nHost: tv:8484\r\nOrigin: http://evil.example\r\nUpgrade: websocket\r\n\
         Connection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n",
    );
    assert!(cross.starts_with("HTTP/1.1 400"), "{cross}");
}

#[test]
fn pairs_authenticates_and_carries_sealed_messages() {
    let remote = start(Phones::default());
    let mut client = connect(&remote);
    let id = pair(&remote, &mut client);
    let mut session = authenticate(&remote, &mut client, id);
    assert_eq!(remote.phones().get(&id).unwrap().name, "Pixel 7");

    // A view the launcher pushes reaches the phone, sealed.
    let view = TvMessage::Home(HomeView {
        focused: Some("about".into()),
        ..HomeView::default()
    });
    remote.push(&view).unwrap();
    assert_eq!(receive_sealed(&mut client, &mut session), view);

    // A key from the phone reaches the launcher, checked.
    let key = PhoneMessage::Key {
        name: Key::Ok,
        action: KeyAction::Press,
    };
    let frame = session
        .seal(serde_json::to_string(&key).unwrap().as_bytes(), [7; 24])
        .unwrap();
    client.send(Message::binary(frame.clone())).unwrap();
    assert_eq!(
        remote.wait_event(WAIT),
        Some(Event::Input {
            phone: id,
            message: key
        })
    );

    // The same frame again is a replay: the connection closes.
    client.send(Message::binary(frame)).unwrap();
    assert_eq!(remote.wait_event(WAIT), Some(Event::Disconnected(id)));
}

#[test]
fn a_paired_phone_connects_again_and_gets_the_latest_view() {
    let remote = start(Phones::default());
    let mut first = connect(&remote);
    let id = pair(&remote, &mut first);
    let _ = authenticate(&remote, &mut first, id);
    drop(first);
    assert_eq!(remote.wait_event(WAIT), Some(Event::Disconnected(id)));

    let view = TvMessage::Home(HomeView::default());
    remote.push(&view).unwrap();
    let mut again = connect(&remote);
    let mut session = authenticate(&remote, &mut again, id);
    assert_eq!(receive_sealed(&mut again, &mut session), view);
}

#[test]
fn refuses_unknown_phones_wrong_answers_and_text_after_auth() {
    let remote = start(Phones::default());
    let mut stranger = connect(&remote);
    send(
        &mut stranger,
        &PhoneMessage::Hello {
            phone_id: PhoneId([9; 16]),
        },
    );
    assert_eq!(receive(&mut stranger), TvMessage::Denied);

    let mut client = connect(&remote);
    let id = pair(&remote, &mut client);
    send(&mut client, &PhoneMessage::Hello { phone_id: id });
    let TvMessage::Challenge { .. } = receive(&mut client) else {
        panic!("no challenge");
    };
    send(
        &mut client,
        &PhoneMessage::Auth {
            hmac: Bytes32([0; 32]),
        },
    );
    assert_eq!(receive(&mut client), TvMessage::Denied);

    let mut client = connect(&remote);
    let _ = authenticate(&remote, &mut client, id);
    // Unsealed text after authentication closes the connection.
    send(
        &mut client,
        &PhoneMessage::Key {
            name: Key::Ok,
            action: KeyAction::Press,
        },
    );
    assert_eq!(remote.wait_event(WAIT), Some(Event::Disconnected(id)));
}

#[test]
fn pairing_needs_the_secret_the_person_and_one_use() {
    let remote = start(Phones::default());
    // Pairing isn't open.
    let mut client = connect(&remote);
    send(
        &mut client,
        &PhoneMessage::Pair {
            phone_nonce: Bytes32(NONCE),
            name: "Pixel 7".into(),
            proof: Bytes32(auth::pairing_proof(&SECRET, &NONCE, "Pixel 7")),
        },
    );
    assert_eq!(receive(&mut client), TvMessage::Denied);

    // Back on the TV refuses, and the QR code is spent.
    remote.open_pairing(SECRET);
    let mut client = connect(&remote);
    let pair_message = PhoneMessage::Pair {
        phone_nonce: Bytes32(NONCE),
        name: "Pixel 7".into(),
        proof: Bytes32(auth::pairing_proof(&SECRET, &NONCE, "Pixel 7")),
    };
    send(&mut client, &pair_message);
    let Some(Event::PairRequest { request, .. }) = remote.wait_event(WAIT) else {
        panic!("no pair request");
    };
    remote.answer_pairing(request, false);
    assert_eq!(receive(&mut client), TvMessage::Denied);
    let mut client = connect(&remote);
    send(&mut client, &pair_message);
    assert_eq!(receive(&mut client), TvMessage::Denied);
    assert!(remote.phones().is_empty());

    // Three wrong proofs close the pairing screen.
    remote.open_pairing(SECRET);
    for _ in 0..3 {
        let mut client = connect(&remote);
        send(
            &mut client,
            &PhoneMessage::Pair {
                phone_nonce: Bytes32(NONCE),
                name: "Pixel 7".into(),
                proof: Bytes32([0; 32]),
            },
        );
        assert_eq!(receive(&mut client), TvMessage::Denied);
    }
    assert_eq!(remote.wait_event(WAIT), Some(Event::PairingClosed));
}

#[test]
fn revoking_closes_the_connection() {
    let remote = start(Phones::default());
    let mut client = connect(&remote);
    let id = pair(&remote, &mut client);
    let _ = authenticate(&remote, &mut client, id);
    assert!(remote.revoke(&id).is_some());
    assert_eq!(remote.wait_event(WAIT), Some(Event::Disconnected(id)));
    assert!(remote.phones().is_empty());
}

#[test]
fn a_taken_port_is_reported() {
    let first = start(Phones::default());
    let taken = Remote::start(
        Options {
            port: first.address().port(),
            page: PAGE,
            random: Arc::new(|_: &mut [u8]| {}),
            clock: Arc::new(|| 0),
        },
        Phones::default(),
    );
    assert!(matches!(taken, Err(RemoteError::PortTaken(_))));
}
