// RustTV Remote: the phone page (REMOTE.md § 1, § 4). Pairs once from the QR
// code's link, then connects, authenticates and talks to the TV in sealed
// messages. Renders what the TV sends: the Home mirror or, while another app
// is in front, the touchpad. Everything the TV sends is shown as text, never
// as HTML.
"use strict";
(() => {
  const C = window.RustTVCrypto;
  const $ = (id) => document.getElementById(id);
  const encoder = new TextEncoder(), decoder = new TextDecoder();
  const random = (length) => crypto.getRandomValues(new Uint8Array(length));
  const STORE = "rusttv-remote";
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

  let credentials = store.get();
  let pairing = null; // { secret, nonce, name } while pairing
  let socket = null, session = null, challenge = null;
  let retry = 0, retryTimer = 0;
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
    if (credentials) {
      show("main");
      connect();
    } else {
      show("scan");
    }
  }

  $("pair-form").addEventListener("submit", (event) => {
    event.preventDefault();
    if (!pairing || pairing.nonce) return;
    const name = cleanName($("phone-name").value);
    if (!name) {
      $("pair-status").textContent = "Give this phone a name (up to 40 characters).";
      return;
    }
    pairing.name = name;
    pairing.nonce = random(32);
    $("code").textContent = String(C.matchCode(pairing.secret, pairing.nonce)).padStart(2, "0");
    $("name-field").hidden = true;
    $("pair-button").hidden = true;
    $("pair-code").hidden = false;
    $("pair-status").textContent = "Waiting for the TV…";
    connect();
  });

  // --- Connection -----------------------------------------------------------
  function connect() {
    clearTimeout(retryTimer);
    if (socket) return;
    socket = new WebSocket(`ws://${location.host}/ws`);
    socket.binaryType = "arraybuffer";
    socket.onopen = () => {
      if (pairing?.nonce) {
        sendText({
          type: "pair",
          phone_nonce: C.hex(pairing.nonce),
          name: pairing.name,
          proof: C.hex(C.pairingProof(pairing.secret, pairing.nonce, pairing.name)),
        });
      } else if (credentials) {
        sendText({ type: "hello", phone_id: credentials.phone_id });
      }
    };
    socket.onmessage = (event) => (typeof event.data === "string" ? onText(event.data) : onSealed(event.data));
    socket.onclose = () => {
      socket = null;
      session = null;
      setStatus(false, "Reconnecting…");
      if (pairing?.nonce) {
        failPairing("The connection to the TV broke. Open “Pair a phone” on the TV and scan again.");
      } else if (credentials && !document.hidden) {
        retry = Math.min(retry + 1, 6);
        retryTimer = setTimeout(connect, 500 * 2 ** retry);
      }
    };
  }

  function sendText(message) {
    socket?.send(JSON.stringify(message));
  }

  function send(message) {
    if (!session || socket?.readyState !== WebSocket.OPEN) return;
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
      challenge = null;
      retry = 0;
      setStatus(true, "Connected");
    } else if (message.type === "denied") {
      if (pairing?.nonce) {
        failPairing("The TV refused, or the code ran out. Open “Pair a phone” on the TV and scan again.");
      } else {
        // The TV doesn't know this phone any more (it was removed in Settings).
        credentials = null;
        store.clear();
        $("scan-text").textContent = "This phone isn't paired with the TV any more. On the TV, open Settings → RustTV Remote → Pair a phone, then scan the code.";
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

  document.addEventListener("visibilitychange", () => {
    if (!document.hidden && credentials && !socket && !pairing) connect();
  });

  // --- Views ------------------------------------------------------------------
  function setStatus(on, text) {
    $("dot").classList.toggle("on", on);
    $("status").textContent = text;
  }

  function onView(message) {
    if (message.type !== "home" && message.type !== "app") return;
    view = message;
    if (message.type !== lastKind) {
      lastKind = message.type;
      setTab(message.type === "home" ? "apps" : "touchpad");
    }
    setStatus(true, message.type === "home" ? "RustTV is in front" : `${message.app || "Another app"} is in front`);
    render();
  }

  function setTab(name) {
    tab = name;
    $("tab-apps").setAttribute("aria-selected", String(name === "apps"));
    $("tab-touchpad").setAttribute("aria-selected", String(name === "touchpad"));
    $("apps").hidden = name !== "apps";
    $("touchpad").hidden = name !== "touchpad";
    render();
  }
  $("tab-apps").addEventListener("click", () => setTab("apps"));
  $("tab-touchpad").addEventListener("click", () => setTab("touchpad"));

  // A banner stand-in until banners travel: a colour from the tile's id.
  function colour(id) {
    let hash = 0;
    for (const c of id) hash = (hash * 31 + c.charCodeAt(0)) >>> 0;
    return `hsl(${hash % 360} 45% 30%)`;
  }

  function art(tile, tiles) {
    const box = document.createElement("div");
    box.className = "art";
    if (tile.id.startsWith("folder:")) {
      box.style.background = "#2A2E37";
      const minis = document.createElement("div");
      minis.className = "minis";
      for (const inner of tiles.filter((t) => t.folder === tile.id).slice(0, 4)) {
        const mini = document.createElement("i");
        mini.style.background = colour(inner.id);
        minis.append(mini);
      }
      box.append(minis);
    } else {
      const builtin = ["about", "settings", "hidden-apps"].includes(tile.id);
      box.style.background = builtin
        ? "linear-gradient(150deg, #FFC35C, #F5A524 42%, #D9480F)"
        : `linear-gradient(150deg, ${colour(tile.id)}, #0E1014 140%)`;
      const label = document.createElement("span");
      label.textContent = tile.label;
      box.append(label);
    }
    return box;
  }

  function tileButton(tile, tiles, focused) {
    const button = document.createElement("button");
    button.className = "tile" + (tile.id === focused ? " focused" : "");
    button.dataset.id = tile.id;
    button.setAttribute("aria-label", tile.label);
    button.append(art(tile, tiles));
    const name = document.createElement("div");
    name.className = "name";
    name.textContent = tile.label;
    button.append(name);
    return button;
  }

  function render() {
    if (!view) return;
    const home = view.type === "home";
    const tiles = home ? view.tiles || [] : [];
    const focused = home ? view.focused : null;
    const focusedTile = tiles.find((t) => t.id === focused);
    $("hero").textContent = focusedTile ? focusedTile.label : "";
    $("glow").style.setProperty("--glow", focusedTile ? colour(focusedTile.id).replace("30%)", "30% / 0.35)") : "#F5A52426");

    if (tab === "apps" && !dragging) {
      const grid = $("grid");
      grid.replaceChildren(...tiles.filter((t) => !t.folder).map((t) => tileButton(t, tiles, focused)));
      const folder = home && view.folder;
      $("folder").hidden = !folder;
      if (folder) {
        $("folder-name").textContent = folder.name;
        $("folder-grid").replaceChildren(...tiles.filter((t) => t.folder === folder.id).map((t) => tileButton(t, tiles, focused)));
      }
    }
    const moving = home && view.mode === "moving";
    $("moving").hidden = !moving;
    if (moving) {
      const moved = (gesture && tiles.find((t) => t.id === gesture.id)) || focusedTile;
      $("moving-title").textContent = `Moving ${moved ? moved.label : "an app"}`;
    }

    const pointer = !home && view.pointer;
    $("mode").textContent = pointer ? "Pointer" : "Arrows";
    $("mode").className = "mode" + (pointer ? " pointer" : "");
    $("pad-hint").textContent = pointer
      ? "One finger moves the pointer · tap to click · two fingers scroll"
      : home
        ? "Swipe to move between apps · tap for OK"
        : "Swipes are arrow keys here · tap for OK · press and hold for a long press";
    const field = view.text;
    $("wants").hidden = !field;
    if (field) $("wants-text").textContent = `${field.label || "A text field"} wants text`;
    if (field && !$("typing").hidden) syncTyping(field);
    if (field && home && view.mode === "text" && $("typing").hidden) openTyping();
    if (!field && !$("typing").hidden && typingAsked) closeTyping();
  }

  // --- Keys -------------------------------------------------------------------
  function press(name, action = "press") {
    if (navigator.vibrate) navigator.vibrate(8);
    send({ type: "key", name, action });
  }
  document.addEventListener("click", (event) => {
    const button = event.target.closest("[data-key]");
    if (button) press(button.dataset.key);
  });

  // --- Tiles: tap, hold, drag ---------------------------------------------------
  const HOLD = 450, SLOP = 12;
  let gesture = null, dragging = false;

  function tileAt(x, y) {
    const element = document.elementFromPoint(x, y)?.closest("[data-id]");
    return element ? element.dataset.id : null;
  }

  function onTileDown(event) {
    const tile = event.target.closest(".tile");
    if (!tile || !session) return;
    gesture = { id: tile.dataset.id, element: tile, x: event.clientX, y: event.clientY, held: false, over: tile.dataset.id, pointer: event.pointerId };
    gesture.timer = setTimeout(() => {
      if (!gesture) return;
      gesture.held = true;
      if (navigator.vibrate) navigator.vibrate(15);
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
      if (over !== gesture.over) {
        document.querySelector(".tile.over")?.classList.remove("over");
        if (over && over !== gesture.id) document.querySelector(`.tile[data-id="${CSS.escape(over)}"]`)?.classList.add("over");
        gesture.over = over;
        send({ type: "drag", id: gesture.id, over, phase: "move" });
      }
    }
  }

  function onTileUp(event, cancelled) {
    if (!gesture || event.pointerId !== gesture.pointer) return;
    clearTimeout(gesture.timer);
    if (dragging) {
      send({ type: "drag", id: gesture.id, over: gesture.over, phase: cancelled ? "cancel" : "end" });
      dragging = false;
      $("ghost").hidden = true;
      document.querySelector(".tile.over")?.classList.remove("over");
      gesture.element.classList.remove("dragging");
      gesture = null;
      render();
      return;
    }
    if (!cancelled) send({ type: "tile", id: gesture.id, gesture: gesture.held ? "long_press" : "tap" });
    gesture = null;
  }

  for (const area of [$("apps"), $("folder")]) {
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
        if (padState && !padState.moved && !pointerMode()) showFlash("Long press");
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
      showFlash(key[0].toUpperCase() + key.slice(1));
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
        showFlash("Click");
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
    $("typing-eyebrow").textContent = field ? "The TV is asking for text" : "Typing on the TV";
    $("typing-label").textContent = field?.label || "Text";
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
    $("typing-label").textContent = field.label || "Text";
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

  start();
})();
