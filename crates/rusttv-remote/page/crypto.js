// RustTV Remote: the phone's side of the crypto (REMOTE.md § 2), in plain
// JavaScript because Web Crypto needs a secure context and the page is
// served over http. Must give exactly the bytes `rusttv-remote` gives:
// page/crypto.test.mjs checks it against the crate's test vectors.
//
// SHA-256 (FIPS 180-4), HMAC-SHA256 (RFC 2104), ChaCha20 and Poly1305
// (RFC 8439), HChaCha20 and XChaCha20-Poly1305 (draft-irtf-cfrg-xchacha).
// Poly1305 uses BigInt: messages are at most 64 KB, so speed doesn't matter,
// and the phone only checks the TV's tags.
"use strict";
(function (root) {
  const enc = new TextEncoder();

  function concat(...parts) {
    let length = 0;
    for (const part of parts) length += part.length;
    const out = new Uint8Array(length);
    let at = 0;
    for (const part of parts) {
      out.set(part, at);
      at += part.length;
    }
    return out;
  }

  function hex(bytes) {
    let text = "";
    for (const byte of bytes) text += byte.toString(16).padStart(2, "0");
    return text;
  }

  function unhex(text, length) {
    if (typeof text !== "string" || text.length !== 2 * length || !/^[0-9a-f]*$/.test(text)) return null;
    const out = new Uint8Array(length);
    for (let i = 0; i < length; i++) out[i] = parseInt(text.substr(2 * i, 2), 16);
    return out;
  }

  // --- SHA-256 ---------------------------------------------------------------
  const K = new Uint32Array([
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
  ]);

  function sha256(data) {
    const h = new Uint32Array([
      0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ]);
    const bits = data.length * 8;
    const padded = new Uint8Array(Math.ceil((data.length + 9) / 64) * 64);
    padded.set(data);
    padded[data.length] = 0x80;
    const view = new DataView(padded.buffer);
    view.setUint32(padded.length - 8, Math.floor(bits / 0x100000000));
    view.setUint32(padded.length - 4, bits >>> 0);
    const w = new Uint32Array(64);
    for (let block = 0; block < padded.length; block += 64) {
      for (let i = 0; i < 16; i++) w[i] = view.getUint32(block + 4 * i);
      for (let i = 16; i < 64; i++) {
        const a = w[i - 15], b = w[i - 2];
        const s0 = ((a >>> 7) | (a << 25)) ^ ((a >>> 18) | (a << 14)) ^ (a >>> 3);
        const s1 = ((b >>> 17) | (b << 15)) ^ ((b >>> 19) | (b << 13)) ^ (b >>> 10);
        w[i] = (w[i - 16] + s0 + w[i - 7] + s1) >>> 0;
      }
      let [a, b, c, d, e, f, g, hh] = h;
      for (let i = 0; i < 64; i++) {
        const S1 = ((e >>> 6) | (e << 26)) ^ ((e >>> 11) | (e << 21)) ^ ((e >>> 25) | (e << 7));
        const ch = (e & f) ^ (~e & g);
        const t1 = (hh + S1 + ch + K[i] + w[i]) >>> 0;
        const S0 = ((a >>> 2) | (a << 30)) ^ ((a >>> 13) | (a << 19)) ^ ((a >>> 22) | (a << 10));
        const maj = (a & b) ^ (a & c) ^ (b & c);
        const t2 = (S0 + maj) >>> 0;
        hh = g; g = f; f = e; e = (d + t1) >>> 0;
        d = c; c = b; b = a; a = (t1 + t2) >>> 0;
      }
      h[0] += a; h[1] += b; h[2] += c; h[3] += d; h[4] += e; h[5] += f; h[6] += g; h[7] += hh;
    }
    const out = new Uint8Array(32);
    const outView = new DataView(out.buffer);
    for (let i = 0; i < 8; i++) outView.setUint32(4 * i, h[i]);
    return out;
  }

  function hmac(key, data) {
    if (key.length > 64) key = sha256(key);
    const inner = new Uint8Array(64), outer = new Uint8Array(64);
    inner.set(key);
    outer.set(key);
    for (let i = 0; i < 64; i++) {
      inner[i] ^= 0x36;
      outer[i] ^= 0x5c;
    }
    return sha256(concat(outer, sha256(concat(inner, data))));
  }

  // The crate's MAC: a domain label, a zero byte, then the parts.
  function mac(key, label, ...parts) {
    return hmac(key, concat(enc.encode(label), new Uint8Array([0]), ...parts));
  }

  // --- ChaCha20, HChaCha20 ---------------------------------------------------
  function rounds(s) {
    function qr(a, b, c, d) {
      s[a] = (s[a] + s[b]) >>> 0; s[d] ^= s[a]; s[d] = (s[d] << 16) | (s[d] >>> 16);
      s[c] = (s[c] + s[d]) >>> 0; s[b] ^= s[c]; s[b] = (s[b] << 12) | (s[b] >>> 20);
      s[a] = (s[a] + s[b]) >>> 0; s[d] ^= s[a]; s[d] = (s[d] << 8) | (s[d] >>> 24);
      s[c] = (s[c] + s[d]) >>> 0; s[b] ^= s[c]; s[b] = (s[b] << 7) | (s[b] >>> 25);
    }
    for (let i = 0; i < 10; i++) {
      qr(0, 4, 8, 12); qr(1, 5, 9, 13); qr(2, 6, 10, 14); qr(3, 7, 11, 15);
      qr(0, 5, 10, 15); qr(1, 6, 11, 12); qr(2, 7, 8, 13); qr(3, 4, 9, 14);
    }
  }

  function words(bytes) {
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.length);
    const out = new Uint32Array(bytes.length / 4);
    for (let i = 0; i < out.length; i++) out[i] = view.getUint32(4 * i, true);
    return out;
  }

  function state(key, counter, nonce) {
    const s = new Uint32Array(16);
    s.set([0x61707865, 0x3320646e, 0x79622d32, 0x6b206574]);
    s.set(words(key), 4);
    s[12] = counter;
    s.set(words(nonce), 13);
    return s;
  }

  function hchacha20(key, nonce16) {
    const s = new Uint32Array(16);
    s.set([0x61707865, 0x3320646e, 0x79622d32, 0x6b206574]);
    s.set(words(key), 4);
    s.set(words(nonce16), 12);
    rounds(s);
    const out = new Uint8Array(32);
    const view = new DataView(out.buffer);
    for (let i = 0; i < 4; i++) {
      view.setUint32(4 * i, s[i], true);
      view.setUint32(16 + 4 * i, s[12 + i], true);
    }
    return out;
  }

  function chachaBlock(key, counter, nonce) {
    const s = state(key, counter, nonce);
    const x = s.slice();
    rounds(x);
    const out = new Uint8Array(64);
    const view = new DataView(out.buffer);
    for (let i = 0; i < 16; i++) view.setUint32(4 * i, (x[i] + s[i]) >>> 0, true);
    return out;
  }

  function chacha20(key, counter, nonce, data) {
    const out = new Uint8Array(data.length);
    for (let at = 0; at < data.length; at += 64, counter++) {
      const block = chachaBlock(key, counter, nonce);
      for (let i = 0; i < 64 && at + i < data.length; i++) out[at + i] = data[at + i] ^ block[i];
    }
    return out;
  }

  // --- Poly1305 --------------------------------------------------------------
  function le(bytes) {
    let n = 0n;
    for (let i = bytes.length - 1; i >= 0; i--) n = (n << 8n) | BigInt(bytes[i]);
    return n;
  }

  function poly1305(key, message) {
    const p = (1n << 130n) - 5n;
    const r = le(key.subarray(0, 16)) & 0x0ffffffc0ffffffc0ffffffc0fffffffn;
    const s = le(key.subarray(16, 32));
    let acc = 0n;
    for (let at = 0; at < message.length; at += 16) {
      const chunk = message.subarray(at, Math.min(at + 16, message.length));
      acc = ((acc + le(chunk) + (1n << BigInt(8 * chunk.length))) * r) % p;
    }
    acc = (acc + s) & ((1n << 128n) - 1n);
    const tag = new Uint8Array(16);
    for (let i = 0; i < 16; i++) {
      tag[i] = Number(acc & 0xffn);
      acc >>= 8n;
    }
    return tag;
  }

  function pad16(length) {
    return new Uint8Array((16 - (length % 16)) % 16);
  }

  function lengths(aad, ciphertext) {
    const out = new Uint8Array(16);
    const view = new DataView(out.buffer);
    view.setUint32(0, aad.length, true);
    view.setUint32(8, ciphertext.length, true);
    return out;
  }

  function tagOf(key, nonce, aad, ciphertext) {
    const polyKey = chachaBlock(key, 0, nonce).subarray(0, 32);
    return poly1305(
      polyKey,
      concat(aad, pad16(aad.length), ciphertext, pad16(ciphertext.length), lengths(aad, ciphertext)),
    );
  }

  function xparts(key, nonce24) {
    return [hchacha20(key, nonce24.subarray(0, 16)), concat(new Uint8Array(4), nonce24.subarray(16, 24))];
  }

  // XChaCha20-Poly1305: ciphertext || tag.
  function xseal(key, nonce24, plaintext, aad) {
    const [subkey, nonce] = xparts(key, nonce24);
    const ciphertext = chacha20(subkey, 1, nonce, plaintext);
    return concat(ciphertext, tagOf(subkey, nonce, aad, ciphertext));
  }

  // The plaintext, or null if the tag is wrong.
  function xopen(key, nonce24, sealed, aad) {
    if (sealed.length < 16) return null;
    const [subkey, nonce] = xparts(key, nonce24);
    const ciphertext = sealed.subarray(0, sealed.length - 16);
    const tag = tagOf(subkey, nonce, aad, ciphertext);
    let diff = 0;
    for (let i = 0; i < 16; i++) diff |= tag[i] ^ sealed[sealed.length - 16 + i];
    return diff === 0 ? chacha20(subkey, 1, nonce, ciphertext) : null;
  }

  // --- RustTV Remote ---------------------------------------------------------
  const PHONE_TO_TV = 1, TV_TO_PHONE = 2;

  function aad(direction, number) {
    const out = new Uint8Array(9);
    out[0] = direction;
    new DataView(out.buffer).setBigUint64(1, BigInt(number));
    return out;
  }

  // The phone's side of a connection (crate: `Session::phone`).
  class Session {
    constructor(key) {
      this.key = key;
      this.sent = 0;
      this.received = 0;
    }

    seal(plaintext, nonce24) {
      const sealed = xseal(this.key, nonce24, plaintext, aad(PHONE_TO_TV, this.sent));
      this.sent++;
      return concat(nonce24, sealed);
    }

    // The plaintext, or null; after null the connection must close.
    open(frame) {
      if (frame.length < 40) return null;
      const plaintext = xopen(this.key, frame.subarray(0, 24), frame.subarray(24), aad(TV_TO_PHONE, this.received));
      if (plaintext) this.received++;
      return plaintext;
    }
  }

  root.RustTVCrypto = {
    hex,
    unhex,
    sha256,
    hmac,
    hchacha20,
    xseal,
    xopen,
    Session,
    pairingProof: (secret, nonce, name) => mac(secret, "rusttv-remote/1/pair", nonce, enc.encode(name)),
    matchCode(secret, nonce) {
      const bytes = mac(secret, "rusttv-remote/1/code", nonce);
      return ((bytes[0] << 8) | bytes[1]) % 100;
    },
    phoneKeys: (secret, nonce) => ({
      auth: mac(secret, "rusttv-remote/1/auth-key", nonce),
      key: mac(secret, "rusttv-remote/1/session-seed", nonce),
    }),
    authResponse: (auth, challenge, phoneId) => mac(auth, "rusttv-remote/1/auth", challenge, phoneId),
    sessionKey: (key, challenge) => mac(key, "rusttv-remote/1/session", challenge),
  };
})(globalThis);
