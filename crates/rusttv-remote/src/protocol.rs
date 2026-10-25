//! The messages between a phone and the TV (REMOTE.md § 4).
//!
//! JSON, one message per WebSocket frame, inside a [`Session`](crate::Session)
//! once the phone has authenticated. The set is fixed: a phone can only send
//! [`PhoneMessage`]s, every one is checked by [`parse_phone`] (size, shape,
//! names from fixed lists, lengths), and unknown fields are ignored. Nothing a
//! phone sends is ever echoed back.

use serde::{Deserialize, Serialize};

use crate::RemoteError;
use crate::auth::PhoneId;

/// Largest message a phone may send, in bytes (before encryption).
pub const MAX_PHONE_MESSAGE: usize = 4 * 1024;
/// Largest message the TV sends, in bytes (before encryption): a
/// [`HomeView`] of a few hundred tiles.
pub const MAX_TV_MESSAGE: usize = 64 * 1024;
/// Most bytes of text one [`TextEdit::Insert`] may carry.
pub const MAX_TEXT: usize = 1024;
/// Most characters in a phone's name.
pub const MAX_PHONE_NAME: usize = 40;

/// 32 bytes as 64 lowercase hex digits: nonces, proofs and HMACs.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Bytes32(pub [u8; 32]);

impl std::fmt::Debug for Bytes32 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Proofs and HMACs are only useful to an attacker in full; a prefix
        // is enough to tell messages apart in a log.
        write!(f, "Bytes32({}…)", crate::hex::encode(&self.0[..4]))
    }
}

impl TryFrom<String> for Bytes32 {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        crate::hex::decode(&text)
            .map(Self)
            .ok_or_else(|| "expected 64 lowercase hex digits".to_owned())
    }
}

impl From<Bytes32> for String {
    fn from(bytes: Bytes32) -> String {
        crate::hex::encode(&bytes.0)
    }
}

/// What a phone sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum PhoneMessage {
    /// Pairing, while the TV's pairing screen is open: the phone's fresh
    /// nonce, its name and the proof that it read the QR code
    /// ([`auth::pairing_proof`](crate::auth::pairing_proof)). Sent unencrypted.
    Pair {
        phone_nonce: Bytes32,
        name: String,
        proof: Bytes32,
    },
    /// The first message of a paired phone's connection. Sent unencrypted.
    Hello { phone_id: PhoneId },
    /// The answer to the TV's [`TvMessage::Challenge`]
    /// ([`auth::auth_response`](crate::auth::auth_response)). Sent unencrypted.
    Auth { hmac: Bytes32 },
    /// A remote key.
    Key { name: Key, action: KeyAction },
    /// An edit to the text field the TV asks for.
    Text { edit: TextEdit },
    /// Open an app, by component, from the list the TV sent.
    Open { app: String },
    /// A tap or long press on a tile of the Home mirror.
    Tile { id: String, gesture: Gesture },
    /// The person is setting up this phone to trust the TV: the TV shows
    /// its check code (REMOTE.md § 2 "Trusting the TV").
    Trust,
    /// A choice in the open long-press menu.
    Menu { choice: MenuChoice },
    /// The picture of an app's tile, by component, or of a built-in tile
    /// (`about`, `settings`, `hidden-apps`): answered with a
    /// [`TvMessage::Banner`] to this phone only.
    Banner { id: String },
    /// Moving a tile of the Home mirror: `over` is the tile it is over;
    /// `on` when the finger is over that tile's middle, where resting makes
    /// a folder (near its edges the tile only takes its place).
    Drag {
        id: String,
        over: Option<String>,
        phase: DragPhase,
        #[serde(default)]
        on: bool,
    },
    /// The touchpad: a movement in phone pixels, a click or a scroll.
    Pointer {
        dx: i16,
        dy: i16,
        action: PointerAction,
    },
}

/// The keys a phone can press: a fixed list, nothing else reaches the TV.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Ok,
    Back,
    Home,
    Menu,
    PlayPause,
    Rewind,
    FastForward,
    VolumeUp,
    VolumeDown,
    Mute,
    Power,
}

/// Press and release in one message, each half on its own (key repeat), or
/// a long press decided on the phone (the finger stayed still and lifted),
/// so a resting finger that then swipes never counts as one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyAction {
    Press,
    Down,
    Up,
    Long,
}

