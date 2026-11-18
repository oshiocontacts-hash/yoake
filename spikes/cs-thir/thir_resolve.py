"""Canonical callees and resolution stats for the written method calls in a thir_calls.py output.

usage: thir_resolve.py <calls json> <out json>
"""
import collections
import json
import re
import sys

REF = re.compile(r"^(&('\{erased\}|'[a-z_]+)?\s*(mut\s+)?)+")


def strip_refs(ty):
    return REF.sub("", ty or "").strip()


def first_generic(generics):
    m = re.search(r"value: \[(.*)", generics or "")
    if not m:
        return None
    text, depth, out = m.group(1), 0, []
    for ch in text:
        if ch in "<([{":
            depth += 1
        elif ch in ">)]}":
            if depth == 0:
                break
            depth -= 1
        elif ch == "," and depth == 0:
            break
        out.append(ch)
    return "".join(out).strip() or None


def generic_param(ty):
    # A bare type parameter (`T`, `S`) or an impl-Trait parameter anywhere in the type.
    return bool(re.search(r"(^|[<(,\s])[A-Z](?=$|[>),\s])", ty)) or "impl " in ty


def classify(call):
    callee = call["callee"]
    method = callee.rsplit("::", 1)[-1]
    recv = strip_refs(call.get("recv_ty"))
    if "{impl#" in callee:
        owner = recv
        key = f"{owner}::{method}"
        kind = "inherent"
    else:
        trait = callee.rsplit("::", 1)[0]
        owner = first_generic(call.get("generics")) or recv
        key = f"<{owner} as {trait}>::{method}"
        kind = "trait"
    if "dyn " in owner:
        res = "dyn (dispatched at run time)"
    elif generic_param(owner):
        res = "generic receiver"
    else:
        res = "concrete"
    return kind, res, key


def main():
    calls = json.load(open(sys.argv[1], encoding="utf-8"))
    written = [c for c in calls if c["kind"] == "method" and c["fn_span"]["ctxt"] == 0]
    delegations = [c for c in written if "__yoake_" in (c["callee"] or "")]
    real = [c for c in written if "__yoake_" not in (c["callee"] or "")]
    stats = collections.Counter()
    by_file = collections.defaultdict(collections.Counter)
    out = []
    for c in real:
        kind, res, key = classify(c)
        stats[(kind, res)] += 1
        f = c["fn_span"]["file"].rsplit("/", 1)[-1]
        by_file[f][res] += 1
        out.append({"file": c["fn_span"]["file"], "line": c["fn_span"]["line"], "col": c["fn_span"]["col"],
                    "caller_body": c["body"], "kind": kind, "resolution": res, "callee_key": key})
    json.dump(out, open(sys.argv[2], "w", encoding="utf-8"), indent=1)
    print("written method calls:", len(real), "+ codemod trait delegations:", len(delegations))
    for k, v in sorted(stats.items()):
        print(" ", k, v)
    for f, cnt in by_file.items():
        total = sum(cnt.values())
        print(f"  {f}: {total} method calls, concrete {cnt['concrete']} ({100*cnt['concrete']/total:.1f} %), "
              f"dyn {cnt['dyn (dispatched at run time)']}, generic {cnt['generic receiver']}")
    for row in out[:8]:
        print("   ", row["line"], row["col"], row["kind"], row["resolution"], row["callee_key"][:110])


if __name__ == "__main__":
    main()
