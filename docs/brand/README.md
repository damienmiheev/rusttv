# RustTV brand assets

Direction "AB3": a warm amber tile with a dark TV screen holding a bold white
"R". Original artwork in the launcher's palette (amber `#F5A524`, ink
`#12141A`, text `#F2F3F5`); no Rust project or third-party logos.

| File | Use |
|---|---|
| `icon-1024.png`, `icon-512.png` | App icon, avatar |
| `logo-1280x640.png` | README, social preview |
| `android/app/src/main/res/drawable-xhdpi/banner.png` | Android TV banner, 320×180 |
| `android/app/src/main/res/mipmap-*/ic_launcher_{background,foreground}.png` + `mipmap-anydpi-v26/ic_launcher.xml` | Adaptive app icon |

All of them are rendered from the HTML/CSS templates in `scripts/brand/`
(one stylesheet, `brand.css`) by `scripts/brand/render.sh`, which uses
headless Chrome. To change the design, edit the templates and rerun it. The
type is Inter (SIL Open Font License 1.1), taken from the `i-slint-common`
crate at render time and not committed.
