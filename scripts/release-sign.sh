#!/bin/sh
# The owner's local part of a release (RELEASE.md § Checklist): sign what the
# Release build of a tag built, and add it to that tag's draft release. Takes
# the CLI archives and SHA256SUMS from the draft and the unsigned APKs from
# the run, checks them, signs the APKs with the release key, checks their
# certificate and code, adds their sums, signs SHA256SUMS with the update
# signing key and uploads the APKs, SHA256SUMS and SHA256SUMS.minisig to the
# draft. Publishes nothing: press Publish on GitHub afterwards.
# Usage: scripts/release-sign.sh v<version>
# Asks for the keystore password once (minisign asks for its own).
set -eu
TAG=$1
cd "$(dirname "$0")/.."
KEYS="$HOME/.rusttv-release"
# The APK certificate's SHA-256 (RELEASE.md § Signing).
CERT=ec16f7040c826d218cfd147c1fa49c4b376253c7b89b3d5e97ac707c8fd54473
SDK="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
TOOLS=$(ls -d "$SDK"/build-tools/* | sort -V | tail -1)

fail() { echo "release-sign: $*" >&2; exit 1; }

version=${TAG#v}
[ "$TAG" = "v$version" ] || fail "give the tag, e.g. v0.1.0"
[ "$(gh release view "$TAG" --json isDraft --jq .isDraft)" = true ] ||
    fail "$TAG has no draft release (published already, or the Release build hasn't made it)"
# The Tag release workflow makes the tag on GitHub; fetch it.
git fetch -q origin "refs/tags/$TAG:refs/tags/$TAG"
commit=$(git rev-parse "$TAG^{commit}")
run=$(gh run list --workflow release-build.yml --limit 50 \
    --json databaseId,headBranch,headSha,conclusion \
    --jq "map(select(.headBranch == \"$TAG\" and .headSha == \"$commit\" and .conclusion == \"success\"))[0].databaseId")
[ -n "$run" ] && [ "$run" != null ] || fail "no successful Release build of $TAG at $commit"
pub=$(sed -n 2p "$KEYS/rusttv-release.pub")
git show "$commit:crates/rusttv-core/src/update.rs" |
    grep -A1 'pub const RELEASE_SIGNING_KEY' | grep -qF "Some(\"$pub\")" ||
    fail "RELEASE_SIGNING_KEY at $TAG is not $KEYS/rusttv-release.pub"

OUT="target/release-$version"
rm -rf "$OUT" && mkdir -p "$OUT/unsigned"
gh release download "$TAG" -D "$OUT" -p '*.tar.gz' -p SHA256SUMS
(cd "$OUT" && shasum -a 256 -c SHA256SUMS)
gh run download "$run" -n rusttv-launcher-apk -D "$OUT/unsigned"

printf 'Keystore password: '
stty -echo; read -r RUSTTV_KEYSTORE_PASSWORD; stty echo; echo
export RUSTTV_KEYSTORE_PASSWORD
for abi in arm64-v8a armeabi-v7a x86_64; do
    in="$OUT/unsigned/rusttv-launcher-$version-$abi-unsigned.apk"
    apk="$OUT/rusttv-launcher-$version-$abi.apk"
    "$TOOLS/zipalign" -c -p 4 "$in" || fail "$in is not aligned"
    "$TOOLS/apksigner" sign --ks "$KEYS/rusttv-release.jks" --ks-key-alias rusttv \
        --ks-pass env:RUSTTV_KEYSTORE_PASSWORD --key-pass env:RUSTTV_KEYSTORE_PASSWORD \
        --out "$apk" "$in"
    "$TOOLS/apksigner" verify --print-certs "$apk" | grep -q "SHA-256 digest: $CERT" ||
        fail "$apk is not signed with the release key"
    android/check-dex.sh "$apk"
done
unset RUSTTV_KEYSTORE_PASSWORD
rm -rf "$OUT/unsigned"

(cd "$OUT" && shasum -a 256 rusttv-launcher-*.apk >> SHA256SUMS &&
    minisign -Sm SHA256SUMS -s "$KEYS/rusttv-release.key" -t "RustTV $version" &&
    minisign -Vm SHA256SUMS -p "$KEYS/rusttv-release.pub")
gh release upload "$TAG" --clobber "$OUT"/rusttv-launcher-*.apk "$OUT/SHA256SUMS" "$OUT/SHA256SUMS.minisig"
echo "release-sign: $TAG is complete; publish it: $(gh release view "$TAG" --json url --jq .url)"
