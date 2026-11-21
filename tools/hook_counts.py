#!/usr/bin/env python3
"""W6 cost gate, step 1: which hooked functions are hot?

Reads the `benchmarks/hook-counts.<scene>.txt` files a `--features hook-count` build writes during
`horizon-client --benchmark <scene>` (one fixed flight over a fixed world; `# frames N` first, then `<slot id> <entries>`),
joins the slot ids with hook-ledger/*.json and prints the functions entered more than --per-frame times per frame.
With --write the keys go to hook-ledger/hot-exclusions.txt (read by the stamping codemod like stamp-exclude.txt).

  python tools/hook_counts.py --counts DIR [--per-frame 2.0] [--write]
"""
import argparse, glob, json, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
LEDGER = os.path.join(HERE, "..", "hook-ledger")


def load_keys():
    keys = {}
    for path in glob.glob(os.path.join(LEDGER, "*.json")):
        if os.path.basename(path) == "meta.json":
            continue
        with open(path, encoding="utf-8") as f:
            shard = json.load(f)
        for e in shard.get("entries", []):
            if e.get("status") == "live":
                keys[e["id"]] = e["key"]
    return keys


def source_ids():
    """Ids of hooks written by hand in the sources (`#[yoake_hook(.., id = N, ..)]`): the stamping overlay does not own
    them, so the cost gate reports them but does not exclude them (they have their own bench gate)."""
    ids = set()
    root = os.path.join(HERE, "..", "crates")
    for path in glob.glob(os.path.join(root, "*", "src", "**", "*.rs"), recursive=True):
        with open(path, encoding="utf-8", errors="ignore") as f:
            for m in re.finditer(r"yoake_hook\([^)]*?\bid\s*=\s*(\d+)", f.read()):
                ids.add(int(m.group(1)))
    return ids


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--counts", required=True, help="directory with hook-counts.<scene>.txt files (searched recursively)")
    ap.add_argument("--per-frame", type=float, default=2.0, help="entries per frame above which a function is hot")
    ap.add_argument("--write", action="store_true", help="write hook-ledger/hot-exclusions.txt")
    args = ap.parse_args()
    keys = load_keys()
    total = {}
    frames = 0
    files = sorted(glob.glob(os.path.join(args.counts, "**", "hook-counts.*.txt"), recursive=True))
    if not files:
        sys.exit("no hook-counts files under " + args.counts)
    for path in files:
        with open(path, encoding="utf-8") as f:
            lines = f.read().splitlines()
        frames += int(lines[0].split()[2])
        for line in lines[1:]:
            slot, n = line.split()
            total[int(slot)] = total.get(int(slot), 0) + int(n)
    rows = sorted(((n / frames, slot, n) for slot, n in total.items()), reverse=True)
    all_entries = sum(total.values())
    manual = source_ids()
    hot_all = [(pf, slot, n) for pf, slot, n in rows if pf > args.per_frame]
    hot = [r for r in hot_all if r[1] not in manual]
    print(f"{len(hot_all) - len(hot)} of the hot functions are hand-written hooks (not excluded here)")
    print(f"{len(files)} run(s), {frames} frames, {len(total)} hooked functions entered, {all_entries / frames:.1f} entries per frame in all")
    hot_entries = sum(n for _, _, n in hot)
    print(f"{len(hot)} functions above {args.per_frame}/frame carry {100 * hot_entries / max(1, all_entries):.1f}% of all entries")
    for pf, slot, n in rows[:40]:
        mark = "HOT" if pf > args.per_frame else "   "
        print(f"{mark} {pf:10.2f}/frame {n:12d}  {slot:8d}  {keys.get(slot, '?')}")
    if args.write:
        out = os.path.join(LEDGER, "hot-exclusions.txt")
        # cumulative: an earlier round's keys stay (their ids are tombstoned, so the next count never sees them)
        kept = {}
        if os.path.exists(out):
            with open(out, encoding="utf-8") as f:
                for line in f:
                    if line.strip() and not line.startswith("#"):
                        key, _, why = line.rstrip().partition(" # ")
                        kept[key] = why
        for pf, slot, n in hot:
            if slot in keys:
                kept[keys[slot]] = "hot: %.2f entries per frame" % pf
        with open(out, "w", encoding="utf-8", newline=chr(10)) as f:
            f.write("# W6: functions the cost gate measured as hot (tools/hook_counts.py): rounds so far > 2 and > 0.5 entries per frame\n")
            f.write("# in the pinned benchmark (flights and world loads). Not stamped; read like stamp-exclude.txt. Cumulative.\n")
            for key in sorted(kept):
                f.write(f"{key} # {kept[key]}\n")
        print("wrote", out, len(kept), "keys")


if __name__ == "__main__":
    main()
