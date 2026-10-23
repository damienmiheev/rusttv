#!/bin/sh
# Builds one launcher APK per ABI (arm64-v8a, armeabi-v7a, x86_64): the Rust
# library with cargo-ndk, then Gradle. Requires the Android SDK (platform 34),
# NDK 30, JDK 17, cargo-ndk, curl, shasum, and for the armeabi-v7a Skia source
# build also git, Python 3 and libclang.
set -eu
cd "$(dirname "$0")"
SDK="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
export ANDROID_HOME="$SDK"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$SDK/ndk/30.0.16248370}"
# cargo-ndk sets ANDROID_PLATFORM=26, which makes Slint's build script look
# for the API 26 SDK platform; point it at the installed android.jar instead.
export ANDROID_JAR="${ANDROID_JAR:-$SDK/platforms/android-34/android.jar}"

# Slint's Android renderer links Skia. skia-bindings would download a prebuilt
# unchecked at build time, so fetch it here instead, check it against a pinned
# hash, and hand it to the build as a local file. A mismatch fails the build.
# Update every pin below together when Slint (and so skia-bindings) is upgraded.
SKIA_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/rusttv/skia"
fetch() { # $1 = URL, $2 = file name in $SKIA_DIR, $3 = SHA-256
    if [ ! -f "$SKIA_DIR/$2" ]; then
        mkdir -p "$SKIA_DIR"
        curl --fail --location --silent --show-error -o "$SKIA_DIR/$2.part" "$1"
        mv "$SKIA_DIR/$2.part" "$SKIA_DIR/$2"
    fi
    echo "$3  $SKIA_DIR/$2" | shasum -a 256 -c -
}

# arm64-v8a and x86_64: one prebuilt archive per ABI.
SKIA_TAG="0.153.3"
fetch_prebuilt() { # $1 = key, $2 = SHA-256
    fetch "https://github.com/rust-skia/skia-binaries/releases/download/$SKIA_TAG/skia-binaries-$1.tar.gz" \
        "skia-binaries-$1.tar.gz" "$2"
}
fetch_prebuilt b7f043e0b1e2a850e702-aarch64-linux-android-ganesh-gl-jpegd-jpege-pdf-vulkan \
    0d917b608d03632fa8428e00ff715bbe5663f89618893a5e0265ffea3e6cf519
fetch_prebuilt b7f043e0b1e2a850e702-x86_64-linux-android-ganesh-gl-jpegd-jpege-pdf-vulkan \
    a424123841ff47e99270ac836e892686a95f27c814cd026a5bafc2e80df794b1

# armeabi-v7a: skia-binaries has no prebuilt, so Skia is built from source.
# The source is rust-skia/skia at the commit of tag m153-0.101.2 (skia-bindings'
# `package.metadata.skia`). Its tools/git-sync-deps then fetches Skia's own
# dependencies at the git commits in its DEPS file, plus gn, and bin/fetch-ninja
# fetches the ninja version DEPS names. This runs once; the synced tree is
# reused from the cache.
SKIA_SRC_COMMIT="61e7ca4e99062cdd0ab69445d5963fb3365778f6"
SKIA_SRC_SHA256="8401d0de878582a03abc1029bb62330aea58cda89904ae6ab187302e6d48ba5d"
SKIA_SRC="$SKIA_DIR/skia-$SKIA_SRC_COMMIT"
fetch "https://codeload.github.com/rust-skia/skia/tar.gz/$SKIA_SRC_COMMIT" \
    "skia-$SKIA_SRC_COMMIT.tar.gz" "$SKIA_SRC_SHA256"
if [ ! -f "$SKIA_SRC/.rusttv-synced" ]; then
    rm -rf "$SKIA_SRC"
    tar -xzf "$SKIA_DIR/skia-$SKIA_SRC_COMMIT.tar.gz" -C "$SKIA_DIR"
    (cd "$SKIA_SRC" && GIT_SYNC_DEPS_SKIP_EMSDK=1 python3 tools/git-sync-deps &&
        python3 bin/fetch-ninja)
    touch "$SKIA_SRC/.rusttv-synced"
fi

SKIA_BINARIES_URL="file://$SKIA_DIR/skia-binaries-{key}.tar.gz" \
    cargo ndk -t arm64-v8a -t x86_64 --platform 26 -o app/src/main/jniLibs \
    build -p rusttv-launcher --lib --profile android-release
# skia-bindings passes clang the unversioned `--target=armv7-linux-androideabi`,
# which NDK 30's headers reject; a later versioned --target wins. The dashed
# BINDGEN_EXTRA_CLANG_ARGS/CXXFLAGS names avoid the underscore ones cargo-ndk sets
# (they need `env`, as sh can't name them). bindgen uses the NDK's own libclang,
# since an older system one fails on NDK 30's libc++ headers.
V7_TARGET="--target=armv7a-linux-androideabi26"
env ANDROID_NDK="$ANDROID_NDK_HOME" SKIA_SOURCE_DIR="$SKIA_SRC" \
    LIBCLANG_PATH="$(echo "$ANDROID_NDK_HOME"/toolchains/llvm/prebuilt/*/lib)" \
    SKIA_NINJA_COMMAND="$SKIA_SRC/third_party/ninja/ninja" \
    SKIA_GN_ARGS="extra_cflags_c=[\"$V7_TARGET\"] extra_cflags_cc=[\"$V7_TARGET\"]" \
    "BINDGEN_EXTRA_CLANG_ARGS_armv7-linux-androideabi=$V7_TARGET" \
    "CXXFLAGS_armv7-linux-androideabi=$V7_TARGET" \
    cargo ndk -t armeabi-v7a --platform 26 -o app/src/main/jniLibs \
    build -p rusttv-launcher --lib --profile android-release
./gradlew -q assembleRelease
ls -la app/build/outputs/apk/release/app-arm64-v8a-release.apk \
    app/build/outputs/apk/release/app-armeabi-v7a-release.apk \
    app/build/outputs/apk/release/app-x86_64-release.apk
