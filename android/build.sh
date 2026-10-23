#!/bin/sh
# Builds the launcher APK (arm64): the Rust library with cargo-ndk, then Gradle.
# Requires the Android SDK (platform 34), NDK 30, JDK 17, cargo-ndk, curl and shasum.
set -eu
cd "$(dirname "$0")"
SDK="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
export ANDROID_HOME="$SDK"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$SDK/ndk/30.0.16248370}"
# cargo-ndk sets ANDROID_PLATFORM=26, which makes Slint's build script look
# for the API 26 SDK platform; point it at the installed android.jar instead.
export ANDROID_JAR="${ANDROID_JAR:-$SDK/platforms/android-34/android.jar}"

# Slint's Android renderer links a prebuilt Skia that skia-bindings would
# download unchecked at build time. Fetch it here instead, check it against a
# pinned hash, and hand it to the build as a local file. Update all three
# values together when Slint (and so skia-bindings) is upgraded; a mismatch
# fails the build.
SKIA_TAG="0.153.3"
SKIA_KEY="b7f043e0b1e2a850e702-aarch64-linux-android-ganesh-gl-jpegd-jpege-pdf-vulkan"
SKIA_SHA256="0d917b608d03632fa8428e00ff715bbe5663f89618893a5e0265ffea3e6cf519"
SKIA_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/rusttv/skia"
SKIA_FILE="$SKIA_DIR/skia-binaries-$SKIA_KEY.tar.gz"
if [ ! -f "$SKIA_FILE" ]; then
    mkdir -p "$SKIA_DIR"
    curl --fail --location --silent --show-error -o "$SKIA_FILE.part" \
        "https://github.com/rust-skia/skia-binaries/releases/download/$SKIA_TAG/skia-binaries-$SKIA_KEY.tar.gz"
    mv "$SKIA_FILE.part" "$SKIA_FILE"
fi
echo "$SKIA_SHA256  $SKIA_FILE" | shasum -a 256 -c -
export SKIA_BINARIES_URL="file://$SKIA_DIR/skia-binaries-{key}.tar.gz"

cargo ndk -t arm64-v8a --platform 26 -o app/src/main/jniLibs \
    build -p rusttv-launcher --lib --profile android-release
./gradlew -q assembleRelease
ls -la app/build/outputs/apk/release/app-release.apk
