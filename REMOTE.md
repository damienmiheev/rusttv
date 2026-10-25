# RustTV Remote — design

Status (2026-10-25): **planned**, nothing built. The owner approved this
design on 2026-10-25 (§ Owner decisions), and the phone page's look was
reviewed as a mock. It ships as **0.3.0**, with phases 1 and 2 together.
It replaces ROADMAP.md § Parked ideas → "Phone remote and second screen".

RustTV Remote turns a phone into the TV's remote, keyboard and touchpad, with
nothing to install on the phone. The launcher serves a web page on the home
network. The phone opens it once from a QR code on the TV and can then add it
to its home screen.

## Owner decisions (2026-10-25)

1. **v1 is two parts.** Inside RustTV, the phone mirrors Home. In other apps,
   control goes through the TV's own **Android TV Remote Service**. Not in
   v1: a RustTV input method, per-app layouts, media sessions, add-on
   layouts.
2. **Where the listener runs:** in Home's process first, and measured. It
   moves to a separate process only if the budgets (§ 7) or a fuzzing round
   say so.
3. **Remote Service client:** in Kotlin, on the platform's TLS, so the APK
   doesn't grow. This goes past D3's ~300-line guideline for the shim
   (ARCHITECTURE D24).
4. **Authentication over plain HTTP:** challenge-response with HMAC-SHA256.
   After pairing, the long-term secret never crosses the network.
5. **Pairing a phone:** the TV asks "Allow *this phone*?" and shows a 2-digit
   match code, which the phone shows too. OK on the TV remote pairs, and the
   pairing is recorded as a `tv` entry in the history. It can be revoked on
   the RustTV Remote page.
6. **Address:** a fixed IP, with no mDNS. If the address changes, scan the QR
   again. Settings recommends a DHCP reservation in the router.
7. **New crates:**
   - `hmac` (with `sha2`, already in the workspace);
   - `tungstenite` and `httparse`;
   - `qrcode` without its default features;
   - `chacha20poly1305`.

   Each gets a size line in its pull request.
8. **The session is encrypted** with XChaCha20-Poly1305. Typed text never
   crosses the network in clear.
9. **Port 8484, fixed.** If the port is taken, the listener doesn't start and
   Settings says so.
10. **The phone mirrors Home in full:** the same tiles with banners, order,
    folders and focus.
    - A tap opens an app; a long press opens the menu.
    - Dragging moves an app. Hiding apps and folders (make, rename, apps in
      and out) also work from the phone.
    - All of it goes through the same `Home` model, so the history records
      these as TV changes.
    - It works both ways: the TV and the phone show the same mode at the same
      time.
11. **Phases 1 and 2 ship together** as 0.3.0. No release has a page that
    can't help while another app is in front.
12. **Design first:** the phone page was mocked up (Claude design canvas) and
    reviewed by the owner before any HTML.
13. **While another app is in front:**
    - a touchpad, Home, Play/Pause, volume and power;
    - the keyboard whenever the TV reports a focused text field.

    Per-app views are phase 4 and need research first.
14. **The touchpad and the pointer:**
    - Home on the phone is only the mirror; RustTV's own screens have no
      pointer.
    - In browsers (and apps the person adds in Settings), the touchpad moves
      a **pointer on the TV**: one finger moves it, a tap clicks, two fingers
      scroll.
    - Everywhere else, swipes become arrow keys and a tap is OK.
    - The pointer needs a **narrow accessibility service**
      (§ 5 The pointer), off until the person switches it on.
15. **Name:** "RustTV Remote", used for its Home tile, its page's title and
    the home-screen icon.
16. **The phone's name** is typed on the phone during pairing, pre-filled
    from what the browser reports (for example "iPhone (Safari)"). It is 1–40
    characters, with control characters and bidi marks removed (the same rule
    as folder names). It can be changed later on the RustTV Remote page.
17. **Two phones share one view:** every phone and the TV show the same focus
    and mode. The last action wins, as with two remotes.

## 1. The page

- **One response:** `page/index.html`, `page/crypto.js` and `page/app.js`,
  vanilla HTML, CSS and JS with no build step and no CDN, compiled in with
  `include_str!`. The listener inlines both scripts into the page and serves
  it at `http://<tv-ip>:8484/` with a Content-Security-Policy that allows
  only those two scripts (by their SHA-256), inline styles, `data:`/`blob:`
  images and a socket to the same host; nothing else loads.
