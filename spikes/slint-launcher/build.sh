#!/bin/sh
# Builds the spike APK (arm64). Requires the Android SDK, NDK 30, JDK 17 and cargo-ndk.
set -eu
cd "$(dirname "$0")"
SDK="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
export ANDROID_HOME="$SDK"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$SDK/ndk/30.0.16248370}"
# cargo-ndk sets ANDROID_PLATFORM=26, which makes Slint's build script look
# for the API 26 SDK platform; point it at the installed android.jar instead.
export ANDROID_JAR="${ANDROID_JAR:-$SDK/platforms/android-34/android.jar}"
cargo ndk -t arm64-v8a --platform 26 -o android/app/src/main/jniLibs build --release
cd android
./gradlew -q assembleRelease
ls -la app/build/outputs/apk/release/app-release.apk