/// One edit to the text field the TV shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TextEdit {
    /// Insert at the cursor; at most [`MAX_TEXT`] bytes, no control
    /// characters.
    Insert { text: String },
    /// Delete this many characters before the cursor (1 to [`MAX_TEXT`]).
    Delete { count: u16 },
    /// The keyboard's Done / Enter.
    Submit,
}

/// A gesture on a tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gesture {
    Tap,
    LongPress,
}

/// Where a drag is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DragPhase {
    Start,
    Move,
    End,
    Cancel,
}

/// What a touchpad message does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerAction {
    Move,
    Click,
    Scroll,
}

/// What the TV sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TvMessage {
    /// The challenge after [`PhoneMessage::Hello`]. Sent unencrypted.
    Challenge { nonce: Bytes32 },
    /// Authenticated (or paired); every later message is sealed.
    Ok,
    /// Refused. Sent unencrypted, then the TV closes the connection; it never
    /// says why.
    Denied,
    /// The person pressed OK on the TV: the phone's new id.
    Paired { phone_id: PhoneId },
    /// RustTV is in front: the Home mirror.
    Home(HomeView),
    /// Another app is in front.
    App(AppView),
    /// An app tile's picture, to the phone that asked for it.
    Banner(Picture),
}

/// An app tile's picture, as the TV draws it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Picture {
    /// The app's component.
    pub id: String,
    pub kind: PictureKind,
    /// The picture as WebP, base64 ([`picture_data`]).
    pub webp: String,
}

/// What a [`Picture`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PictureKind {
    /// A 16:9 TV banner: fills the tile.
    Banner,
    /// A square icon: drawn centred, with the app's name.
    Icon,
}

/// Most WebP bytes in one [`Picture`]: with base64, the message stays under
/// [`MAX_TV_MESSAGE`]. A larger picture isn't sent; the phone keeps its
/// stand-in.
pub const MAX_PICTURE: usize = 44 * 1024;

/// WebP bytes as a [`Picture::webp`]; `None` over [`MAX_PICTURE`].
pub fn picture_data(webp: &[u8]) -> Option<String> {
    (webp.len() <= MAX_PICTURE).then(|| crate::hex::base64(webp))
}

/// The Home mirror (REMOTE.md § 4): sent whole after every change.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeView {
    /// Visible tiles in order. Apps inside folders are listed too, with
    /// their folder; hidden apps never are.
    pub tiles: Vec<TileView>,
    /// The focused tile.
    pub focused: Option<String>,
    /// The open folder.
    pub folder: Option<FolderView>,
    pub mode: HomeMode,
    /// The text field the TV asks for, in [`HomeMode::Text`].
    pub text: Option<TextField>,
    /// The long-press menu, in [`HomeMode::Menu`].
    #[serde(default)]
    pub menu: Option<MenuView>,
    /// The hidden apps, in [`HomeMode::Hidden`] (empty otherwise); the
    /// focused one is [`HomeView::focused`].
    #[serde(default)]
    pub hidden: Vec<TileView>,
}

/// The long-press menu of the focused tile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuView {
    pub items: Vec<MenuChoice>,
    pub selected: u8,
}

/// An entry of the long-press menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MenuChoice {
    /// Pick the tile up to move it.
    Move,
    /// Hide the app from Home.
    Hide,
}

/// A tile of the Home mirror.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TileView {
    /// Tile name: a component, `folder:<n>`, `about`, `settings` or
    /// `hidden-apps`.
    pub id: String,
    pub label: String,
    /// The banner's version (changes when the app updates), if it has one.
    pub banner: Option<u32>,
    /// The folder the tile is in.
    pub folder: Option<String>,
}

/// The open folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderView {
    pub id: String,
    pub name: String,
    pub page: u16,
}

/// What Home is doing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HomeMode {
    #[default]
    Browse,
    Menu,
    Moving,
    Hidden,
    Settings,
    /// The review/approval screen: the TV ignores phone input.
    Review,
    Text,
}

/// While another app is in front.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppView {
    /// The app's package, when the TV knows it.
    pub app: Option<String>,
    /// The touchpad moves a pointer (a listed app with the pointer on);
    /// otherwise swipes are arrow keys.
    pub pointer: bool,
    /// A focused text field, when the TV reports one.
    pub text: Option<TextField>,
}