- **As built (2026-10-25):** pairing, connection, the Home mirror (tap, hold,
  hold and drag), an open folder, move mode, typing, and the touchpad
  (pointer and arrows) work against the real listener
  (`cargo run -p rusttv-remote --example demo`, a stand-in TV with a fake
  Home), and on the Philips TV. The page speaks the phone's language (the
  first of `navigator.languages` among the launcher's 20, else English;
  right to left for Arabic and Hebrew), from `page/i18n.js`, checked by
  `page/i18n.test.mjs` in CI. Tiles show the apps' own banners (or icons),
  see Storage; the built-in tiles (About, Settings, RustTV Remote, Hidden
  apps) show their banners too. Not yet: renaming a folder from the phone
  (typing works).
- **Look:** the launcher's colours, Inter, the RustTV icon and the built-in
  tile images.
  - Focus is shown as on the TV: the focused tile grows slightly, with a soft
    shadow and its name below.
  - Amber marks only the picked-up tile, mode badges and primary buttons.
  - Pressed buttons scale to 0.97 for 160 ms; nothing animates on keys the
    TV sends.
- **Screens:**
  - **Apps:** the Home mirror, including an open folder and move mode.
  - **Touchpad**, in two modes:
    - *Pointer*, in a listed app;
    - *Arrows*, in any other app.
  - **Typing:** the field the TV asks for. The phone's own keyboard opens by
    itself.
  - **Pairing:** the name field and the match code.

  A small bar at the bottom has Back, and Keyboard while the TV asks for
  text. There is no Apps / Touchpad switch: the page shows Apps while
  RustTV is in front and the touchpad while another app is.
- **Any screen:** tile columns follow the width (at least 130 px, growing
  with the screen up to 240 px): 2 on a 320 px phone, 4 to 5 in landscape
  and on an iPad, about 6 on a computer, like the TV. Sheets and cards keep
  a phone's width on wide screens, and nothing sits under a notch.
- **Add to Home Screen:** once paired, a phone browser that didn't open the
  page from the Home Screen suggests adding it (iOS: Share → Add to Home
  Screen; Android: the browser menu). "Got it" ends the suggestion, "Not
  now" for a week. Never while the pairing link is in the address.
- **No secure context on plain HTTP:**
  - What's missing: Web Crypto, clipboard reads, service workers, and so a
    real install on Android Chrome. Its "Add to Home screen" still makes a
    shortcut.
  - What works: iOS Safari opens the page standalone with
    `apple-mobile-web-app-capable`. `ws://` works from an `http://` page.
    Typing, touch and the person's own paste work.
  - So the page carries its own SHA-256, HMAC and XChaCha20-Poly1305 in pure
    JS (`page/crypto.js`, ~350 lines written for this project, MIT like the
    rest; Poly1305 on BigInt). `page/crypto.test.mjs` checks it in CI
    against the crate's test vectors and published SHA-256, HMAC and
    HChaCha20 vectors. `crypto.getRandomValues` works without a secure
    context.
