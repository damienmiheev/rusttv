//! The listener (REMOTE.md § 3): the page over HTTP/1.1 and one WebSocket
//! per phone, on std sockets, with every length, connection and rate capped.
//!
//! [`Remote::start`] binds the port and starts the accept thread; each
//! admitted connection gets a short-lived thread of its own (at most
//! [`MAX_CONNECTED`] phones plus one connection still authenticating). The
//! launcher talks to it only through [`Remote`]: it takes [`Event`]s and
//! pushes [`TvMessage`]s. Phones never reach the launcher's state any other
//! way.
//!
//! A connection reads with a 100 ms timeout so that it can also send the
//! latest view the launcher pushed; the cost of those wakeups is measured
//! with the rest (REMOTE.md § 7).

use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tungstenite::protocol::{Role, WebSocketConfig};
use tungstenite::{Message, WebSocket};

use crate::RemoteError;
use crate::auth::{self, Phone, PhoneId, PhoneKeys, Phones};
use crate::crypto::{NONCE_LEN, Session, TAG_LEN};
use crate::protocol::{MAX_PHONE_MESSAGE, PhoneMessage, TvMessage, parse_phone};

/// The fixed port (owner decision 9).
pub const PORT: u16 = 8484;
/// Most phones connected at once.
pub const MAX_CONNECTED: usize = 4;
/// Most connections still authenticating at once.
const MAX_PENDING: usize = 1;
/// Connection attempts allowed per minute, then a minute of silence.
const ATTEMPTS_PER_MINUTE: usize = 30;
const SILENCE: Duration = Duration::from_secs(60);
/// Most messages a phone may send per second (D-pad repeat is ~30).
const MAX_PER_SECOND: u32 = 200;
/// Largest HTTP request head (request line and headers).
const MAX_HEAD: usize = 4 * 1024;
const MAX_HEADERS: usize = 32;
/// Time to finish HTTP, the upgrade and authentication.
const HANDSHAKE_TIME: Duration = Duration::from_secs(10);
const WRITE_TIME: Duration = Duration::from_secs(5);
/// How often an idle authenticated connection wakes to send pushed views.
const POLL: Duration = Duration::from_millis(100);
/// How often it wakes for a while after the phone sent something: the
/// view that answers it goes out within this instead of up to [`POLL`]
/// (measured on the Philips: a phone input's answer took 115 ms).
const POLL_BUSY: Duration = Duration::from_millis(5);
/// How long the connection stays at [`POLL_BUSY`] after a phone message.
const BUSY_FOR: Duration = Duration::from_millis(500);
const PING_EVERY: Duration = Duration::from_secs(20);
/// Dropped after two missed pongs.
const PONG_DEADLINE: Duration = Duration::from_secs(45);
/// How long a pairing QR code works.
pub const PAIRING_TIME: Duration = Duration::from_secs(120);
/// Failed pairings in a row that close the pairing screen.
const MAX_PAIRING_FAILURES: u8 = 3;

/// Fills a buffer with secure random bytes (Kotlin's `SecureRandom` on the
/// TV).
pub type Random = Arc<dyn Fn(&mut [u8]) + Send + Sync>;
/// Unix seconds, for `paired_at` and `last_seen`.
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// How to start the listener.
pub struct Options {
    /// [`PORT`] on the TV; 0 in tests for any free port.
    pub port: u16,
    /// The page served at `/` ([`PAGE`](crate::PAGE)): its `<!--i18n.js-->`,
    /// `<!--crypto.js-->` and `<!--app.js-->` markers become inline scripts,
    /// allowed by their hashes in the page's Content-Security-Policy.
    pub page: &'static str,
    pub random: Random,
    pub clock: Clock,
}

