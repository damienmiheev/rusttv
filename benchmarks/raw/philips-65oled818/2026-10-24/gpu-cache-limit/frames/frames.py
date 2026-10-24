import sys, statistics, json
# Merge SurfaceFlinger --latency reads; dropped frame = a present gap > 1.5 vsync
# inside a burst (gaps over 200 ms are the UI being still, not a drop).
for path in sys.argv[1:]:
    period, presents = None, set()
    for line in open(path):
        p = line.split()
        if len(p) == 1 and period is None: period = int(p[0])
        if len(p) == 3 and 0 < int(p[1]) < 2**62: presents.add(int(p[1]))
    t = sorted(presents)
    gaps = [(b - a) / 1e6 for a, b in zip(t, t[1:]) if b - a < 200e6]
    v = period / 1e6
    drops = sum(1 for g in gaps if g > 1.5 * v)
    missed = sum(round(g / v) - 1 for g in gaps if g > 1.5 * v)
    qs = statistics.quantiles(gaps, n=100)
    print(json.dumps({"file": path.split('/')[-1], "frames": len(t), "gaps": len(gaps), "drops": drops,
        "drop_pct": round(100 * drops / len(gaps), 2), "missed_vsyncs": missed,
        "p50_ms": round(qs[49], 1), "p90_ms": round(qs[89], 1), "p99_ms": round(qs[98], 1), "max_ms": round(max(gaps), 1)}))
