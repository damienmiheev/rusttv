#!/bin/sh
# Containment check for the launcher library's undefined symbols: whatever the
# C++ runtime provides must be linked into librusttv_launcher.so, never left for
# the platform's own libc++ to supply. A shared library may keep an undefined
# symbol and still link, so the APK installs and the launcher then dies in
# NativeActivity.onCreate wherever the platform libc++ does not happen to export
# it (0.3.1 armeabi-v7a on Android 11: dlopen failed: cannot locate symbol
# "_ZNSt6__ndk113__hash_memoryEPKvj"). __cxa_atexit and __cxa_finalize come from
# libc and are expected. Fails (exit 1) and names each symbol otherwise.
# Usage: check-so.sh <apk>
set -eu
APK=$1
SDK="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
NDK="${ANDROID_NDK_HOME:-$SDK/ndk/30.0.16248370}"
READELF=$(ls -d "$NDK"/toolchains/llvm/prebuilt/*/bin/llvm-readelf 2>/dev/null | sort -V | tail -1)
[ -x "$READELF" ] || { echo "check-so: llvm-readelf not found under $NDK" >&2; exit 2; }
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
unzip -q -o "$APK" 'lib/*/librusttv_launcher.so' -d "$WORK"
for so in "$WORK"/lib/*/librusttv_launcher.so; do
    "$READELF" --dyn-symbols "$so" | awk -v so="$so" '
        # llvm-readelf marks an undefined symbol with UND in its Ndx column;
        # the name is the last field, and carries the version it is bound to
        # (__cxa_atexit@LIBC), which is dropped before the name is compared.
        $0 ~ /[[:space:]]UND[[:space:]]/ {
            name = $NF
            sub(/@.*$/, "", name)
            if (name == "__cxa_atexit" || name == "__cxa_finalize") next
            # Itanium C++ ABI mangling, plus the libc++abi and unwinder names.
            if (name ~ /^(_Z|__cxa_|_Unwind_|__gxx_|__cxxabi)/) {
                print "check-so: undefined C++ symbol in " so ": " name
                bad = 1
            }
        }
        END { exit bad }
    '
done
echo "check-so: ok ($APK)"
