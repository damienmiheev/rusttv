#!/bin/sh
# Read-only capture of a TV for fixtures, compat profile and debugging.
#
# Usage: scripts/tv-capture.sh <adb-serial>        (e.g. 192.168.1.20:5555)
#
# Runs only the fixed, read-only commands below (the same ones the emulator
# fixtures in crates/rusttv-core/src/fixtures/ came from) plus read-only
# `rusttv` commands. It installs, starts, stops and changes nothing.
# Output goes to .rusttv/captures/<serial>-<date>/ (git-ignored): review it,
# then copy what is needed into fixtures/ or compat/devices/ by hand.
#
# This is a developer tool, not part of `rusttv`: the CLI itself never runs
# free-form shell strings (CLAUDE.md).
set -eu

if [ $# -ne 1 ]; then
    echo "usage: $0 <adb-serial>   (see: adb devices -l)" >&2
    exit 2
fi
SERIAL="$1"
ADB="${RUSTTV_ADB:-adb}"
cd "$(dirname "$0")/.."

STATE=$("$ADB" -s "$SERIAL" get-state 2>/dev/null || true)
if [ "$STATE" != "device" ]; then
    echo "device $SERIAL is not connected and authorized (state: ${STATE:-none})" >&2
    exit 1
fi

SAFE_SERIAL=$(printf '%s' "$SERIAL" | tr -c 'A-Za-z0-9._-' '_')
OUT=".rusttv/captures/$SAFE_SERIAL-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"

# Identifiers of this particular unit, never kept (compat/README.md § Privacy).
redact() {
    sed -E \
        -e 's/^(\[[^]]*(serial|serialno|imei|meid|mac|wifimac|btmac|bt_addr|ssid|android_id|device_id|uuid)[^]]*\]: )\[.*\]$/\1[REDACTED]/I' \
        -e 's/([0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}/[REDACTED-MAC]/g'
}

capture() {
    file="$1"
    shift
    echo "  $file"
    "$ADB" -s "$SERIAL" "$@" 2>&1 | redact > "$OUT/$file" || true
}

echo "Capturing $SERIAL into $OUT"
capture devices-l.txt devices -l
capture getprop.txt shell getprop
capture meminfo.txt shell cat /proc/meminfo
capture df-data.txt shell df -k /data
capture packages-f.txt shell pm list packages -f
capture packages-3.txt shell pm list packages -3
capture packages-d.txt shell pm list packages -d
capture proc-stat-1.txt shell cat /proc/stat
sleep 1
capture proc-stat-2.txt shell cat /proc/stat
capture ps.txt shell ps -A -o PID,RSS,NAME
capture ps-time.txt shell ps -A -o PID,TIME+,NAME
capture home-activity.txt shell cmd package resolve-activity --brief \
    -a android.intent.action.MAIN -c android.intent.category.HOME
capture home-activities-all.txt shell cmd package query-activities --brief \
    -a android.intent.action.MAIN -c android.intent.category.HOME
capture leanback-activities.txt shell cmd package query-activities --brief \
    -a android.intent.action.MAIN -c android.intent.category.LEANBACK_LAUNCHER
capture display.txt shell wm size
capture density.txt shell wm density
capture abi.txt shell getprop ro.product.cpu.abilist

# The same reads through rusttv, as JSON (read-only commands only).
rusttv() {
    file="$1"
    shift
    echo "  $file"
    cargo run -q -p rusttv-cli -- "$@" --serial "$SERIAL" 2>"$OUT/$file.stderr" | redact > "$OUT/$file" || true
}
rusttv device-inspect.json device inspect --json
rusttv apps-list.json apps list --json
rusttv performance-snapshot.json performance snapshot --json
rusttv compat-profile.toml compat profile

echo "Done. Review $OUT before copying anything into the repo."
