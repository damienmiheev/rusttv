#!/bin/sh
# Throwaway spike runner (PANEL_STUDIO.md § 15 step 1). Fixed subcommands only;
# nothing free-form reaches adb. Only touches the package ai.rusttv.spike.
#   spike.sh build [--no-png]   build the armv7 .so and the debug APK
#   spike.sh install            install the spike APK (debug key, own id)
#   spike.sh run <mode> <secs>  start a mode, wait, print its log and PSS
#                               modes: compiled-home generic-home generic-<N> compiled-<N>
#   spike.sh startup <mode>     cold start: am start -W (3 times)
#   spike.sh stage <bytes>      stream <bytes> random bytes via content write
#   spike.sh a11y               dump the accessibility tree of the running spike
#   spike.sh uninstall          remove the spike APK
set -eu
cd "$(dirname "$0")"
PKG=ai.rusttv.spike
ACT=$PKG/android.app.NativeActivity
: "${SERIAL:?set SERIAL to the adb serial}"
adb_() { adb -s "$SERIAL" "$@"; }
case "${1:-}" in
build)
    SDK="${ANDROID_HOME:-$HOME/Library/Android/sdk}"; export ANDROID_HOME="$SDK"
    export ANDROID_NDK_HOME="$SDK/ndk/$(cat ../../android/ndk-version)"
    export ANDROID_JAR="$SDK/platforms/android-34/android.jar"
    export SKIA_BINARIES_URL="file://${XDG_CACHE_HOME:-$HOME/.cache}/rusttv/skia/skia-binaries-{key}.tar.gz"
    [ -z "${JAVA_HOME:-}" ] && JAVA_HOME="$(/usr/libexec/java_home -v 17)" && export JAVA_HOME
    feats=""; [ "${2:-}" = "--no-png" ] && feats="--no-default-features"
    cargo ndk -t armeabi-v7a --platform 26 -o android/app/src/main/jniLibs build --profile android-release $feats
    ls -l android/app/src/main/jniLibs/armeabi-v7a/libtv_renderer_spike.so
    ../../android/gradlew -q -p android assembleDebug
    ls -l android/app/build/outputs/apk/debug/app-debug.apk ;;
install) adb_ install -r android/app/build/outputs/apk/debug/app-debug.apk ;;
run)
    mode="$2"; secs="$3"
    case "$mode" in compiled-home|generic-home|generic-[0-9]*|compiled-[0-9]*) ;; *) echo "bad mode" >&2; exit 2 ;; esac
    case "$secs" in *[!0-9]*) echo "bad secs" >&2; exit 2 ;; esac
    adb_ shell am force-stop $PKG; adb_ logcat -c
    adb_ shell am start -n $ACT --es mode "$mode" >/dev/null
    sleep "$secs"
    adb_ shell dumpsys meminfo $PKG | grep -E "TOTAL PSS|TOTAL:" | head -1
    adb_ logcat -d -s RustTVSpike:I | grep -v "^-"
    adb_ shell am force-stop $PKG ;;
startup)
    mode="$2"
    case "$mode" in compiled-home|generic-home|generic-[0-9]*|compiled-[0-9]*) ;; *) echo "bad mode" >&2; exit 2 ;; esac
    for i in 1 2 3; do
        adb_ shell am force-stop $PKG; sleep 2
        adb_ shell am start -W -n $ACT --es mode "$mode" | grep -E "TotalTime|WaitTime"
    done
    adb_ shell am force-stop $PKG ;;
stage)
    n="$2"; case "$n" in *[!0-9]*) echo "bad size" >&2; exit 2 ;; esac
    adb_ logcat -c
    head -c "$n" /dev/urandom > /tmp/spike-stage.bin
    shasum -a 256 /tmp/spike-stage.bin | cut -c1-64
    start=$(date +%s)
    adb_ exec-in content write --uri content://$PKG.stage/package < /tmp/spike-stage.bin || echo "content write exit $?"
    sleep 2; adb_ logcat -d -s RustTVSpike:I | grep STAGE; rm -f /tmp/spike-stage.bin
    echo "wall $(( $(date +%s) - start )) s" ;;
a11y)
    adb_ shell uiautomator dump /sdcard/spike-ui.xml >/dev/null
    adb_ exec-out cat /sdcard/spike-ui.xml | tr '>' '\n' | grep -o 'text="[^"]*"\|content-desc="[^"]*"' | sort | uniq -c | head -20
    adb_ shell rm /sdcard/spike-ui.xml ;;
shot)
    mode="$2"
    case "$mode" in compiled-home|generic-home|generic-[0-9]*|compiled-[0-9]*) ;; *) echo "bad mode" >&2; exit 2 ;; esac
    adb_ shell am force-stop $PKG
    adb_ shell am start -n $ACT --es mode "$mode" >/dev/null
    sleep 4
    adb_ exec-out screencap -p > "shot-$mode.png"
    adb_ shell am force-stop $PKG; ls -l "shot-$mode.png" ;;
uninstall) adb_ uninstall $PKG ;;
*) sed -n '2,13p' "$0"; exit 2 ;;
esac
