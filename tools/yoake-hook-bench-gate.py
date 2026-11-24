"""Bench gate for empty hook slots (callsite micro-bench and full-game stamped vs unstamped).

usage:
  yoake-hook-bench-gate.py <baseline exe> <instrumented exe> [--runs N] [--max-overhead PCT] [--metrics name1,name2]
  yoake-hook-bench-gate.py <unstamped client exe> <stamped client exe> --mode game [--runs N] [--max-overhead PCT]
      [--ctl PATH] [--resources PATH] [--work-dir PATH] [--render-distance N] [--flight-seconds S] [--mobs N]
      [--warmup N] [--settle S] [--no-pregen] [--priority LEVEL|none] [--idle-timeout S]

In default (`bench`) mode, both exes are the release `yoake_hook_bench` example of horizon-client-lib (or another
JSON micro-bench): both run `null` mode alternately, N times each, and the medians of `tick_settle_ns` and
`settle_interpolation_ns_per_entity` are compared (default max overhead 5.0%).

In `game` (full-game) mode, both exes are `horizon-client` builds (unstamped baseline vs stamped instrumented),
launched with no mods via `horizon-ctl launch --offscreen --no-pause --no-focus`, interleaved A B A B ..., N times
each. Before the measured runs the gate pre-generates world `Test` (seed 1, creative) once per invocation: it flies
the measured path with the baseline build, saves, and quits. Every measured run (and warm-up run) copies that saved
world into its own fresh work dir, so the measured flight loads terrain from disk instead of generating it. Each run
then waits for chunks around spawn, settles (--settle), summons a fixed mob set, flies the fixed path (--flight-seconds)
and records server tick time (`server_tick_ms`) and client frame time (`client_frame_p50_ms`, `client_cpu_p50_ms`).
`--warmup N` discarded A/B runs happen before the measured ones. Before each game run the gate waits until no
horizon-client.exe / rustc.exe is running (another job may build on the PC). Default max overhead in `game` mode is
1.0%, decided on the median of the per-pair ratios; a 95% percentile bootstrap interval of those paired ratios is
reported too. Exit 1 when any gated metric exceeds the limit.
"""
import argparse
import json
import math
import os
import random
import shutil
import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EXE = ".exe" if os.name == "nt" else ""
WORLD = "Test"
BUSY_IMAGES = ("horizon-client.exe", "rustc.exe")

BENCH_METRICS = {
    "tick_settle_ns": ("ArmorStand::tick_settle per call", "ns"),
    "settle_interpolation_ns_per_entity": ("settle + interpolate + render_position per entity", "ns"),
}

GAME_METRICS = {
    "server_tick_ms": ("Server tick time (fly smoothed tick_ms)", "ms"),
    "client_frame_p50_ms": ("Client frame time (p50)", "ms"),
    "client_cpu_p50_ms": ("Client CPU frame time (p50)", "ms"),
    "client_frame_avg_ms": ("Client frame time (avg)", "ms"),
    "client_cpu_avg_ms": ("Client CPU frame time (avg)", "ms"),
    "server_tick_max_ms": ("Server tick time (fly max)", "ms"),
}

DEFAULT_GAME_METRICS = ("server_tick_ms", "client_frame_p50_ms")


def run_bench(exe):
    out = subprocess.run([exe, "null"], capture_output=True, text=True, check=True).stdout
    return json.loads(out)


def _cleanup_dir(path: Path):
    for _ in range(20):
        try:
            shutil.rmtree(path)
            return
        except FileNotFoundError:
            return
        except OSError:
            time.sleep(0.5)
    shutil.rmtree(path, ignore_errors=True)


def _busy_images():
    """Names of BUSY_IMAGES that are currently running (tasklist output is OEM-coded; image names are ASCII)."""
    busy = []
    for image in BUSY_IMAGES:
        try:
            out = subprocess.run(
                ["tasklist", "/FI", f"IMAGENAME eq {image}", "/NH"], capture_output=True
            ).stdout.decode("ascii", "replace")
        except FileNotFoundError:
            return []
        if image.lower() in out.lower():
            busy.append(image)
    return busy


def wait_machine_idle(timeout):
    deadline = time.time() + timeout
    while True:
        busy = _busy_images()
        if not busy:
            return
        if time.time() > deadline:
            raise SystemExit(f"gave up waiting for {', '.join(busy)} to exit after {timeout:.0f} s")
        print(f"  waiting for {', '.join(busy)} to exit before measuring", flush=True)
        time.sleep(10.0)