impl std::fmt::Debug for Options {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Options")
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

/// What happened, for the launcher.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event {
    /// A phone proved it read the QR code: show "Allow *name*? Code NN" and
    /// answer with [`Remote::answer_pairing`].
    PairRequest {
        request: u64,
        name: String,
        code: u8,
    },
    /// The person allowed it: save the phones file and record the history
    /// entry.
    Paired(PhoneId),
    /// Too many failed pairings: close the pairing screen.
    PairingClosed,
    Connected(PhoneId),
    Disconnected(PhoneId),
    /// A checked message from an authenticated phone.
    Input {
        phone: PhoneId,
        message: PhoneMessage,
    },
}

/// The running listener. Dropping it stops the listener and closes every
/// connection.
pub struct Remote {
    shared: Arc<Shared>,
    events: Receiver<Event>,
    address: SocketAddr,
}

impl std::fmt::Debug for Remote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Remote")
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}

/// Sends messages to single phones ([`Remote::outlet`]); usable from any
/// thread, and harmless after the listener stops.
#[derive(Clone)]
pub struct Outlet {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for Outlet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Outlet").finish_non_exhaustive()
    }
}

impl Outlet {
    /// Sends `message` to `phone`'s connections, after the views and
    /// messages before it; nothing if it isn't connected.
    pub fn send_to(&self, phone: &PhoneId, message: &TvMessage) -> Result<(), RemoteError> {
        let bytes = message.to_bytes()?;
        let state = self.shared.lock();
        for connection in state.connections.iter().filter(|c| c.phone == *phone) {
            let _ = connection.outbox.send(Outgoing::Once(bytes.clone()));
        }
        Ok(())
    }
}

struct Shared {
    state: Mutex<State>,
    events: Mutex<Sender<Event>>,
    random: Random,
    clock: Clock,
    page: Page,
    stopped: AtomicBool,
    next_id: AtomicU64,
}

#[derive(Default)]
struct State {
    phones: Phones,
    pairing: Option<Pairing>,
    /// The latest view, sent to a phone as soon as it authenticates.
    view: Option<Arc<Vec<u8>>>,
    connections: Vec<Connection>,
    pending: usize,
    attempts: VecDeque<Instant>,
    silent_until: Option<Instant>,
}

struct Pairing {
    secret: [u8; 32],
    until: Instant,
    failures: u8,
    /// The connection waiting for the person's answer.
    waiting: Option<(u64, Sender<bool>)>,
}

struct Connection {
    id: u64,
    phone: PhoneId,
    outbox: Sender<Outgoing>,
}

enum Outgoing {
    /// The latest view: a newer one replaces it before it is sent.
    View(Arc<Vec<u8>>),
    /// A message for this phone only, sent in order (tile pictures).
    Once(Vec<u8>),
    Close,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while holding the lock leaves plain data behind; carry on
        // rather than take Home down.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn emit(&self, event: Event) {
        let sender = self.events.lock().unwrap_or_else(PoisonError::into_inner);
        // The launcher dropped its receiver: nobody is listening any more.
        let _ = sender.send(event);
    }

    fn random<const N: usize>(&self) -> [u8; N] {
        let mut bytes = [0; N];
        (self.random)(&mut bytes);
        bytes
    }

    fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }
}

