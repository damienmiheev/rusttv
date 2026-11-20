# Panels step 0 spike: JSON templates

Throwaway (PANELS.md § 4.1, § 10 step 0). Nothing here merges as is.

Question: can one JSON file per template drive both the TV (Slint, generated
at build time) and the phone page (a small JS renderer), and is it worth it?
The test case is About's tile from § 4.1.

## What is here

| File | What |
|---|---|
| `templates/about_tile.json` | About's tile, as in § 4.1 |
| `convert.rs` | JSON → one Slint component; run by `build.rs` |
| `ui/theme.slint` | `Theme` and `Dir` in their own exported file |
| `ui/hand.slint` | The same tile written by hand: the reference |
| `tests/pixels.rs` | Software-renderer pixel comparison, both tiles |
| `tests/convert.rs` | Converter errors and escaping |
| `phone/render.js`, `phone/preview.html` | The same JSON on the phone page |

Run: `cargo test` (pictures land in `target/pixels/`), and for the phone,
serve this folder (`python3 -m http.server -d spikes/panel-templates`) and
open `/phone/preview.html` at phone width.

## Results against the pass bar

1. **The generated tile looks identical to a hand-written one: pass.**
   16 of 16 renders are byte-identical with Slint's software renderer:
   - every update state (none, update, downloading, installing);
   - LTR and RTL;
   - English and Ukrainian.

   Checked by eye too. The first pass rendered every text at one tiny size,
   and both tiles matched anyway, so a pixel match alone proves little. Two
   fixes made the rendering real: system fonts, and no pre-rendered glyphs.

   The test also checks that each RTL picture differs from its LTR one, and
   that translated captions change the picture.

   **Limit:** `hand.slint` is close to what the converter emits, so a match
   mainly shows the converter is faithful and deterministic for this one
   shape. Nested columns, a text `select` with `else`, and several selects
   are converted and error-tested, but never rendered.
2. **The same JSON looks right on the phone page: pass, by eye.** The
   preview puts each phone tile next to the TV's picture. The layout,
   tokens, mirroring and "…" match. Lines sit a few pixels apart, because
   the browser and Slint measure line height differently. Not
   pixel-compared.
3. **`@tr` and `Theme` tokens work through the converter: pass.**
   - The Ukrainian test asserts that translated captions change the picture
     and untranslated ones don't.
   - The phone reads the same `.po` file by message id.
   - `Theme` is reached by importing `ui/theme.slint`, with no copy.
4. **The converter stays small with clear errors: pass, for 3 nodes.**
   - `convert.rs` is 428 lines after `rustfmt`, with the checks the review
     asked for.
   - `render.js` is 118 lines.
   - The page blocks (§ 4) and `row` are not built yet. Under 500 lines for
     the full set is an estimate, not a measurement.
   - Errors name the JSON path, e.g. `root.column.children[1].text: unknown field "left"`.
   - Unknown fields, nodes, colour tokens, badges and props are refused, as
     are weights off the 100 steps and negative sizes.
   - Quotes and backslashes (Slint's `\{…}`) stay text.
   - Refused: braces other than `{}`; control and bidi characters; props
     that clash with Rectangle properties, `u` or each other; component
     names like `Theme` or `Text`.
   - `render.js` applies the same rules, checking every `select` case and
     not only the one taken. The two error suites are not shared yet
     (step 1).

**Verdict: keep JSON.** It passed all four. The cost is two small
renderers that must follow the same rules (below).

## Findings for PANELS.md

Reviewed by an independent pass (2026-11-20). No blockers; its points
are fixed in the spike or listed here.

- **Translation extraction:**
  - Generated files live in `OUT_DIR`, so `slint-tr-extractor` (which
    `translations/README.md` runs on `ui/home.slint` and `ui/quick.slint`)
    would never see template msgids. They would never reach the `.pot`.
  - Step 1 must either write the generated `.slint` to a stable, ignored
    path the extraction step reads, or have the converter list the msgids.
  - Keep `DefaultTranslationContext::None` (as the launcher does), or the
    same text in a template and in `home.slint` becomes two translations.
- **Placeholders differ:** the phone fills only `{}`. Slint's `@tr` also
  takes `{0}` and `{{` in translations. No `.po` uses them today. Either a
  `.po` lint refuses them, or the phone supports them.
- **One error suite:** run the same bad-JSON cases against `convert.rs` and
  `render.js` (node with a small DOM stub), so the two can't drift apart.
- **Duplicate JSON keys** silently take the last value on both sides.
  Refuse them in step 1.

- **Theme:**
  - Move `Theme` and `Dir` out of `home.slint` into an exported
    `theme.slint`, and export `Theme`, which is private today.
  - The generated files import it by path. A copy would make a second
    `Dir.rtl` that never flips, so it must be an import.
  - `quick.slint` keeps its own `start` (on a local `rtl`) and its own
    `global Colours`. Both move onto the shared file in the same change.
- **`Dir` needs `image-start`** (`ImageHorizontalAlignment`) next to
  `start`, for images.
- **Phone i18n is not message-id based today.** `page/i18n.js` uses its own
  keys (`tile_about`, …), not the launcher's msgids. § 4.1 says "the phone
  page's i18n uses the same message ids". It doesn't yet: the phone needs
  the launcher's `.po` strings for template texts, e.g. a msgid → text
  table generated at build time from `translations/*.po`. That is new work
  in step 1.
- **Token names differ on the phone:** `text-dim` is `--dim` there. The
  renderer maps the names, so `var(--<token>)` in § 4.1 needs that fix.
- **Elision rule, both surfaces:**
  - Text aligns to the UI's start side.
  - The text's own script decides where "…" goes. On the phone that means
    `dir="auto"` plus an explicit left or right alignment.
  - Without this, an RTL page cut a Ukrainian caption at its start.
- **Ukrainian "Доступне оновлення" doesn't fit at 23u** and gets "…". The
  § 3 rule ("tile text is checked elided in all languages") is needed:
  shorter wording or a smaller caption, for the design pass.
- **Build setup:** the generated file is written to `OUT_DIR` and found
  through `with_include_paths`. The launcher's `build.rs` needs one more
  include path and no new crate, because `serde_json` is already in the
  workspace.
- **Test setup:** a pixel test needs image decoding (`i-slint-core`
  `image-decoders`, a Slint internal crate) and system fonts. In the
  launcher those come from the desktop backend, so step 1's screenshot
  tests should run through the existing desktop build, not a bare
  `MinimalSoftwareWindow`.
