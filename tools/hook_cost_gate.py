#!/usr/bin/env python3
"""W6 cost gate, step 2: the empty-slot overhead, stamped against unstamped.

Compares the `--benchmark <scene>` reports of two sets of runs (the same scene, a fixed world and flight, runs
interleaved A B B A ...; first run of each discarded as warm-up by the runner) and fails when the frame-time
overhead of the stamped build is above the limit.

  python tools/hook_cost_gate.py RUNS_DIR SCENE [--limit 1.0] [--metric p50]

RUNS_DIR holds `unstamped-N/benchmarks/*.json` and `stamped-N/benchmarks/*.json` (N >= 1). Prints the difference of
means with a 95% confidence interval (delta method, Student t ~ 2.1 for 10 runs); the exit code is 1 when the
point estimate is above --limit percent, 2 when it is below but the interval's upper end is (inconclusive, printed).
"""
import argparse, glob, json, math, statistics as st, sys


def load(root, variant, scene, metric):
    out = []
    for d in sorted(glob.glob(f"{root}/{variant}-*")):
        if d.endswith("-0"):
            continue
        files = glob.glob(d + "/benchmarks/*.json")
        if not files:
            continue
        j = json.load(open(files[0], encoding="utf-8"))
        fr = [x for x in j["frames"] if x["phase"] == scene][0]
        out.append({"p50": fr["p50_ms"], "cpu": fr["cpu_p50_ms"], "avg": 1000 / fr["avg_fps"], "p99": fr["p99_ms"]}[metric])
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("root")
    ap.add_argument("scene")
    ap.add_argument("--limit", type=float, default=1.0)
    ap.add_argument("--metric", default="p50", choices=["p50", "cpu", "avg", "p99"])
    a = ap.parse_args()
    u = load(a.root, "unstamped", a.scene, a.metric)
    s = load(a.root, "stamped", a.scene, a.metric)
    if len(u) < 3 or len(s) < 3:
        sys.exit("need at least 3 runs of each")
    mu, ms = st.mean(u), st.mean(s)
    d = ms / mu - 1
    se = math.sqrt((st.stdev(s) ** 2 / len(s)) / mu**2 + (ms**2 * st.stdev(u) ** 2 / len(u)) / mu**4)
    lo, hi = (d - 2.1 * se) * 100, (d + 2.1 * se) * 100
    print(f"{a.scene} {a.metric}: unstamped {mu:.3f} ms, stamped {ms:.3f} ms ({len(u)} and {len(s)} runs): overhead {d * 100:+.2f}%, 95% CI [{lo:+.2f}, {hi:+.2f}], limit {a.limit}%")
    if d * 100 > a.limit:
        print("FAIL: overhead above the limit")
        sys.exit(1)
    if hi > a.limit:
        print("PASS (point estimate), but the interval reaches above the limit: run more pairs")
        sys.exit(0)
    print("PASS")


if __name__ == "__main__":
    main()
