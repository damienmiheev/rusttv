# Skia and the libraries compiled into it

The launcher APK links a Skia binary (Slint's Android renderer, via
`skia-bindings` 0.153.3). Skia bundles these libraries; their notices must ship
with the APK. Rust crates are covered separately by `cargo about`
(`scripts/licenses.sh`).

| File | Library | Licence |
|---|---|---|
| `skia-LICENSE.txt` | Skia | BSD-3-Clause |
| `expat-COPYING.txt` | Expat (XML parser) | MIT |
| `freetype-FTL.txt` | FreeType | FreeType License (FTL), chosen from its FTL-or-GPL-2.0 dual licence |
| `libjpeg-turbo-LICENSE.md`, `libjpeg-turbo-README.ijg` | libjpeg-turbo | IJG, BSD-3-Clause, zlib |
| `libpng-LICENSE.txt` | libpng | libpng License |
| `wuffs-LICENSE.txt` | Wuffs (GIF decoder) | Apache-2.0 |
| `zlib-LICENSE.txt` | zlib | Zlib |

Copied unmodified from the Skia source of skia-bindings 0.153.3 (Skia's
`LICENSE` and `third_party/externals/*`). Which libraries are linked was checked
from the exported symbols of `libskia.a` for arm64 (`XML_*`, `png_*`, `FT_*`,
`jpeg_*`, `wuffs_*`, `inflate`/`deflate`); HarfBuzz and ICU are not linked.
Recheck this list when Slint, and so skia-bindings, is upgraded.
