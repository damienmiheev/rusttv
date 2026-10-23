import sys, statistics, json
from perfetto.trace_processor import TraceProcessor
tp = TraceProcessor(trace=sys.argv[1])
q = lambda s: list(tp.query(s))
keys = [r.ts for r in q("select ts from slice where name='interceptKeyBeforeQueueing' order by ts")]
presses = []
for t in keys:
    if not presses or t - presses[-1] > 1_000_000_000: presses.append(t)
frames = q("""select ts, dur, layer_name from actual_frame_timeline_slice
  where layer_name like '%animation-leash%' or layer_name like '%ActivityRecord{%' or layer_name like 'TX - Task=%'
  order by ts""")
app_frames = q("select ts, layer_name from actual_frame_timeline_slice where layer_name like 'TX - %/%' and layer_name not like '%ActivityRecord%' and layer_name not like '%Task=%'")
res = {"open": [], "home": [], "missing": 0, "presses": len(presses)}
for i, p in enumerate(presses):
    nxt = presses[i+1] if i+1 < len(presses) else p + 10**10
    fs = [f for f in frames if p <= f.ts < min(nxt, p + 3_000_000_000)]
    kind = "open" if i % 2 == 0 else "home"
    if i < 4: continue  # 2 warm-up cycles
    if not fs: res["missing"] += 1; continue
    # A frame's dur can span until the next frame (seconds while the screen is still), so cap it.
    res[kind].append((max(f.ts + min(f.dur, 50_000_000) for f in fs) - p) / 1e6)
    apps = {f.layer_name.split('/')[0] for f in app_frames if p <= f.ts < min(nxt, p + 3_000_000_000)}
    if kind == 'open': res.setdefault('apps', set()).update(apps)
def stats(v):
    if len(v) < 4: return None
    qs = statistics.quantiles(v, n=4)
    return {"n": len(v), "median_ms": round(statistics.median(v)), "q1": round(qs[0]), "q3": round(qs[2])}
print(json.dumps({"apps": sorted(res.get("apps", [])), "presses": res["presses"], "missing": res["missing"], "open": stats(res["open"]), "home": stats(res["home"])}))
