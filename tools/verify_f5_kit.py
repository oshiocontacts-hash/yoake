#!/usr/bin/env python3
"""Verify every yoake-cli template (`new` -> `build` -> `check`) and every F1-F5 mod example (`build` -> `check`)."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

TEMPLATES = [
    "minigame",
    "hud-only",
    "custom-mob",
    "utility",
    "script",
    "client-full",
    "server-full",
    "worker",
    "full-access",
]


def run_step(cmd: list[str], cwd: Path, env: dict[str, str]) -> tuple[bool, str, float]:
    t0 = time.monotonic()
    proc = subprocess.run(
        cmd,
        cwd=str(cwd),
        env=env,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    dt = time.monotonic() - t0
    if proc.returncode == 0:
        return True, "", dt
    detail = (proc.stderr.strip() or proc.stdout.strip() or f"exit {proc.returncode}")
    # Trim very long compiler logs while keeping the end where the error is.
    if len(detail) > 1200:
        detail = "...\n" + detail[-1200:]
    return False, detail, dt


def discover_examples() -> list[tuple[str, Path]]:
    examples: list[tuple[str, Path]] = []
    wasm_examples = ROOT / "crates" / "yoake-wasm-sdk" / "examples"
    for entry in sorted(wasm_examples.iterdir()):
        if not entry.is_dir():
            continue
        if entry.name == "test-demo":
            continue
        if entry.name == "mod-call-pair":
            for sub in ("caller", "responder"):
                sub_dir = entry / sub
                if (sub_dir / "yak.toml").is_file():
                    examples.append((f"mod-call-pair/{sub}", sub_dir))
            continue
        if (entry / "yak.toml").is_file():
            examples.append((entry.name, entry))
    guest_f1_f2 = ROOT / "crates" / "yoake-guest-sdk" / "examples" / "f1-f2-demo"
    if (guest_f1_f2 / "yak.toml").is_file():
        examples.append(("yoake-guest-sdk/f1-f2-demo", guest_f1_f2))
    return examples


def find_yoake_bin(explicit: str | None) -> Path:
    if explicit:
        return Path(explicit)
    target_dir = Path(os.environ.get("CARGO_TARGET_DIR") or (ROOT / "target"))
    for profile in ("debug", "release"):
        for name in ("yoake.exe", "yoake"):
            cand = target_dir / profile / name
            if cand.is_file():
                return cand
    raise SystemExit("could not locate built yoake binary; pass --yoake-bin or build `-p yoake`")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--yoake-bin", default=None)
    parser.add_argument("--cargo-target-dir", default=None)
    parser.add_argument("--only", choices=("all", "templates", "examples"), default="all")
    parser.add_argument("--filter", nargs="+", default=None)
    parser.add_argument("--json-out", default=None)
    args = parser.parse_args()
    wanted = set(args.filter) if args.filter else None

    yoake = find_yoake_bin(args.yoake_bin)
    env = os.environ.copy()
    env["CARGO_NET_OFFLINE"] = "true"
    env.setdefault("CARGO_TERM_COLOR", "never")
    if args.cargo_target_dir:
        env["CARGO_TARGET_DIR"] = args.cargo_target_dir
    elif not env.get("CARGO_TARGET_DIR"):
        inferred = yoake.parent.parent
        if (inferred / "debug").is_dir() or (inferred / "release").is_dir():
            env["CARGO_TARGET_DIR"] = str(inferred)

    results: dict[str, list[dict[str, object]]] = {"templates": [], "examples": []}
    failures = 0

    if args.only in ("all", "templates"):
        with tempfile.TemporaryDirectory(prefix="yoake-tpl-") as tmp:
            tmp_root = Path(tmp)
            for tpl in TEMPLATES:
                if wanted is not None and tpl not in wanted:
                    continue
                name = f"tpl-{tpl}"
                ok_new, err_new, dt_new = run_step([str(yoake), "new", tpl, name], tmp_root, env)
                proj = tmp_root / name
                if ok_new:
                    ok_build, err_build, dt_build = run_step([str(yoake), "build"], proj, env)
                else:
                    ok_build, err_build, dt_build = False, "skipped (new failed)", 0.0
                if ok_build:
                    ok_check, err_check, dt_check = run_step([str(yoake), "check"], proj, env)
                else:
                    ok_check, err_check, dt_check = False, "skipped (build failed)", 0.0
                passed = ok_new and ok_build and ok_check
                if not passed:
                    failures += 1
                err = err_new or err_build or err_check
                print(
                    f"[template] {tpl:12s} new={'OK' if ok_new else 'FAIL'} "
                    f"build={'OK' if ok_build else 'FAIL'} check={'OK' if ok_check else 'FAIL'} "
                    f"({dt_new + dt_build + dt_check:.1f}s)",
                    flush=True,
                )
                if err:
                    print(f"  error: {err}", flush=True)
                results["templates"].append(
                    {
                        "name": tpl,
                        "new": ok_new,
                        "build": ok_build,
                        "check": ok_check,
                        "seconds": round(dt_new + dt_build + dt_check, 2),
                        "error": err,
                    }
                )

    if args.only in ("all", "examples"):
        for label, path in discover_examples():
            if wanted is not None and label not in wanted:
                continue
            ok_build, err_build, dt_build = run_step([str(yoake), "build"], path, env)
            if ok_build:
                ok_check, err_check, dt_check = run_step([str(yoake), "check"], path, env)
            else:
                ok_check, err_check, dt_check = False, "skipped (build failed)", 0.0
            passed = ok_build and ok_check
            if not passed:
                failures += 1
            err = err_build or err_check
            print(
                f"[example]  {label:28s} build={'OK' if ok_build else 'FAIL'} "
                f"check={'OK' if ok_check else 'FAIL'} ({dt_build + dt_check:.1f}s)",
                flush=True,
            )
            if err:
                print(f"  error: {err}", flush=True)
            results["examples"].append(
                {
                    "name": label,
                    "path": str(path.relative_to(ROOT)),
                    "build": ok_build,
                    "check": ok_check,
                    "seconds": round(dt_build + dt_check, 2),
                    "error": err,
                }
            )

    if args.json_out:
        out_path = Path(args.json_out)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(json.dumps(results, indent=2), encoding="utf-8")

    print(
        f"Summary: {len(results['templates'])} templates, {len(results['examples'])} examples, {failures} failed"
    )
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
