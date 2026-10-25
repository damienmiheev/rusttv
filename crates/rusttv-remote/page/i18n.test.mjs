// Checks page/i18n.js: every language has every text with English's `{}`
// placeholders, every key the page uses exists, and the phone's language
// is picked as the page expects. Run with `node page/i18n.test.mjs`.
import assert from "node:assert/strict";
import fs from "node:fs";

const dir = new URL(".", import.meta.url);
const root = { navigator: { languages: ["uk-UA", "en"] } };
new Function("globalThis", fs.readFileSync(new URL("i18n.js", dir), "utf8"))(root);
const { KEYS, T, pick, t, lang } = root.RustTVText;

const holes = (text) => text.split("{}").length - 1;
assert.equal(new Set(KEYS).size, KEYS.length, "keys are unique");
assert.equal(Object.keys(T).length, 21, "English and the launcher's 20 languages");
for (const [code, texts] of Object.entries(T)) {
  assert.equal(texts.length, KEYS.length, `${code}: one text per key`);
  texts.forEach((text, index) => {
    assert.ok(typeof text === "string" && text.trim(), `${code}: ${KEYS[index]} is empty`);
    assert.equal(holes(text), holes(T.en[index]), `${code}: ${KEYS[index]} placeholders`);
  });
}

// Every key the page uses exists.
const used = new Set();
const html = fs.readFileSync(new URL("index.html", dir), "utf8");
for (const match of html.matchAll(/data-t(?:-label)?="([a-z_]+)"/g)) used.add(match[1]);
const app = fs.readFileSync(new URL("app.js", dir), "utf8");
for (const match of app.matchAll(/\bt\("([a-z_]+)"/g)) used.add(match[1]);
for (const match of app.matchAll(/\bt\([\w.]+ \? "([a-z_]+)" : "([a-z_]+)"\)/g)) used.add(match[1]).add(match[2]);
for (const key of ["up", "down", "left", "right"]) used.add(key); // showFlash(t(key))
for (const key of used) assert.ok(KEYS.includes(key), `unknown key ${key}`);
for (const key of KEYS) assert.ok(used.has(key), `unused key ${key}`);

// The phone's language.
assert.equal(lang, "uk");
assert.equal(t("moving", "Netflix"), "Переміщення: Netflix");
assert.equal(pick(["fr-CA"]), "fr");
assert.equal(pick(["pt-PT"]), "pt_BR");
assert.equal(pick(["zh-Hant-TW"]), "zh_TW");
assert.equal(pick(["zh-HK"]), "zh_TW");
assert.equal(pick(["zh-Hans-CN"]), "zh_CN");
assert.equal(pick(["xx", "de-AT"]), "de");
assert.equal(pick(["xx"]), "en");
console.log(`i18n: ${Object.keys(T).length} languages × ${KEYS.length} texts ok`);
