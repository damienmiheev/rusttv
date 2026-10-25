// Checks page/crypto.js against the test vectors that pin the Rust side
// (src/auth.rs, src/crypto.rs) and published ones. Run: node page/crypto.test.mjs
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import vm from "node:vm";
import assert from "node:assert/strict";

const context = { TextEncoder, Uint8Array, DataView, BigInt };
context.globalThis = context;
vm.createContext(context);
vm.runInContext(readFileSync(fileURLToPath(new URL("crypto.js", import.meta.url)), "utf8"), context);
const C = context.RustTVCrypto;
const enc = new TextEncoder();
const fill = (byte, length) => new Uint8Array(length).fill(byte);
let checks = 0;
const eq = (actual, expected, what) => {
  assert.equal(actual, expected, what);
  checks++;
};

// SHA-256 and HMAC-SHA256: FIPS 180-4 examples and RFC 4231 test case 2.
eq(C.hex(C.sha256(enc.encode("abc"))), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad", "sha256 abc");
eq(C.hex(C.sha256(new Uint8Array(0))), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", "sha256 empty");
eq(
  C.hex(C.sha256(enc.encode("abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"))),
  "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
  "sha256 two blocks",
);
eq(
  C.hex(C.hmac(enc.encode("Jefe"), enc.encode("what do ya want for nothing?"))),
  "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
  "hmac rfc 4231",
);

// HChaCha20: draft-irtf-cfrg-xchacha § 2.2.1.
const key0 = Uint8Array.from({ length: 32 }, (_, i) => i);
eq(
  C.hex(C.hchacha20(key0, C.unhex("000000090000004a0000000031415927", 16))),
  "82413b4227b27bfed30e42508a877d73a0f9e4d58a74a853c12ec41326d3ecdc",
  "hchacha20",
);

// The crate's vectors (src/auth.rs test_vectors).
const SECRET = fill(0x11, 32), NONCE = fill(0x22, 32), CHALLENGE = fill(0x33, 32), ID = fill(0x44, 16);
eq(C.hex(C.pairingProof(SECRET, NONCE, "Pixel 7")), "a98e0ad09f9b1d9403972c024af0bfd9c3a18cf431547dcba65c845d4705d94e", "pairing proof");
eq(C.matchCode(SECRET, NONCE), 99, "match code");
const keys = C.phoneKeys(SECRET, NONCE);
eq(C.hex(keys.auth), "022c6e5321ecdd6d5f573853b9b8d58e16320a6cdb45c0fedeeba28c86d0a867", "auth key");
eq(C.hex(keys.key), "2cccd5037566ae24a4921e84b37296e87e59a923f26fdd40b1143b08fd1d5268", "session seed");
eq(C.hex(C.authResponse(keys.auth, CHALLENGE, ID)), "8619ebc32fc9c6ffdd6e2c4f1eba95bef2dad871104daaab7513ef70798aa80d", "auth response");
eq(C.hex(C.sessionKey(keys.key, CHALLENGE)), "0d42934d0f8d09a23873d11d8a56cd1f8a9bf3d9604f0c2e96f18074661d735f", "session key");
eq(C.hex(C.handoverProof(SECRET, NONCE, ID)), "24fe519bc7c508e33d69669a79d3824fc9b03837fd75866f8806175b53fb621c", "handover proof");
eq(C.hex(C.handoverKey(SECRET, NONCE)), "3db785d4f42218527494c55f3b5543ba7fa14668388f011e92a62d6dbaeb74cc", "handover key");

// The crate's sealed frame (src/crypto.rs test_vector).
const phone = new C.Session(fill(0x55, 32));
const frame = phone.seal(enc.encode('{"type":"key","name":"ok","action":"press"}'), fill(1, 24));
eq(
  C.hex(frame),
  "010101010101010101010101010101010101010101010101" +
    "e6b154a858658e5a5d1750775536fe971a08c1139d4e3b06d6fd6f104e473012" +
    "c9818219f884d938b3952f96b3912fd7be73a75ccfc98a9283641a",
  "sealed frame",
);

// Opening: a TV frame opens once, in order; tampering and replay fail.
const tvKey = fill(0x55, 32);
const tvFrame = (n, text, nonceByte) => {
  const aad = new Uint8Array(9);
  aad[0] = 2;
  new DataView(aad.buffer).setBigUint64(1, BigInt(n));
  const nonce = fill(nonceByte, 24);
  const out = new Uint8Array(24 + text.length + 16);
  out.set(nonce);
  out.set(C.xseal(tvKey, nonce, enc.encode(text), aad), 24);
  return out;
};
const reader = new C.Session(tvKey);
const first = tvFrame(0, '{"type":"ok"}', 9);
eq(new TextDecoder().decode(reader.open(first)), '{"type":"ok"}', "open first");
eq(reader.open(first), null, "replay refused");
const second = tvFrame(1, "{}", 10);
second[second.length - 1] ^= 1;
eq(reader.open(second), null, "tamper refused");
eq(C.unhex("ABCD", 2), null, "uppercase hex refused");

// A long message (several Poly1305 blocks and ChaCha20 blocks) round-trips.
const long = enc.encode("x".repeat(5000));
const sealed = C.xseal(tvKey, fill(3, 24), long, new Uint8Array(0));
eq(C.xopen(tvKey, fill(3, 24), sealed, new Uint8Array(0)).length, 5000, "long round trip");

console.log(`crypto.js: ${checks} checks passed`);