/// A text field the TV asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextField {
    pub label: String,
    pub value: String,
}

/// Parses and checks one message from a phone. Anything over
/// [`MAX_PHONE_MESSAGE`], not in the fixed set, or with a field out of shape
/// is refused.
pub fn parse_phone(bytes: &[u8]) -> Result<PhoneMessage, RemoteError> {
    if bytes.len() > MAX_PHONE_MESSAGE {
        return Err(RemoteError::TooLong("phone message"));
    }
    let message: PhoneMessage =
        serde_json::from_slice(bytes).map_err(|error| RemoteError::Invalid(error.to_string()))?;
    check_phone(&message)?;
    Ok(message)
}

fn check_phone(message: &PhoneMessage) -> Result<(), RemoteError> {
    let invalid = |what: &str| Err(RemoteError::Invalid(what.to_owned()));
    match message {
        PhoneMessage::Pair { name, .. } if !is_phone_name(name) => invalid("phone name"),
        PhoneMessage::Text {
            edit: TextEdit::Insert { text },
        } => {
            if text.len() > MAX_TEXT {
                Err(RemoteError::TooLong("text"))
            } else if text.is_empty() || text.chars().any(char::is_control) {
                invalid("text")
            } else {
                Ok(())
            }
        }
        PhoneMessage::Text {
            edit: TextEdit::Delete { count },
        } if *count == 0 || usize::from(*count) > MAX_TEXT => invalid("delete count"),
        PhoneMessage::Open { app } if !is_component(app) => invalid("app"),
        PhoneMessage::Banner { id } if !is_component(id) && !is_builtin(id) => invalid("tile"),
        PhoneMessage::Tile { id, .. } if !is_tile_id(id) => invalid("tile"),
        PhoneMessage::Drag { id, over, .. }
            if !is_tile_id(id) || over.as_deref().is_some_and(|over| !is_tile_id(over)) =>
        {
            invalid("tile")
        }
        _ => Ok(()),
    }
}

impl TvMessage {
    /// The message as JSON; refused over [`MAX_TV_MESSAGE`].
    pub fn to_bytes(&self) -> Result<Vec<u8>, RemoteError> {
        let bytes =
            serde_json::to_vec(self).map_err(|error| RemoteError::Invalid(error.to_string()))?;
        if bytes.len() > MAX_TV_MESSAGE {
            return Err(RemoteError::TooLong("TV message"));
        }
        Ok(bytes)
    }
}

/// A phone's name as RustTV keeps it: control characters and bidi marks
/// removed, trimmed, 1 to [`MAX_PHONE_NAME`] characters. `None` if nothing
/// is left or it is too long.
pub fn clean_phone_name(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .filter(|&c| !c.is_control() && !is_bidi_mark(c))
        .collect();
    let trimmed = cleaned.trim();
    let count = trimmed.chars().count();
    (1..=MAX_PHONE_NAME)
        .contains(&count)
        .then(|| trimmed.to_owned())
}

/// Whether `name` is already a clean phone name ([`clean_phone_name`]).
pub fn is_phone_name(name: &str) -> bool {
    clean_phone_name(name).as_deref() == Some(name)
}

