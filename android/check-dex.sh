#!/bin/sh
# Containment check for WRITE_SECURE_SETTINGS (HISTORY.md § 7): only
# ai.rusttv.launcher.SecureSettings may call a settings writer, nothing in
# the APK may write through a ContentResolver, and the launcher's own
# classes use no reflection. Fails (exit 1) and names each call otherwise.
# Usage: check-dex.sh <apk>
set -eu
APK=$1
SDK="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
DEXDUMP=$(ls -d "$SDK"/build-tools/*/dexdump 2>/dev/null | sort -V | tail -1)
[ -x "$DEXDUMP" ] || { echo "check-dex: dexdump not found under $SDK/build-tools" >&2; exit 2; }
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
unzip -q -o "$APK" 'classes*.dex' -d "$WORK"
for dex in "$WORK"/classes*.dex; do
    "$DEXDUMP" -d "$dex"
done | awk '
    # Each method body starts with "|[offset] package.Class.method:(...)".
    index($0, "|[") && match($0, /\|\[[0-9a-f]+\] [^ :]+:/) {
        name = substr($0, RSTART, RLENGTH - 1)
        sub(/^\|\[[0-9a-f]+\] /, "", name)
        sub(/\.[^.]+$/, "", name)
        gsub(/\./, "/", name)
        class = name
    }
    /invoke-/ {
        writer = $0 ~ /Landroid\/provider\/Settings\$(Global|Secure|System);\.put/
        resolver = $0 ~ /Landroid\/content\/ContentResolver;\.(insert|update|delete|call|bulkInsert|applyBatch):/
        reflect = $0 ~ /Ljava\/lang\/Class;\.forName:|Ljava\/lang\/reflect\/Method;\.invoke:/
        if ((writer && class != "ai/rusttv/launcher/SecureSettings") || resolver ||
            (reflect && class ~ /^ai\/rusttv\/launcher\//)) {
            print "check-dex: forbidden call in " class ": " $0
            bad = 1
        }
    }
    END { exit bad }
'
echo "check-dex: ok ($APK)"
