#!/bin/sh
# Writes the third-party licence notices for everything RustTV ships:
# Rust crates (cargo-about, about.toml), the Slint licence text, then Skia and the libraries
# compiled into it (licenses/skia/). Fails if a crate has a licence not
# accepted in about.toml.
#
# Usage: scripts/licenses.sh [output]   (default: THIRD_PARTY_NOTICES.txt)
# Needs: cargo install cargo-about --version 0.9.2 --locked --features cli
set -eu
cd "$(dirname "$0")/.."
OUT="${1:-THIRD_PARTY_NOTICES.txt}"
TMP="$OUT.tmp"

cargo about generate --fail --locked licenses/about.hbs > "$TMP"
{
    # The Slint crates (slint, i-slint-*) are used under this licence; cargo-about
    # finds no file for it, so it is copied from the slint crate's LICENSES/.
    printf '\n------------------------------------------------------------------------------\n'
    printf 'Slint (slint and i-slint-* crates): Slint Royalty-free License 2.0\n\n'
    cat licenses/slint/LicenseRef-Slint-Royalty-free-2.0.md
    printf '\n------------------------------------------------------------------------------\n'
    printf 'Skia and the libraries compiled into it (see licenses/skia/README.md)\n'
    for file in licenses/skia/skia-LICENSE.txt licenses/skia/expat-COPYING.txt \
        licenses/skia/freetype-FTL.txt licenses/skia/libjpeg-turbo-LICENSE.md \
        licenses/skia/libjpeg-turbo-README.ijg licenses/skia/libpng-LICENSE.txt \
        licenses/skia/wuffs-LICENSE.txt licenses/skia/zlib-LICENSE.txt; do
        printf '\n=== %s ===\n\n' "$(basename "$file")"
        cat "$file"
    done
    printf '\nPortions of this software are copyright (c) The FreeType Project (www.freetype.org). All rights reserved.\n'
} >> "$TMP"
mv "$TMP" "$OUT"
echo "Wrote $OUT ($(wc -l < "$OUT" | tr -d ' ') lines)"
