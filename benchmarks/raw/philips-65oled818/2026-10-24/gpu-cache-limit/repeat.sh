#!/bin/zsh
# About and the menu again, twice; native heap after each, settled.
D=${SERIAL:?adb serial}
OUT=${1:?out dir}
key() { adb -s $D shell input keyevent $1; sleep 0.5; }
nat() { sleep 20; m=$(adb -s $D shell dumpsys meminfo ai.rusttv.launcher); echo "$1 pss=$(echo "$m" | awk '/TOTAL PSS:/{print $3}') native=$(echo "$m" | awk '/Native Heap:/{print $3}')" | tee -a $OUT/repeat.txt; }
nat start
for round in 2 3; do
  key DPAD_RIGHT; key DPAD_CENTER; sleep 8; key BACK; sleep 1
  nat "about-$round"
  key DPAD_LEFT; adb -s $D shell input keycombination -t 1200 DPAD_CENTER F12; sleep 1; key DPAD_DOWN; sleep 4; key BACK; sleep 1
  nat "menu-$round"
done