- **Storage:** the long-term secret and the session key live in the page's
  `localStorage`, which belongs to the page's own address.
  - If the browser clears site data, the phone must scan the QR again.
  - Tile pictures are cached in IndexedDB, by component, and asked for
    again after a week (an app's banner rarely changes; reading every app's
    version at listing time would slow the TV's cold start). The Cache API
    is off on plain HTTP.
  - As built (2026-10-25): the phone asks for each app it has no fresh
    picture of; the TV checks the app is installed and listed (on Home, in
    a folder or on the hidden apps page), and its picture thread draws the banner (else the icon)
    through `Bridge.tilePicture` and compresses it to WebP with Android's
    own encoder, then sends it to that phone only. After each burst it asks
    Android for one garbage collection, as the launcher does after loading
    its own banners: without it, what the system decoded kept the launcher
    at ~107 MB PSS instead of ~68 MB. Nothing else is kept on the TV.
    Measured on the Philips with 29 apps (2026-10-25): 109 KB in all
    (median 3.5 KB, largest 11.7 KB); on the phone the first arrived 192 ms
    and the last 827 ms after the page started; 0.70 s of one core on the
    TV in total (3 runs: 70, 70, 71 ticks), about 23 ms per picture for
    drawing and compressing; the collection itself takes 12–17 ms (ART's
    log). Decoding banners with BitmapFactory instead, to skip the
    collection, measured no better (76 ticks) and was dropped. Idle CPU
    unchanged (2 ticks in 30 s); a reload with the cache costs nothing.

## 2. Pairing and authentication

- **As built (2026-10-25):** RustTV Remote is a page of its own (owner
  decision 2026-10-25), opened from its own built-in Home tile (`remote`,
  after About and Settings; it moves like them, and its banner is the
  remote from the phone's home-screen icon). The page shows that banner
  and what a phone may do, then "Pair a phone" and the paired phones (OK,
  then OK again, removes one). "Pair a phone"
  starts the listener if needed and opens a FLAG_SECURE screen with the QR
  code (drawn by the launcher from `qrcode`'s modules) and the TV's address
  (the source address of its route to the internet, found with a UDP
  socket that sends nothing). A phone that asks shows "Allow “name”?" and
  the code; only OK on the TV's remote allows it, Back refuses. Phone input
  never reaches this screen, and on the page a phone can only press Back.
  With no phone left, the listener stops. Not yet: the history entry for
  pairing and removing (needs the phones in the history snapshot, a format
  change of its own) and an on/off switch. `rusttv-launcher-desktop
  --demo-remote <dir>` renders the page and the pairing screen. The tile
  is not in the history snapshot: every Home has it, and RustTV builds that
  don't know its name must still read the history (HISTORY.md § 1).
- **Starting:** "Pair a phone" on the RustTV Remote page shows a QR code with
  `http://<ip>:8484/#p=<secret>`.
  - The secret is 32 random bytes from `SecureRandom`, valid for 2 minutes
    and for one use, and only while that screen is open.
  - It sits in the URL fragment, which browsers never send to a server.
- **Confirming:** the phone sends `pair { phone_nonce, name, proof }`: a
  fresh 32-byte nonce, its name, and an HMAC under the pairing secret over
  both, which proves it read the QR code and binds the name. Then:
  - the TV shows "Allow *iPhone (Safari)*? Code 47", and the phone shows 47;
  - OK on the TV remote pairs, and Back refuses;
  - three failed pairings in a row close the pairing screen.
- **Keys:** the match code and two keys come from the pairing secret and the
  phone's nonce with HMAC; neither key is ever sent:
  - the phone's long-term secret (`auth`);
  - a 32-byte encryption key `K` (`key`).

  Every HMAC input is a domain label, a zero byte, then fixed-length parts
  (a variable-length part only last); `rusttv-remote` pins the bytes with
  test vectors computed independently, for the page's JavaScript.
- **The phones file:** the TV keeps the paired phones in
  `files/remote/phones.toml`, in the launcher's private storage. Each phone
  has an `id`, `name`, `auth`, `key`, `paired_at` and `last_seen`.
  - The file is written atomically.
  - An unreadable file is kept as `.bak` and treated as "no phones", so a
    damaged file can't let anyone in.
  - A debuggable launcher refuses to pair, because the shell could read the
    file with `run-as`.
- **Each connection:**
  1. `hello { phone_id }`
  2. `challenge { nonce }` (32 random bytes)
  3. `auth { hmac }` over the challenge and the phone's id
  4. `ok` or `denied`

  The session key is derived from `K` and the nonce.
- **Encrypted messages:** every WebSocket message after `ok` is
  XChaCha20-Poly1305 with a random 24-byte nonce.
  - The associated data is a direction byte and the message's number in
    that direction, which neither side sends: a message can't be replayed,
    reordered, dropped or reflected without failing to open.
  - Tile pictures travel the same way, one message each.
- **Revoking:** on the RustTV Remote page, a phone can be revoked or renamed (each one a
  `tv` history entry). When the last phone is revoked, the listener stops.

### Trusting the TV (HTTPS)

*Owner decisions 2026-10-25.* Browsers give the motion sensors (the magic
remote) only to secure pages, and show a warning for a self-signed
certificate. So a phone can trust the TV once, after which the page opens
over HTTPS with no warning. It's optional: an untrusting phone keeps the
plain page as before.

- **The TV's certificate authority** (`rusttv_remote::tls`, `rustls` with
  `ring`, `rcgen`): made on first use and kept in the launcher's private
  storage (`files/remote/tls/`), named "RustTV" and a short fingerprint. Its
  name constraints let it vouch only for home network addresses (10/8,
  172.16/12, 192.168/16), so even a leaked key can't impersonate a website
  (tested: a phone refuses a certificate it made for a public address). The
  certificate for an address is issued when a phone first connects to it,
  so a new address after a reboot needs nothing from the person.
- **HTTPS** on port 8485 (TLS 1.3), next to the plain page on 8484. The
  plain page learns the port from a `<meta>` and may ask `https://…:8485/ok`
  whether the phone trusts the TV (a fetch only succeeds then).
- **Setting it up** (on the phone, after pairing): "Make this phone trust
  your TV" → Set up. The TV opens a "Trust this TV" screen with the phone's
  name, the certificate's name and a **check code**: the first 6 bytes of
  the certificate's SHA-256 fingerprint ("C5 8B 5F 12 9A 0E"). The phone
  downloads `/tv.crt` and shows short steps for its platform. Before
  installing, the person compares the code with the fingerprint the phone's
  own Settings show (iOS: the profile's More Details; Android: Trusted
  credentials): a tampered page can't fake that screen, and 48 bits can't
  be matched by making certificates until one fits. Phone input can't close
  the TV's screen; Back or 3 minutes does.
- **Switching:** when the phone comes back and the HTTPS page answers, the
  plain page moves to `https://…:8485/#k=<phone id>.<auth>.<key>`; the HTTPS
  page stores the keys (its storage is its own) and removes them from the
  address at once. A fragment is never sent to the TV.
- **Home Screen icon:** `/icon.png`, the remote on the RustTV tile
  (`scripts/brand/remote-icon.html`), full square as phones round it.
- **What it doesn't stop:** someone who rewrites the plain page on the
  network at the moment of setup could offer their own certificate; the
  code check catches that only if the person compares it. SECURITY.md has
  the full model.

## 3. The listener

- **Shape:** in Home's process, on std sockets with no tokio:
  - one accept thread on a `TcpListener` bound to `0.0.0.0:8484`, blocked
    while nobody connects;
  - one short-lived thread per admitted connection (at most 4 phones plus
    1 still authenticating).

  HTTP/1.1 is parsed with `httparse`. The upgrade answer is computed with
  `sha1dc` (already in the tree), and WebSocket frames are handled by
  `tungstenite` without its handshake feature.
  - A connection reads with a 100 ms timeout, so it can also send the latest
    view the launcher pushed; for 500 ms after the phone sends something it
    reads with 5 ms, so the view that answers goes out at once. Measured on
    the Philips (2026-10-25, 30 runs, phone page in a browser on the LAN):
    phone input to the answering view on the phone, median 115 ms before,
    24 ms after (p90 119 → 28 ms).
  - The UI picks events up on its existing short timer and pushes the
    `HomeView` after every model change.
  - `/ws` is refused unless `Origin` is the page itself, so another site
    open in the phone's browser can't open the socket.
- **When it runs:**
  - It starts only once a phone is paired, and again on each later process
    start if `phones.toml` lists phones.
  - It stops when the last phone is revoked or the Settings switch is off.
- **Before authentication:** anything that isn't a paired phone is refused
  after one small header.
- **Limits:** a parser panic would kill Home (`panic = "abort"`).
  - Every length is capped:
    - request line and headers: 4 KB;
    - WebSocket frames: 4 KB;
    - text: 1 KB per message;
    - tile pictures: at most 44 KB of WebP (64 KB with base64), drawn one
      at a time; a larger one isn't sent.
  - No recursion, and no allocation sized by the peer.
  - Caps on connections and messages:
    - at most 4 phones connected and 1 pending unpaired connection;
    - 30 connection attempts a minute, then 1 minute of silence;
    - 200 messages a second per phone (D-pad repeat is about 30 a second).
  - Idle sockets are pinged every 20 s and dropped after 2 missed pongs.
- **Tests:** property tests with random and damaged input, loopback tests
  with a real client, and fuzz targets in `crates/rusttv-remote/fuzz/`
  (`cargo +nightly fuzz run <target>`, on the Mac, not a runtime
  dependency):
  - `phone_message`: any bytes as a decrypted phone message;
  - `websocket_frames`: frames read with the listener's limits;
  - `request_head`: the HTTP request head and the upgrade's accept key;
  - `phones_file`: any text as the phones file (what parses writes back
    the same).

  First run (2026-10-25): 5 minutes each, about 86 million inputs in all,
  no crash, hang or failed assertion.
- **Fallback:** if the budgets in § 7 or fuzzing say so, the listener moves
  to a Kotlin `Service` with `android:process=":remote"`. That service loads
  the same native library and talks to Home over a `LocalSocket`.

## 4. Protocol (v1)

JSON over one WebSocket. Every message is at most 4 KB and comes from a fixed
set. Unknown fields are ignored; unknown messages are dropped and never
echoed.

- **Phone → TV:**
  - `hello`, `auth`;
  - `key { name, action: down | up | press | long }`, with names from a fixed
    list (`long` is OK held still for 0.6 s and lifted, decided on the
    phone, so a resting finger that then swipes is never a long press):
    up, down, left, right, ok, back, home, menu, play_pause, volume_up,
    volume_down, mute, power, …;
  - `text { insert | delete | submit }`;
  - `open { app }`, a component from the list the TV sent;
  - `tile { id, gesture: tap | long_press }`;
  - `drag { id, over, phase, on }`: `start` picks the tile up as Move in
    its menu would (`Home::pick_up`). A `move` names the tile under the
    finger (`over` is the dragged tile itself while nothing else is under
    the finger, `null` only outside an open folder's panel, which takes the
    app out, `Home::drag_out`). Near that tile's edges (`on` false) the
    dragged tile takes its place; over its middle (`on`: within a quarter
    of its width and 30% of its height of the centre) it stays, and resting
    there for the launcher's 0.6 s makes a folder, on the launcher's own
    timer (`Home::drag_over`). The page sends a move only after the finger
    stays 150 ms, so tiles crossed on the way don't shuffle (on the TV a
    drag across a tile used to move there and back). `end` drops it where
    the last move put it; `cancel` puts it back;
  - `banner { id }`: an app's tile picture, for apps on Home only;
  - `pointer { dx, dy, click | scroll }`;
  - `name { phone_id, text }`, during pairing only.
- **TV → phone:**
  - `challenge`, `ok`, `denied`, `ping`;
  - `state`, which carries the `HomeView` while RustTV is in front, or
    `{ app, mode: pointer | arrows, text_field }` while another app is;
  - `banner { id, kind: banner | icon, webp }`, to the phone that asked,
    in order (views are coalesced, pictures are not).
- **`HomeView`:**
  - the tiles in order: id, label, banner version, folder membership;
  - the open folder and its page;
  - the focused tile;
  - the mode: browse, menu, moving, hidden list, settings, review, or text
    with its label;
  - the open long-press menu (its items and the selected one), and on the
    hidden apps page the hidden apps.

  The phone mirrors Home exactly, the hidden apps page included (owner's
  decision, 2026-10-25): every paired phone, a guest's too, sees the hidden
  apps while that page is open on the TV, and their pictures. It is made
  from `Home` by a pure, tested
  function and is about 4 KB for ~100 tiles. It is sent whole on each change,
  with no diffs in v1.
- **Phone input goes into `Home::handle(Input, now)`**, like the remote's
  keys, so the model tests cover both:
  - a tap on a tile is `FocusTile` followed by `OkPressed`;
  - a drag is move mode, with the model's existing 0.6 s hover-to-folder
    timing;
  - `menu { choice: move | hide }` picks that item of the open menu, as
    selecting it and pressing OK (`Home::menu_pick`);
  - on the hidden apps page, a tap on an app puts it back on Home, as OK
    does.

  Settings, About and the RustTV Remote page show on the phone only as
  their name; there, as before, a phone can only press Back. The phone
  shows Back only where the TV's Back does something (a folder, the menu,
  a move, the hidden apps page, Settings, About, the RustTV Remote page,
  typing).

  The review/approval screen **drops every phone input**: phone keys arrive
  through RustTV's own socket, so they can be told apart from the remote.

