# Visually complete start: Philips 65OLED818/12, 2026-10-23

Time until the home screen shows its real content, measured from screen
recordings rather than Android's first-frame time (`am start -W`), which counts
the stock launcher's skeleton placeholders as "started".

Method (`record.sh`, `analyse.py`): show Settings, force-stop the launcher,
start `adb shell screenrecord` (960×540), cold-start the launcher 1 s later,
record 5 s. Per recording: start = first frame differing from the Settings
frame in more than 2% of pixels; visually complete = first frame after the
last one that differs from the final frame in more than 3% of pixels (small
motion like RustTV's tile sway stays under that). 5 runs per launcher,
interleaved; RustTV at commit 80c4aca (banners load after the first frame).

| | Real content on screen (recording time) | First change → complete |
|---|---|---|
| Stock Google TV launcher | 4.1–4.8 s | median 3.0 s (skeleton placeholders until then) |
| RustTV | 1.9–2.1 s | median 0.14 s |

The stock launcher's first pixels change ~0.4 s earlier, but RustTV shows a
usable home screen ~2.4 s sooner. Checked by eye on run 3's frames: the stock
launcher shows grey skeletons from ~1.7 s to ~3.8 s and real rows at ~4.6 s;
RustTV is complete (banners, hero, glow) at ~1.95 s. Raw output: `results.txt`.
Videos and frames were not committed: they show the owner's apps.