fn is_bidi_mark(c: char) -> bool {
    matches!(
        c,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// A component, `<package>/<class>`, by its characters: it only selects from
/// the TV's own list, so the shape check is a limit, not the safety.
fn is_component(name: &str) -> bool {
    // Dotted Java names: each segment starts with a letter or `_`.
    let dotted = |text: &str, extra: char| {
        text.len() <= 255
            && text.split('.').all(|segment| {
                segment.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                    && segment
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == extra)
            })
    };
    name.split_once('/').is_some_and(|(package, class)| {
        // A class may be relative to the package (`.Main`).
        dotted(package, '_') && dotted(class.strip_prefix('.').unwrap_or(class), '$')
    })
}

/// A tile id: `about`, `settings`, `hidden-apps`, `folder:<number>` or a
/// component.
/// One of the launcher's own tiles.
fn is_builtin(id: &str) -> bool {
    matches!(id, "about" | "settings" | "hidden-apps")
}

fn is_tile_id(id: &str) -> bool {
    is_builtin(id)
        || id
            .strip_prefix("folder:")
            .is_some_and(|number| !number.starts_with('+') && number.parse::<u32>().is_ok())
        || is_component(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn parse(json: &str) -> Result<PhoneMessage, RemoteError> {
        parse_phone(json.as_bytes())
    }

    #[test]
    fn parses_each_phone_message() {
        assert_eq!(
            parse(r#"{"type":"key","name":"play_pause","action":"press"}"#).unwrap(),
            PhoneMessage::Key {
                name: Key::PlayPause,
                action: KeyAction::Press
            }
        );
        assert_eq!(
            parse(r#"{"type":"text","edit":{"op":"insert","text":"Stream"}}"#).unwrap(),
            PhoneMessage::Text {
                edit: TextEdit::Insert {
                    text: "Stream".into()
                }
            }
        );
        assert_eq!(
            parse(r#"{"type":"text","edit":{"op":"submit"}}"#).unwrap(),
            PhoneMessage::Text {
                edit: TextEdit::Submit
            }
        );
        assert!(matches!(
            parse(r#"{"type":"open","app":"com.example.tv/com.example.Main$Tv"}"#),
            Ok(PhoneMessage::Open { .. })
        ));
        assert!(matches!(
            parse(r#"{"type":"drag","id":"folder:3","over":"settings","phase":"move"}"#),
            Ok(PhoneMessage::Drag { .. })
        ));
        assert!(matches!(
            parse(r#"{"type":"pointer","dx":-12,"dy":40,"action":"move"}"#),
            Ok(PhoneMessage::Pointer {
                dx: -12,
                dy: 40,
                ..
            })
        ));
        let hex = "ab".repeat(32);
        let pair = format!(
            r#"{{"type":"pair","phone_nonce":"{hex}","name":"iPhone (Safari)","proof":"{hex}"}}"#
        );
        assert!(matches!(parse(&pair), Ok(PhoneMessage::Pair { .. })));
    }

    #[test]
    fn ignores_unknown_fields_but_not_unknown_messages() {
        assert!(parse(r#"{"type":"key","name":"ok","action":"press","extra":1}"#).is_ok());
        assert!(parse(r#"{"type":"key","name":"ok","action":"long"}"#).is_ok());
        assert!(parse(r#"{"type":"shell","command":"reboot"}"#).is_err());
        assert!(parse(r#"{"type":"key","name":"settings","action":"press"}"#).is_err());
    }

    #[test]
    fn refuses_bad_fields() {
        let long = "a".repeat(MAX_TEXT + 1);
        assert!(matches!(
            parse(&format!(
                r#"{{"type":"text","edit":{{"op":"insert","text":"{long}"}}}}"#
            )),
            Err(RemoteError::TooLong("text"))
        ));
        assert!(parse(r#"{"type":"text","edit":{"op":"insert","text":"a\nb"}}"#).is_err());
        assert!(parse(r#"{"type":"text","edit":{"op":"insert","text":""}}"#).is_err());
        assert!(parse(r#"{"type":"text","edit":{"op":"delete","count":0}}"#).is_err());
        assert!(parse(r#"{"type":"open","app":"../etc"}"#).is_err());
        assert!(parse(r#"{"type":"open","app":"com..example/.Main"}"#).is_err());
        assert!(parse(r#"{"type":"open","app":"com.example/.Main"}"#).is_ok());
        assert!(parse(r#"{"type":"open","app":"com.example; rm -rf /"}"#).is_err());
        assert!(parse(r#"{"type":"tile","id":"folder:+1","gesture":"tap"}"#).is_err());
        assert!(parse(r#"{"type":"pointer","dx":40000,"dy":0,"action":"move"}"#).is_err());
        let hex = "ab".repeat(32);
        let bidi = serde_json::json!({
            "type": "pair", "phone_nonce": hex, "name": "a\u{202E}b", "proof": hex,
        })
        .to_string();
        assert!(parse(&bidi).is_err());
        assert!(parse(r#"{"type":"auth","hmac":"ABCD"}"#).is_err());
    }

    #[test]
    fn refuses_oversized_messages_before_parsing() {
        let padded = format!(
            r#"{{"type":"key","name":"ok","action":"press","pad":"{}"}}"#,
            " ".repeat(MAX_PHONE_MESSAGE)
        );
        assert!(matches!(
            parse(&padded),
            Err(RemoteError::TooLong("phone message"))
        ));
    }

    #[test]
    fn cleans_phone_names() {
        assert_eq!(
            clean_phone_name("  iPhone (Safari)\n").as_deref(),
            Some("iPhone (Safari)")
        );
        assert_eq!(
            clean_phone_name("\u{202E}Pixel 7\u{2069}").as_deref(),
            Some("Pixel 7")
        );
        assert_eq!(clean_phone_name("  \u{200E} "), None);
        assert!(clean_phone_name(&"é".repeat(MAX_PHONE_NAME)).is_some());
        assert_eq!(clean_phone_name(&"é".repeat(MAX_PHONE_NAME + 1)), None);
        assert!(is_phone_name("Pixel 7"));
        assert!(!is_phone_name(" Pixel 7"));
    }

    #[test]
    fn tv_messages_serialize_with_their_type() {
        let home = TvMessage::Home(HomeView {
            tiles: vec![TileView {
                id: "about".into(),
                label: "About".into(),
                banner: Some(1),
                folder: None,
            }],
            focused: Some("about".into()),
            ..HomeView::default()
        });
        let json = String::from_utf8(home.to_bytes().unwrap()).unwrap();
        assert!(json.starts_with(r#"{"type":"home","tiles":[{"id":"about""#));
        assert!(json.contains(r#""mode":"browse""#));
        assert_eq!(
            String::from_utf8(TvMessage::Ok.to_bytes().unwrap()).unwrap(),
            r#"{"type":"ok"}"#
        );
    }

    #[test]
    fn refuses_oversized_tv_messages() {
        let tile = TileView {
            id: "about".into(),
            label: "x".repeat(1000),
            banner: None,
            folder: None,
        };
        let home = TvMessage::Home(HomeView {
            tiles: vec![tile; 80],
            ..HomeView::default()
        });
        assert!(matches!(
            home.to_bytes(),
            Err(RemoteError::TooLong("TV message"))
        ));
    }

    proptest! {
        #[test]
        fn parse_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..6000)) {
            let _ = parse_phone(&bytes);
        }

        #[test]
        fn parse_never_panics_on_near_json(
            kind in prop::sample::select(vec!["key", "text", "open", "tile", "drag", "pointer", "pair", "hello", "auth"]),
            field in "[a-z_]{0,8}",
            value in ".{0,64}",
        ) {
            let json = format!(r#"{{"type":"{kind}","{field}":{}}}"#, serde_json::Value::String(value));
            let _ = parse_phone(json.as_bytes());
        }

        #[test]
        fn accepted_text_is_within_limits(text in ".{0,1200}") {
            let json = serde_json::json!({"type": "text", "edit": {"op": "insert", "text": text}}).to_string();
            if let Ok(PhoneMessage::Text { edit: TextEdit::Insert { text } }) = parse_phone(json.as_bytes()) {
                prop_assert!(!text.is_empty() && text.len() <= MAX_TEXT);
                prop_assert!(!text.chars().any(char::is_control));
            }
        }
    }

    #[test]
    fn a_phone_asks_for_pictures_of_apps_and_built_in_tiles_only() {
        let ask = |id: &str| parse_phone(format!(r#"{{"type":"banner","id":"{id}"}}"#).as_bytes());
        assert_eq!(
            ask("com.example/.Main").unwrap(),
            PhoneMessage::Banner {
                id: "com.example/.Main".into()
            }
        );
        assert!(ask("folder:1").is_err());
        assert!(ask("about").is_ok(), "the built-in tiles have pictures too");
        assert!(ask("../etc/passwd").is_err());
    }

    #[test]
    fn pictures_fit_a_message_or_are_not_sent() {
        assert_eq!(picture_data(b"RIFF").as_deref(), Some("UklGRg=="));
        assert_eq!(picture_data(&vec![0; MAX_PICTURE + 1]), None);
        let largest = TvMessage::Banner(Picture {
            id: "a".repeat(255),
            kind: PictureKind::Icon,
            webp: picture_data(&vec![0; MAX_PICTURE]).unwrap(),
        });
        assert!(largest.to_bytes().is_ok(), "the largest picture still fits");
    }
}