## 5. Other apps

### The Android TV Remote Service

Package `com.google.android.tv.remote.service` is what the Google TV phone app
talks to. Its protocol is documented by open-source clients such as
`androidtvremote2` (Apache-2.0):
- TLS on port 6467 for pairing and 6466 for the remote, with self-signed
  certificates on both sides;
- protobuf messages, each with a length prefix.

- **The client:** `RemoteService.kt`.
  - An RSA key pair with a self-signed certificate from `AndroidKeyStore`.
  - The TV's server certificate is pinned after pairing.
  - It connects with an `SSLSocket` to `127.0.0.1`.
  - Protobuf is encoded by hand: a dozen fields, no library.
  - A typed `RemoteKey` enum is the only way to send a key. No free-form
    bytes come from Rust or the phone.
- **Pairing:** "Connect RustTV to this TV's remote service" is a row the
  person chooses in Settings; RustTV never pairs by itself.
  - The TV shows its own 6-character code, and the person types it on the
    phone page.
  - The pairing is recorded in the history (`remote_service_paired`) and can
    be removed again: RustTV deletes its key and the pinned certificate. The
    TV also lists RustTV under its own remote-control settings.
- **What it gives:**
  - keys, including long presses;
  - volume and power;
  - opening apps;
  - text into **any** focused field, with IME events: the TV sends a show
    request with the field's value and label, and the package of the app in
    front. So RustTV needs no input method of its own.