impl Remote {
    /// Binds the port and starts listening. A taken port is
    /// [`RemoteError::PortTaken`]; the listener doesn't start.
    pub fn start(options: Options, phones: Phones) -> Result<Self, RemoteError> {
        let listener = TcpListener::bind(("0.0.0.0", options.port)).map_err(|error| {
            if error.kind() == ErrorKind::AddrInUse {
                RemoteError::PortTaken(options.port)
            } else {
                RemoteError::Io(error.to_string())
            }
        })?;
        let address = listener
            .local_addr()
            .map_err(|error| RemoteError::Io(error.to_string()))?;
        let (sender, events) = mpsc::channel();
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                phones,
                ..State::default()
            }),
            events: Mutex::new(sender),
            random: options.random,
            clock: options.clock,
            page: Page::build(options.page),
            stopped: AtomicBool::new(false),
            next_id: AtomicU64::new(1),
        });
        let accepting = Arc::clone(&shared);
        thread::Builder::new()
            .name("rusttv-remote".to_owned())
            .spawn(move || accept(&listener, &accepting))
            .map_err(|error| RemoteError::Io(error.to_string()))?;
        Ok(Self {
            shared,
            events,
            address,
        })
    }

    /// The bound address (the port matters in tests).
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// Hands the events to a thread that waits on them (the launcher
    /// forwards them to its UI thread, so nothing polls while nobody
    /// connects). Afterwards [`Remote::try_event`] and [`Remote::wait_event`]
    /// return nothing.
    pub fn take_events(&mut self) -> Receiver<Event> {
        std::mem::replace(&mut self.events, mpsc::channel().1)
    }

    /// The next event, if any.
    pub fn try_event(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    /// The next event, waiting at most `timeout`.
    pub fn wait_event(&self, timeout: Duration) -> Option<Event> {
        self.events.recv_timeout(timeout).ok()
    }

    /// Opens pairing with a fresh secret (the QR code's) for
    /// [`PAIRING_TIME`]; one phone can pair with it.
    pub fn open_pairing(&self, secret: [u8; 32]) {
        self.shared.lock().pairing = Some(Pairing {
            secret,
            until: Instant::now() + PAIRING_TIME,
            failures: 0,
            waiting: None,
        });
    }

    /// Closes pairing (the pairing screen closed); a phone waiting for an
    /// answer is refused.
    pub fn close_pairing(&self) {
        self.shared.lock().pairing = None;
    }

    /// The person's answer to [`Event::PairRequest`]: OK or Back on the TV.
    pub fn answer_pairing(&self, request: u64, allow: bool) {
        let mut state = self.shared.lock();
        if let Some(pairing) = &mut state.pairing
            && let Some((waiting, answer)) = pairing.waiting.take()
        {
            if waiting == request {
                let _ = answer.send(allow);
            } else {
                pairing.waiting = Some((waiting, answer));
            }
        }
    }

    /// Sends a view to every authenticated phone and keeps it for phones
    /// that connect later.
    pub fn push(&self, message: &TvMessage) -> Result<(), RemoteError> {
        let bytes = Arc::new(message.to_bytes()?);
        let mut state = self.shared.lock();
        state.view = Some(Arc::clone(&bytes));
        for connection in &state.connections {
            let _ = connection.outbox.send(Outgoing::View(Arc::clone(&bytes)));
        }
        Ok(())
    }

    /// Sends to single phones from another thread.
    pub fn outlet(&self) -> Outlet {
        Outlet {
            shared: Arc::clone(&self.shared),
        }
    }

    /// The paired phones now (with `last_seen`), for the phones file.
    pub fn phones(&self) -> Phones {
        self.shared.lock().phones.clone()
    }

    /// Unpairs a phone and closes its connection.
    pub fn revoke(&self, id: &PhoneId) -> Option<Phone> {
        let mut state = self.shared.lock();
        for connection in state.connections.iter().filter(|c| c.phone == *id) {
            let _ = connection.outbox.send(Outgoing::Close);
        }
        state.phones.remove(id)
    }

    /// Renames a phone.
    pub fn rename(&self, id: &PhoneId, name: &str) -> Result<(), RemoteError> {
        self.shared.lock().phones.rename(id, name)
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::Relaxed);
        let state = self.shared.lock();
        for connection in &state.connections {
            let _ = connection.outbox.send(Outgoing::Close);
        }
        drop(state);
        // Wake the accept thread so it sees the flag.
        let _ = TcpStream::connect_timeout(
            &SocketAddr::from(([127, 0, 0, 1], self.address.port())),
            Duration::from_millis(200),
        );
    }
}

