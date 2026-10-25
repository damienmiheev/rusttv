//! A stand-in TV for working on the phone page: the real listener with a fake
//! Home. It prints a pairing link, allows every pairing by itself, and moves
//! focus, opens folders and "apps" as the phone asks.
//!
//! `cargo run -p rusttv-remote --example demo [port]`, then open the printed
//! link on a phone on the same network (or in a desktop browser).

#![expect(
    clippy::unwrap_used,
    clippy::print_stderr,
    reason = "a developer tool: it prints and stops on unexpected failures"
)]

use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusttv_remote::protocol::{
    AppView, DragPhase, FolderView, Gesture, HomeMode, HomeView, Key, KeyAction, PhoneMessage,
    TextEdit, TextField, TileView, TvMessage,
};
use rusttv_remote::{Event, Options, PAGE, Phones, Remote};

fn random(bytes: &mut [u8]) {
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(bytes))
        .unwrap();
}

fn tile(id: &str, label: &str, folder: Option<&str>) -> TileView {
    TileView {
        id: id.into(),
        label: label.into(),
        banner: None,
        folder: folder.map(Into::into),
    }
}

fn main() {
    let port = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(8484);
    let remote = Remote::start(
        Options {
            port,
            // HTTPS on the next port, with an authority kept between runs
            // (as the TV keeps its own), so a phone trusts it once.
            secure_port: port + 1,
            authority: Some(Arc::new(authority())),
            page: PAGE,
            random: Arc::new(random),
            clock: Arc::new(now),
        },
        Phones::default(),
    )
    .unwrap();
    let mut secret = [0; 32];
    random(&mut secret);
    remote.open_pairing(secret);
    let mut opened = std::time::Instant::now();
    let hex: String = secret.iter().map(|b| format!("{b:02x}")).collect();
    eprintln!(
        "Pair: http://<this computer's address>:{}/#p={hex}",
        remote.address().port()
    );

    let mut home = HomeView {
        tiles: vec![
            tile("com.google.youtube/.Main", "YouTube", None),
            tile("com.netflix/.Main", "Netflix", None),
            tile("folder:1", "Streaming", None),
            tile("com.prime/.Main", "Prime Video", Some("folder:1")),
            tile("com.disney/.Main", "Disney+", Some("folder:1")),
            tile("com.spotify/.Main", "Spotify", None),
            tile("org.jellyfin/.Main", "Jellyfin", None),
            tile("com.browser/.Main", "Browser", None),
            tile("settings", "Settings", None),
            tile("hidden-apps", "Hidden apps", None),
            tile("about", "About", None),
        ],
        focused: Some("com.google.youtube/.Main".into()),
        ..HomeView::default()
    };
    let mut text = String::new();
    remote.push(&TvMessage::Home(home.clone())).unwrap();
    loop {
        let Some(event) = remote.wait_event(Duration::from_secs(1)) else {
            // Keep the link working until a phone pairs (a real QR code
            // lasts two minutes).
            if remote.phones().is_empty() && opened.elapsed() > Duration::from_secs(90) {
                remote.open_pairing(secret);
                opened = std::time::Instant::now();
            }
            continue;
        };
        eprintln!("{event:?}");
        let Event::Input { message, .. } = event else {
            if let Event::PairRequest { request, .. } = event {
                remote.answer_pairing(request, true);
            }
            continue;
        };
        let reply = match message {
            PhoneMessage::Tile {
                id,
                gesture: Gesture::Tap,
            } => {
                home.focused = Some(id.clone());
                if id.starts_with("folder:") {
                    home.folder = Some(FolderView {
                        id,
                        name: "Streaming".into(),
                        page: 0,
                    });
                    TvMessage::Home(home.clone())
                } else if id == "com.browser/.Main" || id == "com.netflix/.Main" {
                    TvMessage::App(AppView {
                        app: Some(id.split('/').next().unwrap_or_default().into()),
                        pointer: id == "com.browser/.Main",
                        text: (id == "com.netflix/.Main").then(|| TextField {
                            label: "Search".into(),
                            value: String::new(),
                        }),
                    })
                } else {
                    TvMessage::Home(home.clone())
                }
            }
            PhoneMessage::Tile {
                id,
                gesture: Gesture::LongPress,
            } => {
                home.focused = Some(id);
                home.mode = HomeMode::Text;
                home.text = Some(TextField {
                    label: "Folder name".into(),
                    value: text.clone(),
                });
                TvMessage::Home(home.clone())
            }
            PhoneMessage::Drag { phase, .. } => {
                home.mode = if matches!(phase, DragPhase::Start | DragPhase::Move) {
                    HomeMode::Moving
                } else {
                    HomeMode::Browse
                };
                TvMessage::Home(home.clone())
            }
            PhoneMessage::Text { edit } => {
                match edit {
                    TextEdit::Insert { text: more } => text.push_str(&more),
                    TextEdit::Delete { count } => {
                        for _ in 0..count {
                            text.pop();
                        }
                    }
                    TextEdit::Submit => {
                        home.mode = HomeMode::Browse;
                        home.text = None;
                    }
                }
                if let Some(field) = &mut home.text {
                    field.value = text.clone();
                }
                eprintln!("text: {text:?}");
                TvMessage::Home(home.clone())
            }
            PhoneMessage::Key {
                name: Key::Back | Key::Home,
                action: KeyAction::Press,
            } => {
                home.folder = None;
                home.mode = HomeMode::Browse;
                home.text = None;
                TvMessage::Home(home.clone())
            }
            _ => continue,
        };
        remote.push(&reply).unwrap();
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The demo's certificate authority, kept in the temp folder.
fn authority() -> rusttv_remote::tls::Authority {
    use rusttv_remote::tls::Authority;
    let dir = std::env::temp_dir();
    let (certificate, key) = (
        dir.join("rusttv-demo-authority.der"),
        dir.join("rusttv-demo-authority-key.der"),
    );
    if let (Ok(der), Ok(bytes)) = (std::fs::read(&certificate), std::fs::read(&key))
        && let Ok(authority) = Authority::load(der, &bytes, now())
    {
        return authority;
    }
    let authority = Authority::create(now()).unwrap();
    std::fs::write(&certificate, authority.certificate_der()).unwrap();
    std::fs::write(&key, authority.key_der()).unwrap();
    authority
}
