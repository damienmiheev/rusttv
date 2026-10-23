#!/bin/sh
# record.sh <name> <component> <run>: cold-starts <component> from Settings while recording.
export PATH=~/Library/Android/sdk/platform-tools:$PATH
S="${SERIAL:?set SERIAL to the adb serial}"
PKG=${2%%/*}
adb -s $S shell am start -W -n com.android.tv.settings/.MainSettings >/dev/null 2>&1
sleep 2
adb -s $S shell am force-stop $PKG
sleep 2
adb -s $S shell screenrecord --size 960x540 --bit-rate 8000000 --time-limit 5 /data/local/tmp/vc.mp4 &
REC=$!
sleep 1
adb -s $S shell am start -n $2 >/dev/null
wait $REC
adb -s $S pull /data/local/tmp/vc.mp4 "$1-$3.mp4" >/dev/null
adb -s $S shell rm -f /data/local/tmp/vc.mp4