fn accept(listener: &TcpListener, shared: &Arc<Shared>) {
    for stream in listener.incoming() {
        if shared.stopped() {
            return;
        }
        let Ok(stream) = stream else { continue };
        if !admit(shared) {
            continue;
        }
        let serving = Arc::clone(shared);
        let spawned = thread::Builder::new()
            .name("rusttv-remote-phone".to_owned())
            .spawn(move || {
                let mut guard = Admitted {
                    shared: &serving,
                    connection: None,
                    pending: true,
                };
                // Every error just ends this connection.
                let _ = serve(stream, &mut guard);
            });
        if spawned.is_err() {
            shared.lock().pending -= 1;
        }
    }
}

/// Rate and connection caps, before reading a byte.
fn admit(shared: &Shared) -> bool {
    let now = Instant::now();
    let mut state = shared.lock();
    if state.silent_until.is_some_and(|until| now < until) {
        return false;
    }
    state.silent_until = None;
    while state
        .attempts
        .front()
        .is_some_and(|&at| now.duration_since(at) > Duration::from_secs(60))
    {
        state.attempts.pop_front();
    }
    state.attempts.push_back(now);
    if state.attempts.len() > ATTEMPTS_PER_MINUTE {
        state.attempts.clear();
        state.silent_until = Some(now + SILENCE);
        return false;
    }
    if state.pending >= MAX_PENDING || state.connections.len() >= MAX_CONNECTED {
        return false;
    }
    state.pending += 1;
    true
}

/// A connection's bookkeeping, undone when its thread ends.
struct Admitted<'a> {
    shared: &'a Shared,
    connection: Option<(u64, PhoneId)>,
    pending: bool,
}

impl Admitted<'_> {
    fn release(&mut self) {
        if self.pending {
            self.shared.lock().pending -= 1;
            self.pending = false;
        }
    }

    fn authenticated(&mut self, phone: PhoneId) -> Receiver<Outgoing> {
        let (outbox, inbox) = mpsc::channel();
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let now = (self.shared.clock)();
        let mut state = self.shared.lock();
        state.pending -= 1;
        self.pending = false;
        state.phones.seen(&phone, now);
        state.connections.push(Connection { id, phone, outbox });
        drop(state);
        self.connection = Some((id, phone));
        self.shared.emit(Event::Connected(phone));
        inbox
    }
}

impl Drop for Admitted<'_> {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        if self.pending {
            state.pending -= 1;
        }
        if let Some((id, phone)) = self.connection {
            state.connections.retain(|connection| connection.id != id);
            drop(state);
            self.shared.emit(Event::Disconnected(phone));
        }
    }
}

type Socket = WebSocket<TcpStream>;

fn serve(mut stream: TcpStream, guard: &mut Admitted<'_>) -> Result<(), RemoteError> {
    let deadline = Instant::now() + HANDSHAKE_TIME;
    stream.set_nodelay(true).map_err(io)?;
    stream.set_write_timeout(Some(WRITE_TIME)).map_err(io)?;
    let (head, rest) = read_head(&mut stream, deadline)?;
    let Some(key) = route(&mut stream, &guard.shared.page, &head)? else {
        return Ok(());
    };
    let answer = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
        accept_key(&key)?
    );
    stream.write_all(answer.as_bytes()).map_err(io)?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_PHONE_MESSAGE + NONCE_LEN + TAG_LEN))
        .max_frame_size(Some(MAX_PHONE_MESSAGE + NONCE_LEN + TAG_LEN));
    let mut socket = WebSocket::from_partially_read(stream, rest, Role::Server, Some(config));
    let (phone, keys, challenge) = match authenticate(&mut socket, guard.shared, deadline) {
        Ok(authenticated) => authenticated,
        Err(error) => {
            // Free the pending slot before the phone hears "denied", so its
            // next try isn't refused by the cap.
            guard.release();
            deny(&mut socket);
            return Err(error);
        }
    };
    let inbox = guard.authenticated(phone);
    let mut session = Session::tv(&auth::session_key(&keys.key, &challenge));
    send_text(&mut socket, &TvMessage::Ok)?;
    talk(&mut socket, guard.shared, phone, &mut session, &inbox)
}

