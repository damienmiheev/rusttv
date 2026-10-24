#!/bin/sh
# The owner's local part of a release (RELEASE.md § Checklist, Build): take
# the CLI archives and SHA256SUMS from a finished "Release build" run, build
# the APKs with the release key, check their certificate, add their sums and
# sign SHA256SUMS with the update signing key. Publishes nothing.
# Usage: scripts/release-sign.sh <run-id>
# Asks for the keystore password once (minisign asks for its own).
set -eu
RUN=$1
cd "$(dirname "$0")/.."
KEYS="$HOME/.rusttv-release"
# The APK certificate's SHA-256 (RELEASE.md § Signing).
CERT=ec16f7040c826d218cfd147c1fa49c4b376253c7b89b3d5e97ac707c8fd54473
SDK="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
APKSIGNER=$(ls -d "$SDK"/build-tools/*/apksigner | sort -V | tail -1)

fail() { echo "release-sign: $*" >&2; exit 1; }

[ -z "$(git status --porcelain --untracked-files=no)" ] || fail "the tree has uncommitted changes"
head=$(gh run view "$RUN" --json headSha,conclusion,workflowName \
    --jq 'select(.workflowName == "Release build" and .conclusion == "success") | .headSha')
[ -n "$head" ] || fail "run $RUN is not a successful Release build"
[ "$head" = "$(git rev-parse HEAD)" ] || fail "run $RUN built $head, not HEAD"
pub=$(sed -n 2p "$KEYS/rusttv-release.pub")
grep -q "RELEASE_SIGNING_KEY: Option<&str> = Some(\"$pub\")" crates/rusttv-core/src/update.rs ||
    fail "RELEASE_SIGNING_KEY in rusttv-core is not $KEYS/rusttv-release.pub"
version=$(cargo metadata --no-deps --format-version 1 |
    jq -r '.packages[] | select(.name == "rusttv-core") | .version')

OUT="target/release-$version"
rm -rf "$OUT" && mkdir -p "$OUT"
gh run download "$RUN" -p 'cli-*' -p SHA256SUMS -D "$OUT/artifacts"
find "$OUT/artifacts" -name '*.tar.gz' -exec mv {} "$OUT/" \;
mv "$OUT/artifacts/SHA256SUMS/SHA256SUMS" "$OUT/"
rm -rf "$OUT/artifacts"
(cd "$OUT" && shasum -a 256 -c SHA256SUMS)

export RUSTTV_KEYSTORE_FILE="$KEYS/rusttv-release.jks" RUSTTV_KEY_ALIAS=rusttv
printf 'Keystore password: '
stty -echo; read -r RUSTTV_KEYSTORE_PASSWORD; stty echo; echo
export RUSTTV_KEYSTORE_PASSWORD RUSTTV_KEY_PASSWORD="$RUSTTV_KEYSTORE_PASSWORD"
android/build.sh
unset RUSTTV_KEYSTORE_PASSWORD RUSTTV_KEY_PASSWORD

for abi in arm64-v8a armeabi-v7a x86_64; do
    apk="android/app/build/outputs/apk/release/app-$abi-release.apk"
    "$APKSIGNER" verify --print-certs "$apk" | grep -q "SHA-256 digest: $CERT" ||
        fail "$apk is not signed with the release key"
    cp "$apk" "$OUT/rusttv-launcher-$version-$abi.apk"
done
(cd "$OUT" && shasum -a 256 rusttv-launcher-*.apk >> SHA256SUMS &&
    minisign -Sm SHA256SUMS -s "$KEYS/rusttv-release.key" -t "RustTV $version" &&
    minisign -Vm SHA256SUMS -p "$KEYS/rusttv-release.pub")
echo "release-sign: ready in $OUT"
ls -la "$OUT"
