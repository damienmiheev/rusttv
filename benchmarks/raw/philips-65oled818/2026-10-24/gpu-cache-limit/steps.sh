#!/bin/zsh
# Fresh start, then About, then the menu; PSS and native heap every 10 s.
D=${SERIAL:?adb serial}
OUT=${1:?out dir}
sample() { # label, count
  for i in $(seq 1 $2); do
    m=$(adb -s $D shell dumpsys meminfo ai.rusttv.launcher)
    pss=$(echo "$m" | awk '/TOTAL PSS:/{print $3}')
    nat=$(echo "$m" | awk '/Native Heap:/{print $3}')
    code=$(echo "$m" | awk '/ Code:/{print $2}')
    echo "$1 $(date +%H:%M:%S) pss=$pss native=$nat code=$code" | tee -a $OUT/samples.txt
    sleep 10
  done
}
key() { adb -s $D shell input keyevent $1; sleep 0.5; }
adb -s $D shell am force-stop ai.rusttv.launcher
sleep 1; key HOME; sleep 3
sample fresh 10
key DPAD_RIGHT; key DPAD_CENTER; sleep 2
sleep 8; key BACK; sleep 1
sample after-about 6
key DPAD_LEFT; adb -s $D shell input keycombination -t 1200 DPAD_CENTER F12; sleep 1
key DPAD_DOWN; sleep 4; key BACK; sleep 1
sample after-menu 6
