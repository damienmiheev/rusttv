#!/bin/zsh
# Usage: block.sh <serial> <name> ; 2 warm-up + 30 measured open/Home cycles under one Perfetto trace.
S=$1; N=$2; D=${0:A:h}
cat > $D/cfg-$N.pbtx <<CFG
buffers { size_kb: 262144 fill_policy: RING_BUFFER }
data_sources { config { name: "android.surfaceflinger.frametimeline" } }
data_sources { config { name: "linux.ftrace" ftrace_config { atrace_categories: "input" atrace_categories: "wm" } } }
duration_ms: 260000
CFG
adb -s $S shell dumpsys power | grep -q "mWakefulness=Awake" || { adb -s $S shell input keyevent KEYCODE_WAKEUP; sleep 3; }
adb -s $S shell input keyevent KEYCODE_HOME; sleep 3
cat $D/cfg-$N.pbtx | adb -s $S shell perfetto --txt -c - -o /data/misc/perfetto-traces/$N.pftrace --background-wait
sleep 2
for i in $(seq 1 32); do
  adb -s $S shell input keyevent KEYCODE_DPAD_CENTER; sleep 4
  adb -s $S shell input keyevent KEYCODE_HOME; sleep 3.5
done
adb -s $S shell killall -INT perfetto 2>/dev/null; for w in $(seq 1 60); do adb -s $S shell pidof perfetto >/dev/null || break; sleep 2; done
adb -s $S pull /data/misc/perfetto-traces/$N.pftrace $D/ && adb -s $S shell rm /data/misc/perfetto-traces/$N.pftrace
echo done $N
