import cv2, glob, statistics, sys
import numpy as np

def frames(path):
    cap = cv2.VideoCapture(path)
    out = []
    while True:
        ok, f = cap.read()
        if not ok: break
        out.append((cap.get(cv2.CAP_PROP_POS_MSEC), cv2.cvtColor(f, cv2.COLOR_BGR2GRAY).astype(np.int16)))
    return out

def changed(a, b):
    return float((np.abs(a - b) > 24).mean())

res = {}
for path in sorted(glob.glob("*.mp4")):
    fr = frames(path)
    base, final = fr[0][1], fr[-1][1]
    t0 = next(t for t, f in fr if changed(f, base) > 0.02)
    last_far = max((t for t, f in fr if changed(f, final) > 0.03), default=t0)
    idx = [i for i, (t, f) in enumerate(fr) if t > last_far]
    done = fr[idx[0]][0] if idx else last_far
    name = path.split("-")[0]
    res.setdefault(name, []).append(done - t0)
    print(f"{path}: start {t0:.0f} ms, visually complete {done:.0f} ms -> {done - t0:.0f} ms ({len(fr)} frames, {fr[-1][0]:.0f} ms)")
for name, v in res.items():
    print(f"{name}: median {statistics.median(v):.0f} ms  runs {[round(x) for x in v]}")
