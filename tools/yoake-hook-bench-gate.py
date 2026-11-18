"""Bench gate for empty call-site hook slots (MX3CS2 Phase 1d).

usage: yoake-hook-bench-gate.py <baseline exe> <instrumented exe> [--runs N] [--max-overhead PCT]

Both exes are the release `yoake_hook_bench` example of horizon-client-lib: the baseline is built with
`--features yoake-hook-bench,yoake-callsite-baseline` (fn hooks on, call-site hooks off), the instrumented one with
`--features yoake-hook-bench`. Both run `null` mode (no hook registered) alternately, N times each; the medians of
`tick_settle_ns` and `settle_interpolation_ns_per_entity` (the per-entity tick slice) are compared. Exit 1 when an
overhead is above the limit.
"""
import argparse
import json
import statistics
import subprocess
import sys

METRICS = {
    "tick_settle_ns": "ArmorStand::tick_settle per call",
    "settle_interpolation_ns_per_entity": "settle + interpolate + render_position per entity",
}


def run(exe):
    out = subprocess.run([exe, "null"], capture_output=True, text=True, check=True).stdout
    return json.loads(out)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("baseline")
    parser.add_argument("instrumented")
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--max-overhead", type=float, default=5.0, help="percent")
    args = parser.parse_args()

    samples = {"baseline": {k: [] for k in METRICS}, "instrumented": {k: [] for k in METRICS}}
    for _ in range(args.runs):
        for name, exe in (("baseline", args.baseline), ("instrumented", args.instrumented)):
            result = run(exe)
            for key in METRICS:
                samples[name][key].append(result[key])

    failed = False
    for key, label in METRICS.items():
        base = statistics.median(samples["baseline"][key])
        inst = statistics.median(samples["instrumented"][key])
        overhead = (inst - base) / base * 100.0
        verdict = "ok" if overhead <= args.max_overhead else "FAIL"
        failed |= verdict == "FAIL"
        print(f"{verdict:4} {label}: baseline {base:.3f} ns, instrumented {inst:.3f} ns, "
              f"overhead {overhead:+.1f} % (limit {args.max_overhead:.1f} %)")
        print(f"     baseline runs {['%.3f' % v for v in samples['baseline'][key]]}")
        print(f"     instrumented runs {['%.3f' % v for v in samples['instrumented'][key]]}")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
