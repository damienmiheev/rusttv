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
use rusttv_remote::protocol::{
    Bytes32, HomeView, Key, KeyAction, PhoneMessage, Picture, PictureKind, TvMessage,
};
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
            secure_port: 0,
            authority: None,
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
    let mut stream = TcpStream::connect(local(&remote)).unwrap();
    stream.write_all(b"GET /icon.png HTTP/1.1\r\n\r\n").unwrap();
    let mut icon = Vec::new();
    stream.set_read_timeout(Some(WAIT)).unwrap();
    let _ = stream.read_to_end(&mut icon);
    assert!(icon.starts_with(b"HTTP/1.1 200 OK"));
    assert!(icon.ends_with(rusttv_remote::ICON), "the Home Screen icon");
    assert!(page.contains(r#"<link rel="apple-touch-icon" href="/icon.png">"#));
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
fn with_an_authority_the_page_offers_https_and_the_certificate() {
    let authority = Arc::new(rusttv_remote::tls::Authority::create(1_792_938_447).unwrap());
    let remote = Remote::start(
        Options {
            port: 0,
            secure_port: 0,
            authority: Some(Arc::clone(&authority)),
            page: PAGE,
            random: Arc::new(|_: &mut [u8]| {}),
            clock: Arc::new(|| 1_792_938_447),
        },
        Phones::default(),
    )
    .unwrap();
    let port = remote.secure_port().unwrap();
    let page = http(&remote, "GET / HTTP/1.1\r\nHost: 192.168.1.20:8484\r\n\r\n");
    assert!(page.contains(&format!(
        r#"<meta name="rusttv-secure-port" content="{port}">"#
    )));
    // The plain page may ask the HTTPS page whether the phone trusts the TV.
    assert!(page.contains(&format!(
        "connect-src ws://192.168.1.20:8484 https://192.168.1.20:{port};"
    )));
    // The certificate to install, as a download iOS offers as a profile.
    let mut stream = TcpStream::connect(local(&remote)).unwrap();
    stream.write_all(b"GET /tv.crt HTTP/1.1\r\n\r\n").unwrap();
    let mut response = Vec::new();
    stream.set_read_timeout(Some(WAIT)).unwrap();
    let _ = stream.read_to_end(&mut response);
    let split = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    let head = String::from_utf8_lossy(&response[..split]);
    assert!(
        head.contains("Content-Type: application/x-x509-ca-cert"),
        "{head}"
    );
    assert!(head.contains(&format!(r#"filename="{}.crt""#, authority.name())));
    assert_eq!(&response[split..], authority.certificate_der());
    // Without an authority there is neither.
    let plain = start(Phones::default());
    assert!(plain.secure_port().is_none());
    assert!(http(&plain, "GET /tv.crt HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 404"));
    let page = http(&plain, "GET / HTTP/1.1\r\nHost: tv\r\n\r\n");
    assert!(page.contains(r#"<meta name="rusttv-secure-port" content="">"#));
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
fn pictures_reach_only_their_phone_all_of_them_in_order() {
    let remote = start(Phones::default());
    let mut client = connect(&remote);
    let id = pair(&remote, &mut client);
    let mut session = authenticate(&remote, &mut client, id);
    let picture = |name: &str| {
        TvMessage::Banner(Picture {
            id: format!("{name}/.Main"),
            kind: PictureKind::Banner,
            webp: "UklGRg==".into(),
        })
    };
    let outlet = remote.outlet();
    // Unlike views, which only the latest of is sent, each one arrives.
    outlet
        .send_to(&PhoneId([9; 16]), &picture("other"))
        .unwrap();
    outlet.send_to(&id, &picture("a")).unwrap();
    outlet.send_to(&id, &picture("b")).unwrap();
    assert_eq!(receive_sealed(&mut client, &mut session), picture("a"));
    assert_eq!(receive_sealed(&mut client, &mut session), picture("b"));
    // Another phone's picture never came: the next message is this view.
    let view = TvMessage::Home(HomeView::default());
    remote.push(&view).unwrap();
    assert_eq!(receive_sealed(&mut client, &mut session), view);
}

#[test]
fn the_answer_to_a_phone_input_goes_out_without_waiting_for_the_idle_poll() {
    let remote = start(Phones::default());
    let mut client = connect(&remote);
    let id = pair(&remote, &mut client);
    let mut session = authenticate(&remote, &mut client, id);
    let key = PhoneMessage::Key {
        name: Key::Right,
        action: KeyAction::Press,
    };
    let mut slowest = Duration::ZERO;
    for round in 0..5_u8 {
        let frame = session
            .seal(serde_json::to_string(&key).unwrap().as_bytes(), [round; 24])
            .unwrap();
        client.send(Message::binary(frame)).unwrap();
        assert!(matches!(remote.wait_event(WAIT), Some(Event::Input { .. })));
        // The launcher answers with a view, as after a key.
        let view = TvMessage::Home(HomeView {
            focused: Some(format!("a{round}/.Main")),
            ..HomeView::default()
        });
        let sent = std::time::Instant::now();
        remote.push(&view).unwrap();
        assert_eq!(receive_sealed(&mut client, &mut session), view);
        slowest = slowest.max(sent.elapsed());
    }
    // Idle, a connection only looks for views every 100 ms.
    assert!(
        slowest < Duration::from_millis(60),
        "slowest answer {slowest:?}"
    );
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

/// Sends a sealed message and waits until the listener has read it: a key
/// after it reaches the launcher only once it has.
fn send_sealed_and_settle(
    remote: &Remote,
    client: &mut Client,
    session: &mut Session,
    id: PhoneId,
    message: &PhoneMessage,
) {
    let key = PhoneMessage::Key {
        name: Key::Back,
        action: KeyAction::Press,
    };
    for (n, message) in [message, &key].into_iter().enumerate() {
        let json = serde_json::to_string(message).unwrap();
        let frame = session.seal(json.as_bytes(), [n as u8 + 40; 24]).unwrap();
        client.send(Message::binary(frame)).unwrap();
    }
    assert_eq!(
        remote.wait_event(WAIT),
        Some(Event::Input {
            phone: id,
            message: key
        })
    );
}

/// A Home Screen icon trading a handover token: the keys it gets, or `None`
/// when the TV refuses.
fn redeem(
    client: &mut Client,
    id: PhoneId,
    token: &[u8; 32],
    nonce: &[u8; 32],
) -> Option<PhoneKeys> {
    send(
        client,
        &PhoneMessage::Redeem {
            phone_id: id,
            phone_nonce: Bytes32(*nonce),
            proof: Bytes32(auth::handover_proof(token, nonce, &id)),
        },
    );
    loop {
        match client.read().unwrap() {
            Message::Binary(frame) => {
                let mut session = Session::phone(&auth::handover_key(token, nonce));
                let plain = session.open(&frame, 1024).unwrap();
                let TvMessage::Keys { auth, key } = serde_json::from_slice(&plain).unwrap() else {
                    panic!("expected keys");
                };
                return Some(PhoneKeys {
                    auth: auth.0,
                    key: key.0,
                });
            }
            Message::Text(text) => {
                assert_eq!(
                    serde_json::from_str::<TvMessage>(text.as_str()).unwrap(),
                    TvMessage::Denied
                );
                return None;
            }
            Message::Ping(_) | Message::Pong(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[test]
fn a_home_screen_icon_trades_a_handover_token_once_for_the_keys() {
    let remote = start(Phones::default());
    let mut browser = connect(&remote);
    let id = pair(&remote, &mut browser);
    let mut session = authenticate(&remote, &mut browser, id);
    let token = [0x33; 32];
    let handover = PhoneMessage::Handover {
        token: Bytes32(token),
    };
    send_sealed_and_settle(&remote, &mut browser, &mut session, id, &handover);

    // A wrong token, or the right one for another phone, gets nothing.
    let mut stranger = connect(&remote);
    assert_eq!(redeem(&mut stranger, id, &[0x44; 32], &[0x55; 32]), None);
    let mut stranger = connect(&remote);
    assert_eq!(
        redeem(&mut stranger, PhoneId([9; 16]), &token, &[0x55; 32]),
        None
    );

    // The icon gets this phone's keys and signs in with them.
    let mut icon = connect(&remote);
    let keys = redeem(&mut icon, id, &token, &[0x66; 32]).unwrap();
    assert_eq!(keys, PhoneKeys::derive(&SECRET, &NONCE));
    let _ = authenticate(&remote, &mut icon, id);

    // One use.
    let mut again = connect(&remote);
    assert_eq!(redeem(&mut again, id, &token, &[0x77; 32]), None);

    // Removing the phone drops its tokens.
    let token = [0x88; 32];
    let handover = PhoneMessage::Handover {
        token: Bytes32(token),
    };
    send_sealed_and_settle(&remote, &mut browser, &mut session, id, &handover);
    assert!(remote.revoke(&id).is_some());
    let mut late = connect(&remote);
    assert_eq!(redeem(&mut late, id, &token, &[0x99; 32]), None);
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
            secure_port: 0,
            authority: None,
            page: PAGE,
            random: Arc::new(|_: &mut [u8]| {}),
            clock: Arc::new(|| 0),
        },
        Phones::default(),
    );
    assert!(matches!(taken, Err(RemoteError::PortTaken(_))));
}
