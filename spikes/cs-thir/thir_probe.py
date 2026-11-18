"""Run the crate's own rustc command with -Zunpretty=thir-tree and stream its stdout.

usage: thir_probe.py <cargo -v Running line file> <workspace root> sample <out.txt>
Prints a small sample around the first method-syntax Call (from_hir_call: false) and stops.
"""
import os
import shlex
import subprocess
import sys


def cargo_env(line_file):
    """Env vars from a `cargo -vv` Running line (`set K='V'&& set K2=V2&& rustc ...`)."""
    text = open(line_file, encoding="utf-8").read().strip()
    body = text[text.index("`") + 1:text.rindex("`")]
    env = {}
    while body.startswith("set "):
        item, body = body[4:].split("&& ", 1)
        key, value = item.split("=", 1)
        if len(value) >= 2 and value[0] == value[-1] == "'":
            value = value[1:-1]
        env[key] = value
    return env, body


def rustc_argv(line_file):
    _, body = cargo_env(line_file)
    text = "`" + body + "`"
    start = text.index("`") + 1
    end = text.rindex("`")
    # cargo prints Windows paths with backslashes, quoting only some tokens with '...':
    # split without POSIX escapes and strip the quotes ourselves.
    argv = []
    for token in shlex.split(text[start:end], posix=False):
        if len(token) >= 2 and token[0] == token[-1] and token[0] in "'\"":
            token = token[1:-1]
        argv.append(token)
    out = []
    skip = False
    for i, a in enumerate(argv):
        if skip:
            skip = False
            continue
        if a.startswith("--emit="):
            continue
        if a == "--cfg" and i + 1 < len(argv) and argv[i + 1].startswith("yoake_thir_probe"):
            skip = True
            continue
        if a.startswith("--error-format") or a.startswith("--json="):
            continue
        out.append(a)
    out.append("-Zunpretty=thir-tree")
    return out


def stream(argv, root, line_file=None):
    env = dict(os.environ)
    if line_file:
        env.update(cargo_env(line_file)[0])
    env["RUSTC_BOOTSTRAP"] = "1"
    proc = subprocess.Popen(argv, cwd=root, env=env, stdout=subprocess.PIPE,
                            stderr=subprocess.DEVNULL, text=True, encoding="utf-8",
                            errors="replace", bufsize=1 << 20)
    return proc


def main():
    line_file, root, mode, out_path = sys.argv[1:5]
    argv = rustc_argv(line_file)
    proc = stream(argv, root, line_file)
    window = []
    found_at = None
    with open(out_path, "w", encoding="utf-8") as out:
        for n, line in enumerate(proc.stdout):
            window.append(line)
            if len(window) > 80:
                window.pop(0)
            if found_at is None and "from_hir_call: false" in line:
                found_at = n
            if found_at is not None and n - found_at > 40:
                out.writelines(window)
                break
    proc.kill()
    print("lines read", n, "found", found_at)


if __name__ == "__main__":
    main()
