#!/bin/sh
# Release facts the release workflows read, in one place (RELEASE.md).
# Usage: scripts/release-info.sh version [<rev>]      workspace version in Cargo.toml
#        scripts/release-info.sh signing-key [<rev>]  RELEASE_SIGNING_KEY's value; fails if unset
#        scripts/release-info.sh apk-cert             the APK release certificate's SHA-256
# <rev> reads the file at that git revision instead of the work tree. Needs no
# Rust toolchain, so the workflows can call it before installing one.
set -eu
cd "$(dirname "$0")/.."

# The APK certificate's SHA-256 (RELEASE.md § Signing).
APK_CERT=ec16f7040c826d218cfd147c1fa49c4b376253c7b89b3d5e97ac707c8fd54473

fail() { echo "release-info: $*" >&2; exit 1; }
read_file() {
    if [ -n "${2:-}" ]; then git show "$2:$1"; else cat "$1"; fi
}

case ${1:-} in
version)
    version=$(read_file Cargo.toml "${2:-}" | awk '
        /^\[workspace\.package\]/ { on = 1; next }
        /^\[/ { on = 0 }
        on && /^version[ \t]*=/ { sub(/^[^"]*"/, ""); sub(/".*/, ""); print; exit }')
    echo "$version" | grep -Eqx '[0-9]+\.[0-9]+\.[0-9]+' || fail "no plain MAJOR.MINOR.PATCH version in [workspace.package]"
    echo "$version"
    ;;
signing-key)
    # rustfmt may put `Some(...)` on the line after the name.
    key=$(read_file crates/rusttv-core/src/update.rs "${2:-}" |
        grep -A1 'pub const RELEASE_SIGNING_KEY' | grep -o 'Some("RW[A-Za-z0-9+/=]*")' |
        sed 's/^Some("\(.*\)")$/\1/') || true
    [ -n "$key" ] || fail "RELEASE_SIGNING_KEY is not set (RELEASE.md § Update signing key)"
    echo "$key"
    ;;
apk-cert)
    echo "$APK_CERT"
    ;;
*)
    fail "usage: $0 version|signing-key [<rev>] | apk-cert"
    ;;
esac
