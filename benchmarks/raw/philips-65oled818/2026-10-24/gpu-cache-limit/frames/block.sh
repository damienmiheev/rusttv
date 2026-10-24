#!/bin/zsh
# Usage: block.sh <out file> ; 1 warm-up + 6 measured navigation rounds, RustTV's
# frame present times (SurfaceFlinger --latency) read every 1.5 s, raw lines to <out>.
D=${SERIAL:?adb serial}; OUT=$1
key() { adb -s $D shell input keyevent $1; sleep 1; }
round() {
  for k in DPAD_DOWN DPAD_DOWN DPAD_DOWN DPAD_DOWN DPAD_RIGHT DPAD_RIGHT DPAD_RIGHT DPAD_RIGHT DPAD_RIGHT \
           DPAD_UP DPAD_UP DPAD_UP DPAD_UP DPAD_LEFT DPAD_LEFT DPAD_LEFT DPAD_LEFT DPAD_LEFT; do key $k; done
  adb -s $D shell input keycombination -t 1500 DPAD_CENTER F12; sleep 1.5; key DPAD_DOWN; sleep 2; key BACK
}
adb -s $D shell dumpsys power | grep -q "mWakefulness=Awake" || { adb -s $D shell input keyevent KEYCODE_WAKEUP; sleep 3; }
adb -s $D shell input keyevent HOME; sleep 3
round  # warm-up
L=$(adb -s $D shell dumpsys SurfaceFlinger --list | grep "^ai.rusttv.launcher/ai.rusttv.launcher.LauncherActivity#" | tail -1)
: > $OUT
( while [ ! -f $OUT.stop ]; do adb -s $D shell dumpsys SurfaceFlinger --latency "'$L'" >> $OUT; sleep 1.5; done ) &
for r in 1 2 3 4 5 6; do echo "round $r" >> $OUT.rounds; date +%s >> $OUT.rounds; round; done
touch $OUT.stop; wait; rm $OUT.stop