### The pointer

- **What it is:** a narrow accessibility service, `PointerService.kt`. It
  draws a round cursor in a `TYPE_ACCESSIBILITY_OVERLAY` window and performs
  taps, drags and two-finger scrolls with `dispatchGesture`, at the pointer's
  position.
- **Declaration:** `canPerformGestures` only.
  - No `canRetrieveWindowContent`.
  - No key-event filtering (`flagRequestFilterKeyEvents` is off), so it can't
    read key presses.
  - The only event type is `TYPE_WINDOW_STATE_CHANGED`, read for its package
    name, to know which app is in front.
- **When it acts:** only while a listed app is in front. Browsers are listed
  by default, and the person can add apps in Settings. In other apps it
  draws nothing and the touchpad sends arrows through the Remote Service.
- **Switching it on:** it is off by default, and the person turns it on in
  Android's accessibility settings. A changed accessibility service shows as
  a warning in `history sync` (HISTORY.md watchlist), once, when the person
  turns it on.
- **Checks:** `check-dex.sh` gains rules that only `PointerService.kt` uses
  the accessibility APIs, and that it never calls a window-content API.

## 6. Safety (SECURITY.md § RustTV Remote)

- **What a phone can do:** what a remote can do — keys, text, opening an app
  and, in a listed app, pointer taps.
