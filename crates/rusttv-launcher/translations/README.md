# Launcher translations

The launcher shows its texts in the TV's language, with English as the
fallback. Translations are gettext `.po` files, compiled into the launcher
(`build.rs`), so the APK ships no separate files.

- `rusttv-launcher.pot`: every text, from `@tr(...)` in `../ui/home.slint`.
- `<lang>/LC_MESSAGES/rusttv-launcher.po`: one language. The folder name is
  the language (`de`, `uk`), with the region only where it matters (`pt_BR`,
  `zh_CN` Simplified, `zh_TW` Traditional).

The device language picks the folder (`src/i18n.rs`): an exact language
match, Chinese by script or region, every Portuguese to `pt_BR`, else
English. Arabic and Hebrew also mirror the layout (`Dir` in `home.slint`).

The drafts were written by an AI and need review by native speakers;
Ukrainian is reviewed by the owner first.

## Translate

- Keep each `{}` (a version, an app name, a number or an English error
  message); you may move it within the sentence.
- Keep "OK" (printed on remotes), "RustTV", "Slint", "Skia", "Rust", the
  licence names and the URL as they are.
- Keep texts about as long as the English: TV space is limited.
- Never leave a `msgstr` empty and never mark an entry `fuzzy`: such
  entries are not used.

## Add a language

1. Copy `rusttv-launcher.pot` to `<lang>/LC_MESSAGES/rusttv-launcher.po`,
   add the `Language:` and `Plural-Forms:` header lines, and translate.
2. Add the folder name to `LANGUAGES` in `src/i18n.rs` (a test checks that
   every listed language has a file).
3. A right-to-left language also goes into `is_right_to_left` there.

## Change a text

Change it in `home.slint`, regenerate the template, and update every `.po`
file: a `msgid` that no longer matches shows English. Regenerate with
Slint's extractor (`cargo install slint-tr-extractor`), without the default
context (`build.rs` turns it off):

```bash
slint-tr-extractor --no-default-translation-context ui/home.slint -o translations/rusttv-launcher.pot
```
