"""Stream a THIR tree dump and list the calls whose fn_span is in the target files.

usage: thir_calls.py <thir file> <out json> <file suffix>...
Each Call node: callee DefId path, generic args text, from_hir_call, fn_span, fun span, first arg
(receiver) type and span. Classification:
  overloaded  from_hir_call false (operators, autoderef: compiler-inserted, not a written call)
  method      written method-call syntax (receiver span starts before the method-name span)
  path        written path call (`f(x)`, `Type::f(x)`, `<T as Tr>::f(x)`)
"""
import json
import re
import sys

SPAN = re.compile(r"span: (.*?):(\d+):(\d+): (\d+):(\d+) \(#(\d+)\)")
FNDEF = re.compile(r"ty: FnDef\(DefId\(\d+:\d+ ~ (.*?)\), (.*)$")
HASH = re.compile(r"\[[0-9a-f]{4}\]")


def indent(line):
    return len(line) - len(line.lstrip(" "))


def norm_path(path):
    return HASH.sub("", path)


def parse_span(text):
    m = SPAN.search(text)
    if not m:
        return None
    return {"file": m.group(1).replace("\\", "/"), "line": int(m.group(2)), "col": int(m.group(3)),
            "end_line": int(m.group(4)), "end_col": int(m.group(5)), "ctxt": int(m.group(6))}


def main():
    thir_path, out_path, *suffixes = sys.argv[1:]
    suffixes = [s.replace("\\", "/") for s in suffixes]
    stack = []
    calls = []
    body = None
    with open(thir_path, encoding="utf-8", errors="replace") as thir:
        for raw in thir:
            line = raw.rstrip("\n")
            if not line.strip():
                continue
            ind = indent(line)
            text = line.strip()
            if ind == 0 and text.startswith("DefId("):
                body = norm_path(text.split(" ~ ", 1)[-1].rstrip("):"))
                stack.clear()
                continue
            while stack and ind <= stack[-1]["ind"] and text != "Call {":
                done = stack.pop()
                if done.get("keep"):
                    calls.append(done)
            if text == "Call {":
                while stack and ind <= stack[-1]["ind"]:
                    done = stack.pop()
                    if done.get("keep"):
                        calls.append(done)
                stack.append({"ind": ind, "body": body, "state": "head"})
                continue
            if not stack:
                continue
            cur = stack[-1]
            rel = ind - cur["ind"]
            if rel == 4:
                if text.startswith("ty: FnDef("):
                    m = FNDEF.match(text)
                    if m:
                        cur["callee"] = norm_path(m.group(1))
                        cur["generics"] = norm_path(m.group(2))[:400]
                elif text.startswith("from_hir_call:"):
                    cur["from_hir_call"] = text.endswith("true")
                elif text.startswith("fn_span:"):
                    span = parse_span(text.replace("fn_span:", "span:"))
                    cur["fn_span"] = span
                    cur["keep"] = bool(span and any(span["file"].endswith(s) for s in suffixes))
                elif text == "fun:":
                    cur["state"] = "fun"
                elif text == "args: [":
                    cur["state"] = "args"
                elif text == "]":
                    cur["state"] = "done"
            elif rel == 12 and cur["state"] == "fun" and text.startswith("span:") and "fun_span" not in cur:
                cur["fun_span"] = parse_span(text)
            elif rel == 12 and cur["state"] == "args":
                if text.startswith("ty:") and "recv_ty" not in cur:
                    cur["recv_ty"] = norm_path(text[3:].strip())[:300]
                elif text.startswith("span:") and "recv_span" not in cur:
                    cur["recv_span"] = parse_span(text)
    for done in reversed(stack):
        if done.get("keep"):
            calls.append(done)
    out = []
    for c in calls:
        if not c.get("from_hir_call", True):
            kind = "overloaded"
        else:
            fs, rs = c.get("fun_span"), c.get("recv_span")
            if fs and rs and (rs["line"], rs["col"]) < (fs["line"], fs["col"]):
                kind = "method"
            else:
                kind = "path"
        out.append({
            "kind": kind,
            "body": c.get("body"),
            "callee": c.get("callee"),
            "generics": c.get("generics"),
            "recv_ty": c.get("recv_ty"),
            "fn_span": c.get("fn_span"),
            "fun_span": c.get("fun_span"),
        })
    json.dump(out, open(out_path, "w", encoding="utf-8"), indent=1)
    by = {}
    for c in out:
        f = c["fn_span"]["file"].rsplit("/", 1)[-1]
        by.setdefault(f, {}).setdefault(c["kind"], 0)
        by[f][c["kind"]] += 1
    print(json.dumps(by, indent=1))


if __name__ == "__main__":
    main()
