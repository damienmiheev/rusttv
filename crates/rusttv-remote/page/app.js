// RustTV Remote: the phone page (REMOTE.md § 1, § 4). Pairs once from the QR
// code's link, then connects, authenticates and talks to the TV in sealed
// messages. Renders what the TV sends: the Home mirror or, while another app
// is in front, the touchpad. Everything the TV sends is shown as text, never
// as HTML.
"use strict";
(() => {
  const C = window.RustTVCrypto;
  const { t } = window.RustTVText;
  const $ = (id) => document.getElementById(id);
  const encoder = new TextEncoder(), decoder = new TextDecoder();
  const random = (length) => crypto.getRandomValues(new Uint8Array(length));
  const STORE = "rusttv-remote";
  // Phase 2 (REMOTE.md § 5): until the TV controls other apps through its
  // remote service, their buttons stay hidden instead of doing nothing.
  const CAN_CONTROL_APPS = false;
  const MAX_TEXT = 1024;

  // --- Storage (the phone's keys for this TV's address) ----------------------
  const store = {
    get() {
      try {
        const value = JSON.parse(localStorage.getItem(STORE));
        return value && C.unhex(value.phone_id, 16) && C.unhex(value.auth, 32) && C.unhex(value.key, 32) ? value : null;
      } catch {
        return null;
      }
    },
    set(value) {
      try {
        localStorage.setItem(STORE, JSON.stringify(value));
      } catch {
        // Private browsing: this tab still works; the next one pairs again.
      }
    },
    clear() {
      try {
        localStorage.removeItem(STORE);
      } catch {
        // Nothing stored.
      }
    },
  };

  // Arriving on the HTTPS page from the plain one: the phone's keys come
  // in the address's fragment, which browsers never send, and leave it at
  // once (REMOTE.md § 2 "Trusting the TV").
  const moved = location.hash.match(/^#k=([0-9a-f]{32})\.([0-9a-f]{64})\.([0-9a-f]{64})$/);
  if (moved && window.isSecureContext) {
    history.replaceState(null, "", location.pathname);
    store.set({ phone_id: moved[1], auth: moved[2], key: moved[3] });
  }
  let credentials = store.get();
  // A Home Screen icon's address carries a one-time handover token from
  // the browser it was added in (iOS gives the icon storage of its own):
  // without keys, it trades the token for them (REMOTE.md § 1).
  const handover = location.hash.match(/^#h=([0-9a-f]{32})\.([0-9a-f]{64})$/);
  let redeeming = null; // { phone_id, token, nonce } until the keys come
  if (handover) {
    history.replaceState(null, "", location.pathname);
    if (!credentials) redeeming = { phone_id: handover[1], token: C.unhex(handover[2], 32), nonce: random(32) };
  }
  let pairing = null; // { secret, nonce, name } while pairing
  let socket = null, session = null, challenge = null;
  let retry = 0, retryTimer = 0;
  // `soon`: the next close reconnects at once. `hiddenAt`: when the page
  // last went to the background.
  let soon = false, hiddenAt = Date.now();
  let view = null, lastKind = null, tab = "apps";

  function show(id) {
    for (const screen of ["scan", "pair", "main"]) $(screen).hidden = screen !== id;
  }

  // --- Names ----------------------------------------------------------------
  function cleanName(raw) {
    const text = raw.replace(/[\u0000-\u001f\u007f-\u009f؜‎‏‪-‮⁦-⁩]/g, "").trim();
    return [...text].length >= 1 && [...text].length <= 40 ? text : null;
  }

  async function guessName() {
    const agent = navigator.userAgent;
    const browser = /SamsungBrowser/.test(agent) ? "Samsung Internet"
      : /EdgA|EdgiOS/.test(agent) ? "Edge"
      : /Firefox|FxiOS/.test(agent) ? "Firefox"
      : /CriOS|Chrome/.test(agent) ? "Chrome"
      : /Safari/.test(agent) ? "Safari" : "Browser";
    let device = /iPad/.test(agent) ? "iPad" : /iPhone/.test(agent) ? "iPhone" : /Android/.test(agent) ? "Android phone" : "Phone";
    try {
      const data = await navigator.userAgentData?.getHighEntropyValues(["model"]);
      if (data?.model) device = data.model;
    } catch {
      // Not offered: keep the guess.
    }
    return cleanName(`${device} (${browser})`) || "Phone";
  }

  // --- Start ----------------------------------------------------------------
  async function start() {
    const link = location.hash.match(/^#p=([0-9a-f]{64})$/);
    if (link) {
      // The secret never stays in the address bar or the history.
      history.replaceState(null, "", location.pathname);
      pairing = { secret: C.unhex(link[1], 32) };
      show("pair");
      $("phone-name").value = await guessName();
      return;
    }
    if (credentials || redeeming) {
      show("main");
      connect();
    } else {
      // A Home Screen icon without keys can't pair itself: a QR code always
      // opens in the browser.
      if (standalone()) $("scan-text").textContent = t("home_unpaired");
      show("scan");
    }
  }

  $("pair-form").addEventListener("submit", (event) => {
    event.preventDefault();
    if (!pairing || pairing.nonce) return;
    const name = cleanName($("phone-name").value);
    if (!name) {
      $("pair-status").textContent = t("name_needed");
      return;
    }
    pairing.name = name;
    pairing.nonce = random(32);
    $("code").textContent = String(C.matchCode(pairing.secret, pairing.nonce)).padStart(2, "0");
    $("name-field").hidden = true;
    $("pair-button").hidden = true;
    $("pair-code").hidden = false;
    $("pair-status").textContent = t("waiting");
    connect();
  });

  // --- Connection -----------------------------------------------------------
  function connect() {
    clearTimeout(retryTimer);
    if (socket) return;
    // Over HTTPS the socket is TLS too (a secure page can't open ws://).
    socket = new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws`);
    socket.binaryType = "arraybuffer";
    socket.onopen = () => {
      if (pairing?.nonce) {
        sendText({
          type: "pair",
          phone_nonce: C.hex(pairing.nonce),
          name: pairing.name,
          proof: C.hex(C.pairingProof(pairing.secret, pairing.nonce, pairing.name)),
        });
      } else if (redeeming) {
        sendText({
          type: "redeem",
          phone_id: redeeming.phone_id,
          phone_nonce: C.hex(redeeming.nonce),
          proof: C.hex(C.handoverProof(redeeming.token, redeeming.nonce, C.unhex(redeeming.phone_id, 16))),
        });
      } else if (credentials) {
        sendText({ type: "hello", phone_id: credentials.phone_id });
      }
    };
    socket.onmessage = (event) => (typeof event.data === "string" ? onText(event.data) : onSealed(event.data));
    socket.onclose = () => {
      socket = null;
      session = null;
      setStatus(false, t("reconnecting"));
      if (pairing?.nonce) {
        failPairing(t("broke"));
      } else if (credentials && !document.hidden) {
        // A TV back from standby is found within 3 seconds: 0.5, 1, 2, 3, 3…
        retry = Math.min(retry + 1, 4);
        retryTimer = setTimeout(connect, soon ? 0 : Math.min(3000, 250 * 2 ** retry));
        soon = false;
      }
    };
  }

  // Connects again now: the phone is back from the background, or the
  // person tapped while the connection was down.
  function reconnectNow() {
    if (!credentials || pairing) return;
    clearTimeout(retryTimer);
    retry = 0;
    if (socket) {
      soon = true;
      socket.close();
    } else {
      connect();
    }
  }

  function sendText(message) {
    socket?.send(JSON.stringify(message));
  }

  function send(message) {
    if (!session || socket?.readyState !== WebSocket.OPEN) {
      // Down and waiting to retry: a tap retries now. (Still signing in:
      // wait for it.)
      if (!socket) reconnectNow();
      return;
    }
    socket.send(session.seal(encoder.encode(JSON.stringify(message)), random(24)));
  }

  function onText(text) {
    let message;
    try {
      message = JSON.parse(text);
    } catch {
      socket?.close();
      return;
    }
    if (message.type === "paired" && pairing?.nonce && C.unhex(message.phone_id, 16)) {
      const keys = C.phoneKeys(pairing.secret, pairing.nonce);
      credentials = { phone_id: message.phone_id, auth: C.hex(keys.auth), key: C.hex(keys.key) };
      store.set(credentials);
      pairing = null; // the pairing secret is forgotten
      show("main");
      sendText({ type: "hello", phone_id: credentials.phone_id });
    } else if (message.type === "challenge" && credentials && C.unhex(message.nonce, 32)) {
      challenge = C.unhex(message.nonce, 32);
      const response = C.authResponse(C.unhex(credentials.auth, 32), challenge, C.unhex(credentials.phone_id, 16));
      sendText({ type: "auth", hmac: C.hex(response) });
    } else if (message.type === "ok" && challenge) {
      session = new C.Session(C.sessionKey(C.unhex(credentials.key, 32), challenge));
      asked.clear(); // answers lost with the last connection are asked again
      challenge = null;
      retry = 0;
      setStatus(true, t("connected"));
      offerHandover();
      checkTrust();
    } else if (message.type === "denied") {
      if (pairing?.nonce) {
        failPairing(t("refused"));
      } else if (redeeming) {
        // The token was spent, too old, or the TV restarted since.
        redeeming = null;
        $("scan-text").textContent = t("home_unpaired");
        show("scan");
      } else {
        // The TV doesn't know this phone any more (removed on the RustTV
        // Remote page).
        credentials = null;
        store.clear();
        $("scan-text").textContent = t("unpaired");
        show("scan");
      }
      socket?.close();
    }
  }

  function failPairing(text) {
    pairing = null;
    $("pair-code").hidden = true;
    $("pair-status").textContent = text;
  }

  function onSealed(data) {
    if (redeeming && !session) return gotKeys(new Uint8Array(data));
    const plain = session?.open(new Uint8Array(data));
    if (!plain) {
      socket?.close();
      return;
    }
    try {
      onView(JSON.parse(decoder.decode(plain)));
    } catch {
      socket?.close();
    }
  }

  // The TV's answer to a handover token: this phone's keys, sealed under a
  // key from the token. Then the icon signs in as usual.
  function gotKeys(frame) {
    const plain = new C.Session(C.handoverKey(redeeming.token, redeeming.nonce)).open(frame);
    let keys = null;
    try {
      keys = plain && JSON.parse(decoder.decode(plain));
    } catch {
      // Not keys: refused below.
    }
    if (keys?.type !== "keys" || !C.unhex(keys.auth, 32) || !C.unhex(keys.key, 32)) {
      socket?.close();
      return;
    }
    credentials = { phone_id: redeeming.phone_id, auth: keys.auth, key: keys.key };
    store.set(credentials);
    redeeming = null;
    sendText({ type: "hello", phone_id: credentials.phone_id });
  }

  // In a browser on iOS, the address always carries a fresh one-time token,
  // so a Home Screen icon added from it, with or without the hint, starts
  // paired. The TV keeps a phone's 3 newest for a day. (Android's icons open
  // in the browser, with its storage.)
  function offerHandover() {
    const ios = /iPhone|iPad|iPod/.test(navigator.userAgent) || (navigator.maxTouchPoints > 1 && /Mac/.test(navigator.userAgent));
    if (!ios || standalone() || (location.hash && !location.hash.startsWith("#h="))) return;
    const token = random(32);
    send({ type: "handover", token: C.hex(token) });
    history.replaceState(null, "", `${location.pathname}#h=${credentials.phone_id}.${C.hex(token)}`);
  }

  document.addEventListener("visibilitychange", () => {
    if (document.hidden) {
      hiddenAt = Date.now();
      return;
    }
    if (!credentials || pairing) return;
    if (!socket) {
      reconnectNow();
    } else if (Date.now() - hiddenAt > 5000) {
      // iOS freezes pages in the background: a socket that looks open may
      // be dead, and taps would go nowhere until the TV gave up on it.
      reconnectNow();
    }
  });

  // --- Views ------------------------------------------------------------------
  function setStatus(on, text) {
    $("dot").classList.toggle("on", on);
    $("status").textContent = text;
  }

  function onView(message) {
    if (message.type === "banner") return gotPicture(message);
    if (message.type !== "home" && message.type !== "app") return;
    view = message;
    if (message.type !== lastKind) {
      lastKind = message.type;
      setTab(message.type === "home" ? "apps" : "touchpad");
    }
    setStatus(true, message.type === "home" ? t("rusttv_front") : message.app ? t("app_front", message.app) : t("other_front"));
    render();
  }

  function setTab(name) {
    tab = name;
    $("apps").hidden = name !== "apps";
    $("touchpad").hidden = name !== "touchpad";
    render();
  }

  // A banner stand-in until banners travel: a colour from the tile's id.
  function colour(id) {
    let hash = 0;
    for (const c of id) hash = (hash * 31 + c.charCodeAt(0)) >>> 0;
    return `hsl(${hash % 360} 45% 30%)`;
  }

  // --- Tile pictures ------------------------------------------------------------
  // The TV draws an app's banner (or icon) when asked; the phone keeps it in
  // IndexedDB and asks again after a week, so a connection only fetches what
  // is new. Until it arrives, the tile shows a colour from its id.
  const WEEK_MS = 7 * 24 * 3600 * 1000;
  // The launcher's own tiles; their pictures come from the TV too.
  const BUILTINS = ["about", "settings", "remote", "hidden-apps"];
  const pictures = new Map(); // id -> { kind, url, at }
  const asked = new Set();
  const db = new Promise((resolve) => {
    // Without the cache within 2 s (another tab or the browser holds it
    // blocked), the page fetches pictures each time rather than none.
    setTimeout(() => resolve(null), 2000);
    try {
      const open = indexedDB.open("rusttv-remote", 1);
      open.onupgradeneeded = () => open.result.createObjectStore("pictures", { keyPath: "id" });
      open.onsuccess = () => {
        // Let go when another tab or the browser wants to change it.
        open.result.onversionchange = () => open.result.close();
        resolve(open.result);
      };
      open.onerror = () => resolve(null);
    } catch {
      resolve(null); // No IndexedDB (private browsing): fetch each time.
    }
  });
  const picturesLoaded = db.then((store) => new Promise((resolve) => {
    if (!store) return resolve();
    let all;
    try {
      all = store.transaction("pictures").objectStore("pictures").getAll();
    } catch {
      return resolve(); // Closed meanwhile: no cache this time.
    }
    all.onsuccess = () => {
      for (const { id, kind, blob, at } of all.result) {
        pictures.set(id, { kind, url: URL.createObjectURL(blob), at });
      }
      resolve();
    };
    all.onerror = () => resolve();
  }));

  picturesLoaded.then(render);

  function gotPicture({ id, kind, webp }) {
    if (typeof id !== "string" || typeof webp !== "string" || (kind !== "banner" && kind !== "icon")) return;
    let blob;
    try {
      blob = new Blob([Uint8Array.from(atob(webp), (c) => c.charCodeAt(0))], { type: "image/webp" });
    } catch {
      return;
    }
    const old = pictures.get(id);
    if (old) URL.revokeObjectURL(old.url);
    const at = Date.now();
    pictures.set(id, { kind, url: URL.createObjectURL(blob), at });
    db.then((store) => {
      try {
        store?.transaction("pictures", "readwrite").objectStore("pictures").put({ id, kind, blob, at });
      } catch {
        // Not kept: asked for again next time.
      }
    });
    render();
  }

  // Asks the TV for the apps' pictures this phone lacks (or got a week ago).
  function askPictures(tiles) {
    for (const tile of tiles) {
      const builtin = BUILTINS.includes(tile.id);
      if ((!tile.id.includes("/") && !builtin) || asked.has(tile.id)) continue;
      const have = pictures.get(tile.id);
      if (have && Date.now() - have.at < WEEK_MS) continue;
      asked.add(tile.id);
      send({ type: "banner", id: tile.id });
    }
  }

  function art(tile, tiles) {
    const box = document.createElement("div");
    box.className = "art";
    const picture = pictures.get(tile.id);
    if (picture) {
      const image = document.createElement("img");
      image.src = picture.url;
      image.alt = "";
      image.draggable = false;
      box.classList.add(picture.kind);
      box.dataset.picture = picture.url;
      box.append(image);
      if (picture.kind === "icon") {
        const label = document.createElement("span");
        label.textContent = tile.label;
        box.append(label);
      }
      return box;
    }
    if (tile.id.startsWith("folder:")) {
      box.style.background = "#2A2E37";
      const minis = document.createElement("div");
      minis.className = "minis";
      for (const inner of tiles.filter((t) => t.folder === tile.id).slice(0, 4)) {
        const mini = document.createElement("i");
        const picture = pictures.get(inner.id);
        mini.style.background = picture?.kind === "banner" ? `center / cover url("${picture.url}")` : colour(inner.id);
        minis.append(mini);
      }
      box.append(minis);
    } else {
      const builtin = BUILTINS.includes(tile.id);
      box.style.background = builtin
        ? "linear-gradient(150deg, #FFC35C, #F5A524 42%, #D9480F)"
        : `linear-gradient(150deg, ${colour(tile.id)}, #0E1014 140%)`;
      const label = document.createElement("span");
      label.textContent = nameOf(tile);
      box.append(label);
    }
    return box;
  }

  // Tiles that changed place slide there (FLIP), short and strong ease-out.
  const reducedMotion = matchMedia("(prefers-reduced-motion: reduce)");
  function flip(container, rebuild) {
    const before = new Map();
    for (const element of container.querySelectorAll(".tile")) before.set(element.dataset.id, element.getBoundingClientRect());
    rebuild();
    if (reducedMotion.matches || before.size === 0) return;
    for (const element of container.querySelectorAll(".tile")) {
      const was = before.get(element.dataset.id);
      if (!was) continue;
      const now = element.getBoundingClientRect();
      const dx = was.left - now.left, dy = was.top - now.top;
      if (dx || dy) {
        element.animate([{ transform: `translate(${dx}px, ${dy}px)` }, { transform: "none" }],
          { duration: 220, easing: "cubic-bezier(0.23, 1, 0.32, 1)" });
      }
    }
  }

  // Puts `shown` into `container` in order, reusing each tile's element: an
  // element under a finger must stay in the page, or the browser can lose
  // the rest of the touch. The dragged tile keeps its children too.
  function sync(container, shown, tiles, focused) {
    const old = new Map([...container.children].map((element) => [element.dataset.id, element]));
    const next = shown.map((tile) => {
      const fresh = tileButton(tile, tiles, focused);
      const element = old.get(tile.id);
      if (!element) return fresh;
      element.className = fresh.className;
      element.setAttribute("aria-label", nameOf(tile));
      if (!(gesture && element === gesture.element)) {
        // The same picture: keep its element, so it isn't decoded again.
        const was = element.firstChild, art = fresh.firstChild;
        if (art.dataset.picture && was?.dataset.picture === art.dataset.picture) fresh.replaceChild(was, art);
        element.replaceChildren(...fresh.childNodes);
      }
      return element;
    });
    for (const element of old.values()) if (!next.includes(element)) element.remove();
    next.forEach((element, index) => {
      if (container.children[index] !== element) container.insertBefore(element, container.children[index] || null);
    });
  }

  // Built-in tiles in the phone's language (RustTV Remote is a name);
  // apps and folders by their name.
  function nameOf(tile) {
    if (tile.id === "remote") return "RustTV Remote";
    const key = { about: "tile_about", settings: "tile_settings", "hidden-apps": "hidden_title" }[tile.id];
    return key ? t(key) : tile.label;
  }

  function tileButton(tile, tiles, focused) {
    const button = document.createElement("button");
    const moving = dragging && gesture && tile.id === gesture.id;
    const target = dragging && gesture && gesture.on && tile.id === gesture.over;
    button.className = "tile" + (tile.id === focused ? " focused" : "") + (moving ? " dragging" : "") + (target ? " over" : "");
    button.dataset.id = tile.id;
    button.setAttribute("aria-label", nameOf(tile));
    button.append(art(tile, tiles));
    const name = document.createElement("div");
    name.className = "name";
    name.textContent = nameOf(tile);
    button.append(name);
    return button;
  }

  function render() {
    if (!view) return;
    const home = view.type === "home";
    const tiles = home ? view.tiles || [] : [];
    const focused = home ? view.focused : null;
    const focusedTile = tiles.find((t) => t.id === focused);
    $("hero").textContent = focusedTile ? nameOf(focusedTile) : "";
    $("glow").style.setProperty("--glow", focusedTile ? colour(focusedTile.id).replace("30%)", "30% / 0.35)") : "#F5A52426");

    if (tab === "apps") {
      // While a tile is dragged the grid reflows live, as the TV's does.
      flip($("grid"), () => sync($("grid"), tiles.filter((t) => !t.folder), tiles, focused));
      const folder = home && view.folder;
      $("folder").hidden = !folder;
      if (folder) {
        $("folder-name").textContent = folder.name;
        flip($("folder-grid"), () =>
          sync($("folder-grid"), tiles.filter((t) => t.folder === folder.id), tiles, focused));
      }
    }
    // The TV's long-press menu, hidden apps page and other screens.
    const mode = home ? view.mode : null;
    const menu = mode === "menu" && view.menu;
    $("menu").hidden = !menu;
    if (menu) {
      $("menu-title").textContent = focusedTile ? nameOf(focusedTile) : "";
      $("menu-items").replaceChildren(...menu.items.map((choice, index) => {
        const item = document.createElement("button");
        item.setAttribute("role", "menuitem");
        item.className = index === menu.selected ? "selected" : "";
        item.textContent = t(choice === "hide" ? "menu_hide" : "menu_move");
        item.addEventListener("click", () => send({ type: "menu", choice }));
        return item;
      }));
    }
    const hidden = mode === "hidden" ? view.hidden || [] : [];
    $("hidden-page").hidden = mode !== "hidden";
    if (mode === "hidden") sync($("hidden-grid"), hidden, hidden, focused);
    $("screen").hidden = mode !== "settings";
    if (mode === "settings") $("screen-title").textContent = nameOf({ id: BUILTINS.includes(focused) ? focused : "settings" });
    // Back only where the TV's Back does something.
    const back = !!(home && (view.folder || ["menu", "moving", "hidden", "settings", "text"].includes(mode)));
    $("back").hidden = !back;
    if (home && session) picturesLoaded.then(() => askPictures(tiles.concat(hidden)));
    const moving = home && view.mode === "moving";
    $("moving").hidden = !moving;
    if (moving) {
      const moved = (gesture && tiles.find((t) => t.id === gesture.id)) || focusedTile;
      $("moving-title").textContent = moved ? t("moving", moved.label) : t("moving_any");
    }

    for (const element of document.querySelectorAll("[data-other-apps]")) element.hidden = !CAN_CONTROL_APPS;
    const pointer = !home && view.pointer;
    $("mode").textContent = t(pointer ? "mode_pointer" : "mode_arrows");
    $("mode").className = "mode" + (pointer ? " pointer" : "");
    $("pad-hint").textContent = pointer
      ? t("hint_pointer")
      : home
        ? t("hint_home")
        : CAN_CONTROL_APPS
          ? t("hint_app")
          : t("hint_soon");
    if (!home && !CAN_CONTROL_APPS) $("mode").textContent = t("mode_soon");
    const field = view.text;
    $("wants").hidden = !field;
    $("keyboard").hidden = !field;
    $("nav").hidden = $("back").hidden && $("keyboard").hidden;
    if (field) $("wants-text").textContent = field.label ? t("wants", field.label) : t("wants_any");
    if (field && !$("typing").hidden) syncTyping(field);
    if (field && home && view.mode === "text" && $("typing").hidden) openTyping();
    if (!field && !$("typing").hidden && typingAsked) closeTyping();
  }

  // A short tap felt in the hand. iOS has no navigator.vibrate, but since
  // iOS 18 its browsers tick when a switch checkbox flips, also when its
  // label is clicked from script.
  function haptic(ms) {
    if (navigator.vibrate) {
      navigator.vibrate(ms);
      return;
    }
    const label = document.createElement("label");
    label.ariaHidden = "true";
    label.style.display = "none";
    const input = document.createElement("input");
    input.type = "checkbox";
    input.setAttribute("switch", "");
    label.append(input);
    document.head.append(label);
    label.click();
    label.remove();
  }

  // --- Keys -------------------------------------------------------------------
  function press(name, action = "press") {
    haptic(8);
    send({ type: "key", name, action });
  }
  document.addEventListener("click", (event) => {
    const button = event.target.closest("[data-key]");
    if (button) press(button.dataset.key);
  });

  // --- Tiles: tap, hold, drag ---------------------------------------------------
  const HOLD = 450, SLOP = 12, DWELL = 150;
  let gesture = null, dragging = false;

  function tileAt(x, y) {
    const element = document.elementFromPoint(x, y)?.closest("[data-id]");
    return element ? element.dataset.id : null;
  }

  function onTileDown(event) {
    const tile = event.target.closest(".tile");
    if (!tile || !session) return;
    gesture = { id: tile.dataset.id, element: tile, x: event.clientX, y: event.clientY, held: false, over: tile.dataset.id, on: false, pointer: event.pointerId };
    gesture.timer = setTimeout(() => {
      if (!gesture) return;
      gesture.held = true;
      haptic(15);
    }, HOLD);
  }

  function onTileMove(event) {
    if (!gesture || event.pointerId !== gesture.pointer) return;
    const dx = event.clientX - gesture.x, dy = event.clientY - gesture.y;
    if (!gesture.held) {
      if (Math.hypot(dx, dy) > SLOP) {
        clearTimeout(gesture.timer); // a scroll, not a tap
        gesture = null;
      }
      return;
    }
    if (!dragging && Math.hypot(dx, dy) > SLOP) {
      dragging = true;
      gesture.element.classList.add("dragging");
      const ghost = $("ghost");
      ghost.replaceChildren(gesture.element.querySelector(".art").cloneNode(true));
      ghost.style.width = `${gesture.element.offsetWidth}px`;
      ghost.hidden = false;
      send({ type: "drag", id: gesture.id, over: null, phase: "start" });
    }
    if (dragging) {
      const ghost = $("ghost");
      ghost.style.left = `${event.clientX - ghost.offsetWidth / 2}px`;
      ghost.style.top = `${event.clientY - ghost.offsetHeight / 2}px`;
      // The tile under the finger; the dragged tile itself while nothing
      // else is; null only outside an open folder's panel (the app leaves
      // the folder).
      let over = tileAt(event.clientX, event.clientY) || gesture.id;
      if (!$("folder").hidden) {
        const panel = document.querySelector("#folder .panel").getBoundingClientRect();
        const inside = event.clientX >= panel.left && event.clientX <= panel.right
          && event.clientY >= panel.top && event.clientY <= panel.bottom;
        if (!inside) over = null;
      }
      // Over the middle of another tile's picture: rest there for a folder.
      // Near its edges: take its place. Once a folder is being made, it
      // holds while the finger stays over that picture or just past it,
      // so a finger that wobbles doesn't drop it; only leaving does.
      const middle = (id, reach) => {
        const art = document.querySelector(`.tile[data-id="${CSS.escape(id)}"] .art`)?.getBoundingClientRect();
        return !!art && Math.abs(event.clientX - (art.left + art.width / 2)) < art.width * reach
          && Math.abs(event.clientY - (art.top + art.height / 2)) < art.height * reach;
      };
      let on = false;
      // (Outside an open folder's panel, over is null: the app leaves it.)
      if (over !== null && gesture.on && gesture.over !== gesture.id && middle(gesture.over, 0.65)) {
        over = gesture.over;
        on = true;
      } else if (over && over !== gesture.id) {
        on = middle(over, 0.35);
      }
      // Only where the finger stays a moment: crossing tiles on the way
      // must not shuffle them (and the shuffle can undo itself).
      const same = over === gesture.over && on === gesture.on;
      const waiting = gesture.next && over === gesture.next.over && on === gesture.next.on;
      if (same) {
        clearTimeout(gesture.dwell);
        gesture.next = null;
      } else if (!waiting) {
        clearTimeout(gesture.dwell);
        gesture.next = { over, on };
        gesture.dwell = setTimeout(commitOver, DWELL);
      }
    }
  }

  function commitOver() {
    if (!gesture?.next) return;
    const { over, on } = gesture.next;
    gesture.next = null;
    document.querySelector(".tile.over")?.classList.remove("over");
    if (on) document.querySelector(`.tile[data-id="${CSS.escape(over)}"]`)?.classList.add("over");
    gesture.over = over;
    gesture.on = on;
    send({ type: "drag", id: gesture.id, over, phase: "move", on });
  }

  function onTileUp(event, cancelled) {
    if (!gesture || event.pointerId !== gesture.pointer) return;
    clearTimeout(gesture.timer);
    if (dragging) {
      clearTimeout(gesture.dwell);
      // Lifted before the dwell ran out: the drop goes where the finger is.
      if (!cancelled) commitOver();
      send({ type: "drag", id: gesture.id, over: gesture.over, phase: cancelled ? "cancel" : "end" });
      dragging = false;
      $("ghost").hidden = true;
      document.querySelector(".tile.over")?.classList.remove("over");
      gesture = null;
      render();
      return;
    }
    if (!cancelled) send({ type: "tile", id: gesture.id, gesture: gesture.held ? "long_press" : "tap" });
    gesture = null;
  }

  // Tapping outside the menu closes it, as Back does.
  $("menu").addEventListener("click", (event) => {
    if (event.target === $("menu")) press("back");
  });

  for (const area of [$("apps"), $("folder"), $("hidden-page")]) {
    area.addEventListener("pointerdown", onTileDown);
    area.addEventListener("contextmenu", (event) => event.preventDefault());
  }
  document.addEventListener("pointermove", onTileMove);
  document.addEventListener("pointerup", (event) => onTileUp(event, false));
  document.addEventListener("pointercancel", (event) => onTileUp(event, true));
  // While dragging, the finger moves the tile, not the page.
  document.addEventListener("touchmove", (event) => {
    if (dragging) event.preventDefault();
  }, { passive: false });

  // --- Touchpad -----------------------------------------------------------------
  const pad = $("pad"), touch = $("touch"), flash = $("flash");
  const touches = new Map();
  const LONG = 600; // the launcher's long press
  let padState = null, pending = { dx: 0, dy: 0, scroll: false }, frame = 0, flashTimer = 0;

  function showFlash(text) {
    flash.textContent = text;
    flash.classList.add("on");
    clearTimeout(flashTimer);
    flashTimer = setTimeout(() => flash.classList.remove("on"), 600);
  }

  function pointerMode() {
    return view?.type === "app" && view.pointer;
  }

  function flushPointer() {
    frame = 0;
    const clamp = (n) => Math.max(-32768, Math.min(32767, Math.round(n)));
    if (pending.dx || pending.dy) {
      send({ type: "pointer", dx: clamp(pending.dx), dy: clamp(pending.dy), action: pending.scroll ? "scroll" : "move" });
    }
    pending = { dx: 0, dy: 0, scroll: false };
  }

  pad.addEventListener("pointerdown", (event) => {
    try {
      pad.setPointerCapture(event.pointerId);
    } catch {
      // Not a live pointer (synthetic events): movement still arrives.
    }
    touches.set(event.pointerId, { x: event.clientX, y: event.clientY });
    const box = pad.getBoundingClientRect();
    touch.style.left = `${event.clientX - box.left}px`;
    touch.style.top = `${event.clientY - box.top}px`;
    touch.style.opacity = "1";
    if (touches.size === 1) {
      padState = { x: event.clientX, y: event.clientY, startX: event.clientX, startY: event.clientY, at: performance.now(), moved: false };
      // Only shown here; the long press is sent on release, if the finger
      // never moved (a resting finger that then swipes is a swipe).
      padState.timer = setTimeout(() => {
        if (padState && !padState.moved && !pointerMode()) showFlash(t("flash_long"));
      }, LONG);
    }
  });

  pad.addEventListener("pointermove", (event) => {
    const last = touches.get(event.pointerId);
    if (!last || !padState) return;
    const dx = event.clientX - last.x, dy = event.clientY - last.y;
    touches.set(event.pointerId, { x: event.clientX, y: event.clientY });
    const box = pad.getBoundingClientRect();
    touch.style.left = `${event.clientX - box.left}px`;
    touch.style.top = `${event.clientY - box.top}px`;
    if (Math.hypot(event.clientX - padState.startX, event.clientY - padState.startY) > 10) {
      padState.moved = true;
      clearTimeout(padState.timer);
    }
    if (pointerMode()) {
      // Two fingers scroll; their movement is counted once.
      const scroll = touches.size >= 2;
      if (scroll && touches.keys().next().value !== event.pointerId) return;
      pending.dx += dx * 2;
      pending.dy += dy * 2;
      pending.scroll = scroll;
      if (!frame) frame = requestAnimationFrame(flushPointer);
      return;
    }
    // Arrows: one key per 48 px of travel, so a long swipe keeps going.
    const tx = event.clientX - padState.x, ty = event.clientY - padState.y;
    if (Math.max(Math.abs(tx), Math.abs(ty)) >= 48) {
      const key = Math.abs(tx) > Math.abs(ty) ? (tx > 0 ? "right" : "left") : ty > 0 ? "down" : "up";
      press(key);
      showFlash(t(key));
      padState.x = event.clientX;
      padState.y = event.clientY;
    }
  });

  function padUp(event) {
    touches.delete(event.pointerId);
    if (touches.size > 0 || !padState) return;
    touch.style.opacity = "0";
    clearTimeout(padState.timer);
    const held = performance.now() - padState.at;
    if (!padState.moved && held >= LONG && !pointerMode()) {
      press("ok", "long");
    } else if (!padState.moved && held < 400) {
      if (pointerMode()) {
        send({ type: "pointer", dx: 0, dy: 0, action: "click" });
        showFlash(t("flash_click"));
      } else {
        press("ok");
        showFlash("OK");
      }
    }
    padState = null;
  }
  pad.addEventListener("pointerup", padUp);
  pad.addEventListener("pointercancel", padUp);

  // --- Typing -------------------------------------------------------------------
  let typed = "", typingAsked = false;
  const input = $("typing-input");

  function openTyping(free = false) {
    typingAsked = !free;
    const field = view?.text;
    $("typing-eyebrow").textContent = t(field ? "asking" : "typing_free");
    $("typing-label").textContent = field?.label || t("text_label");
    typed = field?.value || "";
    input.value = typed;
    $("typing").hidden = false;
    input.focus();
  }

  function closeTyping() {
    $("typing").hidden = true;
    input.blur();
  }

  // Follows the TV's value while the phone isn't typing.
  function syncTyping(field) {
    $("typing-label").textContent = field.label || t("text_label");
    if (document.activeElement !== input) {
      typed = field.value || "";
      input.value = typed;
    }
  }

  function sendDelete(count) {
    while (count > 0) {
      const n = Math.min(count, MAX_TEXT);
      send({ type: "text", edit: { op: "delete", count: n } });
      count -= n;
    }
  }

  function sendInsert(text) {
    const clean = text.replace(/[\u0000-\u001f\u007f-\u009f]/g, "");
    let chunk = "";
    for (const c of clean) {
      if (encoder.encode(chunk + c).length > MAX_TEXT) {
        send({ type: "text", edit: { op: "insert", text: chunk } });
        chunk = "";
      }
      chunk += c;
    }
    if (chunk) send({ type: "text", edit: { op: "insert", text: chunk } });
  }

  // Sends the change since the last value: the common prefix stays, the rest
  // is deleted and typed again (the TV's cursor is at the end).
  input.addEventListener("input", () => {
    const now = [...input.value], before = [...typed];
    let same = 0;
    while (same < now.length && same < before.length && now[same] === before[same]) same++;
    sendDelete(before.length - same);
    sendInsert(now.slice(same).join(""));
    typed = input.value;
  });
  $("typing-clear").addEventListener("click", () => {
    sendDelete([...typed].length);
    typed = "";
    input.value = "";
    input.focus();
  });
  $("typing-form").addEventListener("submit", (event) => {
    event.preventDefault();
    send({ type: "text", edit: { op: "submit" } });
    closeTyping();
  });
  $("typing-close").addEventListener("click", closeTyping);
  $("keyboard").addEventListener("click", () => openTyping(!view?.text));
  $("wants").addEventListener("click", () => openTyping(false));

  // iOS Safari shows :active only on pages that listen for touches.
  document.addEventListener("touchstart", () => {}, { passive: true });

  // --- The phone's language -----------------------------------------------------
  document.documentElement.lang = window.RustTVText.lang.replace("_", "-");
  document.documentElement.dir = window.RustTVText.rtl ? "rtl" : "ltr";
  for (const element of document.querySelectorAll("[data-t]")) element.textContent = t(element.dataset.t);
  for (const element of document.querySelectorAll("[data-t-label]")) element.setAttribute("aria-label", t(element.dataset.tLabel));

  // --- Add to Home Screen ---------------------------------------------------------
  // Suggested once paired (never with the pairing link in the address, which
  // the Home Screen would keep), until "Got it", or a week after "Not now".
  // "-2": icons added before the keys travelled with them start unpaired,
  // so every phone is offered the hint once more.
  const INSTALL = "rusttv-remote-install-2";
  function standalone() {
    return matchMedia("(display-mode: standalone)").matches || navigator.standalone === true;
  }
  const WEEK = 7 * 24 * 3600 * 1000;
  // `asked`: the person tapped the header button, so "Not now" and
  // "Got it" don't hold it back.
  function suggestInstall(asked = false) {
    const ios = /iPhone|iPad|iPod/.test(navigator.userAgent);
    const ours = location.hash.startsWith("#h=");
    if (standalone() || (location.hash && !ours) || (!ios && !/Android/.test(navigator.userAgent))) return;
    $("install-open").hidden = false;
    let seen = null;
    try {
      seen = JSON.parse(localStorage.getItem(INSTALL));
    } catch {
      // Unreadable: suggest again.
    }
    if (!asked && seen && (seen.done || Date.now() - seen.at < WEEK)) return;
    $("trust").hidden = true;
    $("install-text").textContent = t(ios ? "install_ios" : "install_android");
    $("install").hidden = false;
  }
  function closeInstall(done) {
    $("install").hidden = true;
    try {
      localStorage.setItem(INSTALL, JSON.stringify({ done, at: Date.now() }));
    } catch {
      // Private browsing: suggested again next time.
    }
  }
  $("install-ok").addEventListener("click", () => closeInstall(true));
  $("install-open").addEventListener("click", () => suggestInstall(true));
  $("install-later").addEventListener("click", () => closeInstall(false));

  // --- Trusting the TV (REMOTE.md § 2) ------------------------------------------
  // Over plain HTTP, a paired phone is offered to trust the TV's certificate
  // once; from then on the HTTPS page takes over, with the phone's keys.
  // Whether the phone trusts it: the HTTPS page answers only then.
  const SECURE_PORT = document.querySelector('meta[name="rusttv-secure-port"]')?.content || "";
  const TRUST = "rusttv-remote-trust";
  let trustOffered = false, trustStarted = false;
  async function phoneTrustsTv() {
    try {
      await fetch(`https://${location.hostname}:${SECURE_PORT}/ok`, { mode: "no-cors", cache: "no-store" });
      return true;
    } catch {
      return false;
    }
  }
  async function checkTrust() {
    if (window.isSecureContext || !SECURE_PORT || !credentials) {
      suggestInstall();
      return;
    }
    $("trust-open").hidden = false;
    if (await phoneTrustsTv()) {
      const { phone_id: id, auth, key } = credentials;
      location.replace(`https://${location.hostname}:${SECURE_PORT}/#k=${id}.${auth}.${key}`);
      return;
    }
    let seen = null;
    try {
      seen = JSON.parse(localStorage.getItem(TRUST));
    } catch {
      // Unreadable: offer again.
    }
    // "Not now" holds for a week, and only for this pairing: paired again
    // (after the TV removed the phone), it is offered again.
    if (seen && seen.phone === credentials.phone_id && Date.now() - seen.at < WEEK) {
      suggestInstall();
    } else if (!trustOffered) {
      trustOffered = true;
      $("trust").hidden = false;
    }
  }
  $("trust-start").addEventListener("click", () => {
    trustStarted = true;
    // The TV shows the code to compare before installing.
    send({ type: "trust" });
    const ios = /iPhone|iPad|iPod/.test(navigator.userAgent) || (navigator.maxTouchPoints > 1 && /Mac/.test(navigator.userAgent));
    const steps = t(ios ? "trust_ios" : "trust_android").split("\n");
    // On iOS only Safari hands a certificate to Settings; other browsers
    // (Chrome, Firefox, Edge…) just save the file. Their Download opens
    // Safari, and the step names the address in case it doesn't.
    if (ios && /CriOS|FxiOS|EdgiOS|OPiOS|GSA\/|DuckDuckGo|YaBrowser/.test(navigator.userAgent)) {
      const certificate = `http://${location.host}/tv.crt`;
      steps[0] = t("trust_in_safari", certificate);
      // iOS opens Safari from a link only for https: Safari warns once,
      // since it doesn't trust the TV yet (the step says what to tap).
      $("trust-download").href = `x-safari-https://${location.hostname}:${SECURE_PORT}/tv.crt`;
    }
    $("trust-steps").replaceChildren(...steps.map((step) => {
      const item = document.createElement("li");
      item.textContent = step;
      return item;
    }));
    for (const id of ["trust-steps", "trust-download", "trust-waiting"]) $(id).hidden = false;
    $("trust-intro").hidden = true;
    $("trust-start").hidden = true;
  });
  $("trust-download").addEventListener("click", () => haptic(8));
  $("trust-open").addEventListener("click", () => {
    $("install").hidden = true;
    $("trust").hidden = false;
  });
  $("trust-later").addEventListener("click", () => {
    $("trust").hidden = true;
    try {
      localStorage.setItem(TRUST, JSON.stringify({ at: Date.now(), phone: credentials?.phone_id }));
    } catch {
      // Private browsing: offered again next time.
    }
  });
  // Back from the phone's Settings: if it trusts the TV now, move.
  document.addEventListener("visibilitychange", async () => {
    if (!document.hidden && trustStarted && credentials && (await phoneTrustsTv())) checkTrust();
  });

  start();
})();