/// The request head (up to the blank line) and any bytes after it.
fn read_head(stream: &mut TcpStream, deadline: Instant) -> Result<(Vec<u8>, Vec<u8>), RemoteError> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0; 1024];
    loop {
        let left = deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or(RemoteError::Io("handshake timed out".to_owned()))?;
        stream.set_read_timeout(Some(left)).map_err(io)?;
        let read = stream.read(&mut chunk).map_err(io)?;
        if read == 0 {
            return Err(RemoteError::Io("closed".to_owned()));
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = buffer.split_off(end + 4);
            return Ok((buffer, rest));
        }
        if buffer.len() > MAX_HEAD {
            return Err(RemoteError::TooLong("request head"));
        }
    }
}

/// What a request asks for.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Route {
    /// The page, for this `Host`.
    Page(String),
    /// A valid WebSocket upgrade from the page itself, with its key.
    Upgrade(String),
    BadRequest,
    NotFound,
    NotAllowed,
}

/// Reads a request head (pure, so it can be fuzzed).
pub(crate) fn classify(head: &[u8]) -> Result<Route, RemoteError> {
    if head.len() > MAX_HEAD {
        return Err(RemoteError::TooLong("request head"));
    }
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut request = httparse::Request::new(&mut headers);
    let parsed = request
        .parse(head)
        .map_err(|error| RemoteError::Invalid(error.to_string()))?;
    if parsed.is_partial() {
        return Err(RemoteError::Invalid("partial request".to_owned()));
    }
    let header = |name: &str| {
        request
            .headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .and_then(|header| std::str::from_utf8(header.value).ok())
    };
    if request.method != Some("GET") {
        return Ok(Route::NotAllowed);
    }
    Ok(match request.path {
        Some("/") => match header("Host") {
            Some(host) if is_host(host) => Route::Page(host.to_owned()),
            _ => Route::BadRequest,
        },
        Some("/ws") => {
            let host = header("Host").unwrap_or_default();
            let upgrade = header("Upgrade").is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
            let connection = header("Connection").is_some_and(|v| {
                v.split(',')
                    .any(|part| part.trim().eq_ignore_ascii_case("upgrade"))
            });
            // Only the page itself may open the socket: another site's page
            // in the phone's browser has another Origin.
            let same_origin = header("Origin") == Some(format!("http://{host}").as_str());
            let version = header("Sec-WebSocket-Version") == Some("13");
            match header("Sec-WebSocket-Key") {
                Some(key) if upgrade && connection && same_origin && version && key.len() <= 64 => {
                    Route::Upgrade(key.to_owned())
                }
                _ => Route::BadRequest,
            }
        }
        _ => Route::NotFound,
    })
}

/// The page with its scripts inlined, and the hashes that allow exactly
/// those scripts.
struct Page {
    html: String,
    script_hashes: String,
}

impl Page {
    fn build(template: &str) -> Self {
        let mut html = template.to_owned();
        let mut hashes = Vec::new();
        for (marker, script) in [
            ("<!--i18n.js-->", crate::I18N_JS),
            ("<!--crypto.js-->", crate::CRYPTO_JS),
            ("<!--app.js-->", crate::APP_JS),
        ] {
            html = html.replace(marker, &format!("<script>{script}</script>"));
            hashes.push(format!(
                "'sha256-{}'",
                crate::hex::base64(&Sha256::digest(script.as_bytes()))
            ));
        }
        Self {
            html,
            script_hashes: hashes.join(" "),
        }
    }
}

/// A `Host` header we put into the page's policy: a name or address and a
/// port, nothing else.
fn is_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 255
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
}