- **What a phone can never do:**
  - change a setting or a package;
  - pair or revoke phones;
  - write to the history store;
  - pass the review screen.
- **The person's own actions on the RustTV Remote page:** pairing a phone, revoking one, and
  pairing with the Remote Service. Each one:
  - shows a preview ("This phone will be able to press keys and type on this
    TV");
  - asks for OK;
  - is recorded as a `tv` history entry (`phone_paired`, `phone_revoked`,
    `remote_service_paired`, `remote_service_unpaired`).
- **Outside the CLI's risk classes:** this is a TV-side feature, like
  launcher updates. The CLI's watchlist gains no key.

## 7. Phases and budgets

- **Phase 0 — facts (no launcher code).** Read-only checks on the Philips:
  - whether the Remote Service is installed and enabled, its version, and
    whether it listens on 6466/6467;
  - whether Google TV lets a person turn on a third-party accessibility
    service;
  - on the owner's phones: whether `http://` pages can be added to the home
    screen and keep `localStorage` for days.
- **Phase 1 — the mirror and the keyboard inside RustTV:**
  - the `rusttv-remote` crate: protocol, auth, encryption, server, page;
  - the remote thread;
  - the RustTV Remote page (its own Home tile) and the pairing prompt;
  - the `HomeView` push and banner transfer;
  - phone editing;
  - history entries.
- **Phase 2 — other apps:**
  - `RemoteService.kt`;
  - the keyboard from IME events;
  - the touchpad (arrows);
  - `PointerService.kt` and the pointer.
- **Phase 3 — polish:**
  - haptics on Android, landscape, several TVs on one phone;
  - `rusttv remote status` (R0, read through the history door).
- **Phase 4 — second screen per app, research first:**
  - usage access for the app in front;
  - media sessions (notification access) for seek and artwork;
  - Jellyfin's API;
  - layouts as data add-ons (ROADMAP § Panels).

  Each is its own owner decision.

**Budgets** (BENCHMARK_PLAN.md method, raw data under
`benchmarks/raw/<device>/<date>/remote/`):
- cold start, resume, and key → frame: unchanged beyond noise;
- idle PSS: at most +1 MB with no phone, at most +2 MB with one phone
  connected;
- idle CPU: unchanged with a phone connected and idle;
- APK size: at most +600 KB.

Phone tap → TV frame latency is measured with the animation-metric recipe.

## Code layout

```text
crates/rusttv-remote/   protocol.rs, auth.rs (pairing, HMAC, phones file), crypto.rs,
                        server.rs (httparse + tungstenite on std), page/index.html
crates/rusttv-launcher/ remote.rs (the remote thread), ui/settings_view.rs (RustTV Remote
                        section), model.rs (Input::Remote*, review drops them)
android/                RemoteService.kt, PointerService.kt, Bridge.kt (lanAddress)
```

`rusttv-remote` depends on neither `rusttv-core` nor the launcher
(ARCHITECTURE § Naming and boundaries).
