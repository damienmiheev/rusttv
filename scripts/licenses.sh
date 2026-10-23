#!/bin/sh
# Writes the third-party licence notices for everything RustTV ships:
# Rust crates (cargo-about, about.toml), the Slint licence text, Skia and the libraries
# compiled into it (licenses/skia/), then the NDK's LLVM runtimes
# (licenses/llvm/) and the Rust standard library (licenses/rust-std/).
# Fails if a crate has a licence not accepted in about.toml.
# android/build.sh runs it and Gradle packs the result into every APK.
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
    printf '\n------------------------------------------------------------------------------\n'
    printf 'libc++, libc++abi and libunwind from the Android NDK (see licenses/llvm/README.md)\n\n'
    cat licenses/llvm/LICENSE.TXT
    printf '\n------------------------------------------------------------------------------\n'
    printf 'The Rust standard library and the crates it is built from (see licenses/rust-std/README.md)\n'
    for file in licenses/rust-std/COPYRIGHT licenses/rust-std/LICENSE-MIT \
        licenses/rust-std/LICENSE-APACHE licenses/rust-std/Unicode-3.0.txt \
        licenses/rust-std/compiler-builtins-LICENSE.txt licenses/rust-std/libm-LICENSE.txt \
        licenses/rust-std/stdarch-LICENSE-MIT.txt licenses/rust-std/addr2line-LICENSE-MIT.txt \
        licenses/rust-std/adler2-LICENSE-0BSD.txt licenses/rust-std/cfg-if-LICENSE-MIT.txt \
        licenses/rust-std/gimli-LICENSE-MIT.txt licenses/rust-std/hashbrown-LICENSE-MIT.txt \
        licenses/rust-std/libc-LICENSE-MIT.txt licenses/rust-std/memchr-LICENSE-MIT.txt \
        licenses/rust-std/miniz_oxide-LICENSE-MIT.md licenses/rust-std/object-LICENSE-MIT.txt \
        licenses/rust-std/rustc-demangle-LICENSE-MIT.txt; do
        printf '\n=== %s ===\n\n' "$(basename "$file")"
        cat "$file"
    done
} >> "$TMP"
mv "$TMP" "$OUT"
echo "Wrote $OUT ($(wc -l < "$OUT" | tr -d ' ') lines)"