/// Answers a page request itself; for a WebSocket upgrade returns the key.
fn route(stream: &mut TcpStream, page: &Page, head: &[u8]) -> Result<Option<String>, RemoteError> {
    let respond = |stream: &mut TcpStream, status: &str, kind: &str, policy: &str, body: &str| {
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\
             X-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\n\
             Content-Security-Policy: {policy}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).map_err(io)
    };
    let none = "default-src 'none'";
    match classify(head)? {
        Route::Upgrade(key) => return Ok(Some(key)),
        Route::Page(host) => {
            // Only the inlined scripts run; the page may only open its own
            // socket.
            let policy = format!(
                "default-src 'none'; script-src {}; style-src 'unsafe-inline'; img-src data: blob:; \
                 connect-src ws://{host}; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
                page.script_hashes
            );
            respond(
                stream,
                "200 OK",
                "text/html; charset=utf-8",
                &policy,
                &page.html,
            )?;
        }
        Route::BadRequest => respond(stream, "400 Bad Request", "text/plain", none, "")?,
        Route::NotFound => respond(stream, "404 Not Found", "text/plain", none, "")?,
        Route::NotAllowed => respond(stream, "405 Method Not Allowed", "text/plain", none, "")?,
    }
    Ok(None)
}

/// `Sec-WebSocket-Accept`: base64 of SHA-1 over the key and RFC 6455's GUID.
pub(crate) fn accept_key(key: &str) -> Result<String, RemoteError> {
    let mut input = Vec::with_capacity(key.len() + 36);
    input.extend_from_slice(key.as_bytes());
    input.extend_from_slice(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    let digest: [u8; 20] = sha1dc::digest(&input)
        .map_err(|_| RemoteError::Invalid("websocket key".to_owned()))?
        .into();
    Ok(crate::hex::base64(&digest))
}

/// Pairing (if the phone pairs), then the challenge. The phone, its keys and
/// the challenge the session key comes from; on an error the caller answers
/// "denied".
fn authenticate(
    socket: &mut Socket,
    shared: &Shared,
    deadline: Instant,
) -> Result<(PhoneId, PhoneKeys, [u8; 32]), RemoteError> {
    let mut message = read_text(socket, deadline)?;
    if let PhoneMessage::Pair {
        phone_nonce,
        name,
        proof,
    } = &message
    {
        let phone = pair(shared, &phone_nonce.0, name, &proof.0);
        let Some(phone) = phone else {
            return Err(RemoteError::Decrypt);
        };
        send_text(socket, &TvMessage::Paired { phone_id: phone })?;
        // The person took time to press OK; the phone now authenticates as
        // usual.
        message = read_text(socket, Instant::now() + HANDSHAKE_TIME)?;
    }
    let PhoneMessage::Hello { phone_id } = message else {
        return Err(RemoteError::Invalid("expected hello".to_owned()));
    };
    let Some(keys) = shared.lock().phones.get(&phone_id).map(|p| p.keys.clone()) else {
        return Err(RemoteError::Decrypt);
    };
    let challenge: [u8; 32] = shared.random();
    send_text(
        socket,
        &TvMessage::Challenge {
            nonce: crate::protocol::Bytes32(challenge),
        },
    )?;
    let PhoneMessage::Auth { hmac } =
        read_text(socket, deadline.max(Instant::now() + HANDSHAKE_TIME))?
    else {
        return Err(RemoteError::Invalid("expected auth".to_owned()));
    };
    if !auth::verify_auth(&keys.auth, &challenge, &phone_id, &hmac.0) {
        return Err(RemoteError::Decrypt);
    }
    Ok((phone_id, keys, challenge))
}

/// Checks a pairing request and waits for the person's answer; the new
/// phone's id if they allowed it.
fn pair(shared: &Shared, phone_nonce: &[u8; 32], name: &str, proof: &[u8; 32]) -> Option<PhoneId> {
    let request = shared.next_id.fetch_add(1, Ordering::Relaxed);
    let (answer, answered) = mpsc::channel();
    let (secret, until) = {
        let mut state = shared.lock();
        let pairing = state.pairing.as_mut()?;
        if Instant::now() >= pairing.until || pairing.waiting.is_some() {
            return None;
        }
        if !auth::verify_pairing_proof(&pairing.secret, phone_nonce, name, proof) {
            pairing.failures += 1;
            if pairing.failures >= MAX_PAIRING_FAILURES {
                state.pairing = None;
                drop(state);
                shared.emit(Event::PairingClosed);
            }
            return None;
        }
        pairing.waiting = Some((request, answer));
        (pairing.secret, pairing.until)
    };
    shared.emit(Event::PairRequest {
        request,
        name: name.to_owned(),
        code: auth::match_code(&secret, phone_nonce),
    });
    let left = until.saturating_duration_since(Instant::now());
    let allowed = matches!(answered.recv_timeout(left), Ok(true));
    let mut state = shared.lock();
    // One use: whatever the answer, this QR code is spent.
    if state
        .pairing
        .as_ref()
        .is_some_and(|pairing| pairing.secret == secret)
    {
        state.pairing = None;
    }
    if !allowed {
        return None;
    }
    let id = PhoneId(shared.random());
    let phone = Phone {
        id,
        name: name.to_owned(),
        keys: PhoneKeys::derive(&secret, phone_nonce),
        paired_at: (shared.clock)(),
        last_seen: None,
    };
    state.phones.add(phone).ok()?;
    drop(state);
    shared.emit(Event::Paired(id));
    Some(id)
}

/// The authenticated conversation: sealed messages in, pushed views out.
fn talk(
    socket: &mut Socket,
    shared: &Shared,
    phone: PhoneId,
    session: &mut Session,
    inbox: &Receiver<Outgoing>,
) -> Result<(), RemoteError> {
    socket.get_mut().set_read_timeout(Some(POLL)).map_err(io)?;
    let mut poll = POLL;
    let mut busy_until = Instant::now();
    let mut pending = shared.lock().view.clone();
    let mut once = VecDeque::new();
    let mut window = (Instant::now(), 0_u32);
    let mut last_ping = Instant::now();
    let mut last_pong = Instant::now();
    loop {
        loop {
            match inbox.try_recv() {
                Ok(Outgoing::View(view)) => pending = Some(view),
                Ok(Outgoing::Once(message)) => once.push_back(message),
                Ok(Outgoing::Close) | Err(mpsc::TryRecvError::Disconnected) => {
                    return close(socket);
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        if shared.stopped() {
            return close(socket);
        }
        if let Some(view) = pending.take() {
            let frame = session.seal(&view, shared.random())?;
            socket.send(Message::binary(frame)).map_err(ws)?;
        }
        while let Some(message) = once.pop_front() {
            let frame = session.seal(&message, shared.random())?;
            socket.send(Message::binary(frame)).map_err(ws)?;
        }
        let now = Instant::now();
        if now.duration_since(last_pong) > PONG_DEADLINE {
            return close(socket);
        }
        if now.duration_since(last_ping) >= PING_EVERY {
            socket.send(Message::Ping(Vec::new().into())).map_err(ws)?;
            last_ping = now;
        }
        let wanted = if now < busy_until { POLL_BUSY } else { POLL };
        if wanted != poll {
            socket
                .get_mut()
                .set_read_timeout(Some(wanted))
                .map_err(io)?;
            poll = wanted;
        }
        match socket.read() {
            Ok(Message::Binary(frame)) => {
                busy_until = Instant::now() + BUSY_FOR;
                if now.duration_since(window.0) >= Duration::from_secs(1) {
                    window = (now, 0);
                }
                window.1 += 1;
                if window.1 > MAX_PER_SECOND {
                    return close(socket);
                }
                let plaintext = session.open(&frame, MAX_PHONE_MESSAGE)?;
                let message = parse_phone(&plaintext)?;
                if matches!(
                    message,
                    PhoneMessage::Pair { .. }
                        | PhoneMessage::Hello { .. }
                        | PhoneMessage::Auth { .. }
                ) {
                    return close(socket);
                }
                shared.emit(Event::Input { phone, message });
            }
            Ok(Message::Pong(_)) => last_pong = Instant::now(),
            Ok(Message::Ping(_) | Message::Frame(_)) => {}
            // After authentication every message is sealed.
            Ok(Message::Text(_) | Message::Close(_)) => return close(socket),
            Err(tungstenite::Error::Io(error))
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(error) => return Err(ws(error)),
        }
    }
}

fn read_text(socket: &mut Socket, deadline: Instant) -> Result<PhoneMessage, RemoteError> {
    loop {
        let left = deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or(RemoteError::Io("handshake timed out".to_owned()))?;
        socket.get_mut().set_read_timeout(Some(left)).map_err(io)?;
        match socket.read() {
            Ok(Message::Text(text)) => return parse_phone(text.as_bytes()),
            Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {}
            Ok(Message::Binary(_) | Message::Close(_)) => {
                return Err(RemoteError::Invalid("expected a text message".to_owned()));
            }
            Err(tungstenite::Error::Io(error))
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(error) => return Err(ws(error)),
        }
    }
}

fn send_text(socket: &mut Socket, message: &TvMessage) -> Result<(), RemoteError> {
    let bytes = message.to_bytes()?;
    let text = String::from_utf8(bytes).map_err(|_| RemoteError::Invalid("utf-8".to_owned()))?;
    socket.send(Message::text(text)).map_err(ws)
}

fn deny(socket: &mut Socket) {
    let _ = send_text(socket, &TvMessage::Denied);
    let _ = close(socket);
}

fn close(socket: &mut Socket) -> Result<(), RemoteError> {
    let _ = socket.close(None);
    let _ = socket.flush();
    Ok(())
}

fn io(error: std::io::Error) -> RemoteError {
    RemoteError::Io(error.to_string())
}

fn ws(error: tungstenite::Error) -> RemoteError {
    RemoteError::Io(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_key_matches_rfc_6455() {
        // RFC 6455 § 1.3.
        assert_eq!(
            accept_key("dGhlIHNhbXBsZSBub25jZQ==").unwrap(),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    fn shared() -> Shared {
        let (sender, _) = mpsc::channel();
        Shared {
            state: Mutex::new(State::default()),
            events: Mutex::new(sender),
            random: Arc::new(|_: &mut [u8]| {}),
            clock: Arc::new(|| 0),
            page: Page::build(""),
            stopped: AtomicBool::new(false),
            next_id: AtomicU64::new(1),
        }
    }

    #[test]
    fn admits_one_pending_connection_at_a_time() {
        let shared = shared();
        assert!(admit(&shared));
        assert!(!admit(&shared));
        shared.lock().pending = 0;
        assert!(admit(&shared));
    }

    #[test]
    fn caps_connected_phones() {
        let shared = shared();
        for id in 0..MAX_CONNECTED {
            let (outbox, _) = mpsc::channel();
            shared.lock().connections.push(Connection {
                id: id as u64,
                phone: PhoneId([0; 16]),
                outbox,
            });
        }
        assert!(!admit(&shared));
    }

    #[test]
    fn goes_silent_after_too_many_attempts() {
        let shared = shared();
        for _ in 0..ATTEMPTS_PER_MINUTE {
            shared.lock().pending = 0;
            assert!(admit(&shared));
        }
        shared.lock().pending = 0;
        assert!(!admit(&shared));
        // Silent for a minute, even though nothing else is pending.
        assert!(!admit(&shared));
        assert!(shared.lock().silent_until.is_some());
    }
}
