#!/bin/sh
# Builds one launcher APK per ABI (arm64-v8a, armeabi-v7a, x86_64): the Rust
# library with cargo-ndk, then Gradle. Requires the Android SDK (platform 34),
# NDK 30, JDK 17, cargo-ndk, cargo-about, curl and shasum.
set -eu
cd "$(dirname "$0")"
SDK="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
export ANDROID_HOME="$SDK"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$SDK/ndk/30.0.16248370}"
# cargo-ndk sets ANDROID_PLATFORM=26, which makes Slint's build script look
# for the API 26 SDK platform; point it at the installed android.jar instead.
export ANDROID_JAR="${ANDROID_JAR:-$SDK/platforms/android-34/android.jar}"

# Slint's Android renderer links a prebuilt Skia that skia-bindings would
# download unchecked at build time. Fetch one archive per ABI here instead,
# check it against a pinned hash, and hand it to the build as a local file. A
# mismatch fails the build. Update every key/hash pair together when Slint (and
# so skia-bindings) is upgraded.
SKIA_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/rusttv/skia"
fetch_skia() { # $1 = release URL, $2 = key, $3 = SHA-256
    file="$SKIA_DIR/skia-binaries-$2.tar.gz"
    if [ ! -f "$file" ]; then
        mkdir -p "$SKIA_DIR"
        curl --fail --location --silent --show-error -o "$file.part" \
            "$1/skia-binaries-$2.tar.gz"
        mv "$file.part" "$file"
    fi
    echo "$3  $file" | shasum -a 256 -c -
}
# arm64-v8a and x86_64: rust-skia's official prebuilts.
RUST_SKIA="https://github.com/rust-skia/skia-binaries/releases/download/0.153.3"
fetch_skia "$RUST_SKIA" b7f043e0b1e2a850e702-aarch64-linux-android-ganesh-gl-jpegd-jpege-pdf-vulkan \
    0d917b608d03632fa8428e00ff715bbe5663f89618893a5e0265ffea3e6cf519
fetch_skia "$RUST_SKIA" b7f043e0b1e2a850e702-x86_64-linux-android-ganesh-gl-jpegd-jpege-pdf-vulkan \
    a424123841ff47e99270ac836e892686a95f27c814cd026a5bafc2e80df794b1
# armeabi-v7a: rust-skia publishes none, so ours is built from source by the
# manual workflow .github/workflows/skia-armv7.yml (run 39956273511) and hosted
# as a release of this repository. On a skia-bindings upgrade, update the key
# in the workflow, run it, publish a new release and pin it here.
fetch_skia "https://github.com/damienmiheev/rusttv/releases/download/skia-0.153.3-armv7" \
    b7f043e0b1e2a850e702-armv7-linux-androideabi-ganesh-gl-jpegd-jpege-pdf-vulkan \
    4a83dbd6b479c561fd358b59810b33598255e70010f731aa284306f33de5d136
export SKIA_BINARIES_URL="file://$SKIA_DIR/skia-binaries-{key}.tar.gz"

cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 --platform 26 -o app/src/main/jniLibs \
    build -p rusttv-launcher --lib --profile android-release
# Third-party notices; Gradle copies them into every APK as
# assets/licenses/THIRD_PARTY_NOTICES.txt and fails if they are missing.
../scripts/licenses.sh
./gradlew -q assembleRelease
ls -la app/build/outputs/apk/release/app-arm64-v8a-release.apk \
    app/build/outputs/apk/release/app-armeabi-v7a-release.apk \
    app/build/outputs/apk/release/app-x86_64-release.apk