def _set_priority(level):
    if level == "none" or os.name != "nt":
        return
    cmd = (
        "Get-Process -Name horizon-client -ErrorAction SilentlyContinue | "
        f"ForEach-Object {{ $_.PriorityClass = '{level}' }}"
    )
    subprocess.run(["powershell", "-NoProfile", "-Command", cmd], capture_output=True)


def run_game(exe, name, args, template=None, pregen_out=None):
    """One full-game run in a fresh work dir. Returns the metric dict, or None for a pregen run.

    template: a saved world folder copied into saves/WORLD before launch (terrain is loaded, not generated).
    pregen_out: when set, this run only flies the measured path and saves; the saved world is copied there.
    """
    work = Path(args.work_dir) / name
    _cleanup_dir(work)
    for sub in ("game", "home", "launcher"):
        (work / sub).mkdir(parents=True, exist_ok=True)
    (work / "game" / "options.txt").write_text(
        f"maxFps:260\nenableVsync:false\nrenderDistance:{args.render_distance}\n"
        f"simulationDistance:{args.render_distance}\ninactivityFpsLimit:\"minimized\"\npauseOnLostFocus:false\n",
        encoding="utf-8",
    )
    if template is not None:
        shutil.copytree(template, work / "game" / "saves" / WORLD)
    sock = work / "control.json"
    env = dict(
        os.environ,
        HORIZON_NO_AUDIO="1",
        HORIZON_TELEMETRY_URL="off",
        HORIZON_HOME=str(work / "home"),
        HORIZON_LAUNCHER_HOME=str(work / "launcher"),
    )

    def ctl(*cmd_args):
        res = subprocess.run(
            [str(args.ctl), "--socket", str(sock), *map(str, cmd_args)],
            env=env,
            capture_output=True,
            text=True,
            check=True,
        )
        lines = [line for line in res.stdout.strip().splitlines() if line.strip()]
        return json.loads(lines[-1]) if lines else {}

    def wait_for_chunks(min_chunks, timeout):
        deadline = time.time() + timeout
        last_counts, changed_at, quiet_at = None, time.time(), time.time()
        while time.time() < deadline:
            s = ctl("status")
            counts = (s.get("chunks"), s.get("generated_chunks"))
            if counts != last_counts:
                last_counts, changed_at = counts, time.time()
            streaming = s.get("streaming") or {}
            if any(v for k, v in streaming.items() if k != "protos") or s.get("pending_chunks", 1) != 0:
                quiet_at = time.time()
            if (
                s.get("chunks", 0) >= min_chunks
                and time.time() - changed_at >= 1.5
                and time.time() - quiet_at >= 1.0
            ):
                return
            time.sleep(0.1)

    min_chunks = max(40, int(0.8 * math.pi * (args.render_distance - 1) ** 2))
    path_out = ("169.5", "100.0", "162.5", "289.5", "100.0", "162.5")
    done = False
    try:
        ctl(
            "launch",
            "--game",
            str(exe),
            "--world",
            WORLD,
            "--seed",
            "1",
            "--creative",
            "--wait",
            "--offscreen",
            "--no-pause",
            "--no-focus",
            "--resources",
            str(args.resources),
            "--game-dir",
            str(work / "game"),
            "--home",
            str(work / "home"),
        )
        _set_priority(args.priority)
        wait_for_chunks(min_chunks, 180.0)

        ctl("fly", "169.5", "100.0", "162.5", "169.5", "100.0", "162.5", "0.1", "25")
        time.sleep(1.0)

        if pregen_out is not None:
            ctl("fly", *path_out, str(args.flight_seconds), "25")
            wait_for_chunks(min_chunks, 300.0)
            ctl("save")
            done = True
            return None

        for i in range(args.mobs):
            x = 180 + (i % 8) * 2
            z = 160 + (i // 8) * 2
            ctl("summon", "cow" if i % 2 == 0 else "pig", str(x), "85", str(z))
        time.sleep(args.settle)

        fly = ctl("fly", *path_out, str(args.flight_seconds), "25")
        avg_fps = fly.get("avg_fps") or 0.0
        cpu = fly.get("cpu") or {}
        return {
            "server_tick_ms": fly["tick_ms"],
            "server_tick_max_ms": fly.get("tick_ms_max", fly["tick_ms"]),
            "client_frame_p50_ms": fly["p50_ms"],
            "client_frame_avg_ms": 1000.0 / avg_fps if avg_fps > 0 else 0.0,
            "client_cpu_p50_ms": cpu["p50_ms"],
            "client_cpu_avg_ms": cpu["avg_ms"],
            "frames": fly.get("frames"),
            "phases_avg_ms": fly.get("phases_avg_ms"),
        }
    finally:
        try:
            ctl("quit")
        except Exception:
            pass
        time.sleep(2.0)
        if done and pregen_out is not None:
            _cleanup_dir(pregen_out)
            shutil.copytree(work / "game" / "saves" / WORLD, pregen_out)
        _cleanup_dir(work)


def summarize(values):
    ordered = sorted(values)
    n = len(ordered)
    med = statistics.median(ordered)
    lo, hi = ordered[0], ordered[-1]
    sd = statistics.stdev(ordered) if n > 1 else 0.0
    if n >= 4:
        q = statistics.quantiles(ordered, n=4)
        iqr = q[2] - q[0]
    else:
        iqr = hi - lo
    return {
        "n": n,
        "median": med,
        "min": lo,
        "max": hi,
        "spread": hi - lo,
        "iqr": iqr,
        "stdev": sd,
        "runs": values,
    }


def bootstrap_median_ci(values, iters=10000, seed=20260324):
    """95% percentile bootstrap interval of the median of `values` (fixed seed, so reruns agree)."""
    if len(values) < 2:
        return values[0], values[0]
    rng = random.Random(seed)
    n = len(values)
    meds = sorted(statistics.median([values[rng.randrange(n)] for _ in range(n)]) for _ in range(iters))
    return meds[int(0.025 * iters)], meds[int(0.975 * iters) - 1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("baseline")
    parser.add_argument("instrumented")
    parser.add_argument("--mode", choices=("bench", "game"), default="bench")
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument(
        "--max-overhead",
        type=float,
        default=None,
        help="percent (default 5.0 for bench mode, 1.0 for game mode)",
    )
    parser.add_argument("--metrics", default=None, help="comma-separated JSON metric names")
    parser.add_argument("--ctl", default=str(ROOT / "target/release" / f"horizon-ctl{EXE}"))
    parser.add_argument(
        "--resources",
        default=str(ROOT.parent / "horizon/.local/horizon-client/26.3"),
    )
    parser.add_argument("--work-dir", default=str(ROOT / "target/bench-gate"))
    parser.add_argument("--render-distance", type=int, default=8)
    parser.add_argument("--flight-seconds", type=float, default=20.0)
    parser.add_argument("--mobs", type=int, default=40)
    parser.add_argument("--warmup", type=int, default=1, help="discarded A/B runs before the measured ones (game mode)")
    parser.add_argument("--settle", type=float, default=5.0, help="seconds between summoning mobs and the flight")
    parser.add_argument("--no-pregen", action="store_true", help="generate terrain inside each run (old behaviour)")
    parser.add_argument(
        "--priority",
        default="AboveNormal",
        help="process priority class for the game process, same for both builds (Windows; 'none' disables)",
    )
    parser.add_argument("--idle-timeout", type=float, default=900.0, help="seconds to wait for busy processes to exit")
    parser.add_argument("--json-out", default=None, help="write full JSON summary to this path")
    args = parser.parse_args()

    if args.max_overhead is None:
        args.max_overhead = 1.0 if args.mode == "game" else 5.0

    if args.mode == "game":
        known = GAME_METRICS
        default_names = list(DEFAULT_GAME_METRICS)
    else:
        known = {k: v for k, v in BENCH_METRICS.items()}
        default_names = list(BENCH_METRICS.keys())

    if args.metrics:
        names = [name.strip() for name in args.metrics.split(",") if name.strip()]
    else:
        names = default_names
    if not names:
        parser.error("--metrics must name at least one metric")

    metrics = {name: known.get(name, (name, "ms" if args.mode == "game" else "ns")) for name in names}
    extra_keys = ["client_cpu_p50_ms"] if args.mode == "game" and "client_cpu_p50_ms" not in metrics else []
    all_keys = list(metrics.keys()) + extra_keys

    template = None
    warmup = args.warmup if args.mode == "game" else 0
    if args.mode == "game" and not args.no_pregen:
        template = Path(args.work_dir) / "pregen-world"
        wait_machine_idle(args.idle_timeout)
        print(f"pregenerating world {WORLD} with the baseline build into {template}", flush=True)
        run_game(args.baseline, "pregen-run", args, pregen_out=template)
        print("pregen done", flush=True)

    samples = {"baseline": {k: [] for k in all_keys}, "instrumented": {k: [] for k in all_keys}}
    for idx in range(warmup + args.runs):
        discard = idx < warmup
        tag = f"warmup{idx}" if discard else f"run{idx - warmup}"
        for name, exe in (("baseline", args.baseline), ("instrumented", args.instrumented)):
            if args.mode == "game":
                wait_machine_idle(args.idle_timeout)
                result = run_game(exe, f"{name}-{tag}", args, template=template)
            else:
                result = run_bench(exe)
            if discard:
                print(
                    f"[warm-up {idx + 1}/{warmup}] {name:12} "
                    + "  ".join(f"{k}={result[k]:.3f}" for k in all_keys),
                    flush=True,
                )
                continue
            for key in all_keys:
                samples[name][key].append(result[key])
            if args.mode == "game":
                print(
                    f"[run {idx - warmup + 1}/{args.runs}] {name:12} "
                    + "  ".join(f"{k}={result[k]:.3f}" for k in all_keys),
                    flush=True,
                )

    failed = False
    summary_out = {"mode": args.mode, "runs": args.runs, "warmup": warmup, "max_overhead_pct": args.max_overhead, "metrics": {}}
    for key in all_keys:
        label, unit = known.get(key, (key, "ms" if args.mode == "game" else "ns"))
        gated = key in metrics
        base_s = summarize(samples["baseline"][key])
        inst_s = summarize(samples["instrumented"][key])
        unpaired = (inst_s["median"] - base_s["median"]) / base_s["median"] * 100.0
        # Runs are interleaved A/B, so run i of both builds shares the machine's state at that moment (thermals,
        # background load, drift over a session). In game mode the gate uses the median of the per-pair ratios,
        # which cancels that drift; the unpaired median difference is reported next to it.
        ratios = [i / b - 1.0 for b, i in zip(samples["baseline"][key], samples["instrumented"][key]) if b > 0]
        paired = statistics.median(ratios) * 100.0 if ratios else unpaired
        if ratios:
            ci_lo, ci_hi = (x * 100.0 for x in bootstrap_median_ci(ratios))
        else:
            ci_lo, ci_hi = unpaired, unpaired
        overhead = paired if args.mode == "game" else unpaired
        if gated:
            verdict = "ok" if overhead <= args.max_overhead else "FAIL"
            failed |= verdict == "FAIL"
        else:
            verdict = "info"
        summary_out["metrics"][key] = {
            "label": label,
            "unit": unit,
            "gated": gated,
            "baseline": base_s,
            "instrumented": inst_s,
            "overhead_pct": overhead,
            "paired_median_pct": paired,
            "paired_ratios_pct": [r * 100.0 for r in ratios],
            "paired_median_ci95_pct": [ci_lo, ci_hi],
            "unpaired_median_pct": unpaired,
            "verdict": verdict,
        }
        print(
            f"{verdict:4} {label}: baseline {base_s['median']:.3f} {unit} "
            f"(min {base_s['min']:.3f}, max {base_s['max']:.3f}, spread {base_s['spread']:.3f}, stdev {base_s['stdev']:.3f}), "
            f"instrumented {inst_s['median']:.3f} {unit} "
            f"(min {inst_s['min']:.3f}, max {inst_s['max']:.3f}, spread {inst_s['spread']:.3f}, stdev {inst_s['stdev']:.3f}), "
            f"overhead {overhead:+.2f} % (limit {args.max_overhead:.1f} %)"
        )
        if args.mode == "game":
            print(
                f"     paired median {paired:+.2f} %, 95% bootstrap interval of paired median "
                f"[{ci_lo:+.2f}, {ci_hi:+.2f}] %, unpaired median {unpaired:+.2f} %"
            )
            print(f"     paired ratios %   {['%+.2f' % (r * 100.0) for r in ratios]}")
        print(f"     baseline runs     {['%.3f' % v for v in base_s['runs']]}")
        print(f"     instrumented runs {['%.3f' % v for v in inst_s['runs']]}")

    if args.json_out:
        out_path = Path(args.json_out)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(json.dumps(summary_out, indent=2) + "\n", encoding="utf-8")

    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
