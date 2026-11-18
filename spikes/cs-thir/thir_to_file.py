"""Dump a crate's THIR to a file (pipes fail with os error 1450), with a disk watchdog.

usage: thir_to_file.py <cargo -vv Running line file> <workspace root> <out file> [min free GB]
Kills rustc if free space on the out file's drive drops below the limit (default 15 GB).
"""
import importlib.util
import os
import shutil
import subprocess
import sys
import time

here = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("probe", os.path.join(here, "thir_probe.py"))
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


def main():
    line_file, root, out_file = sys.argv[1:4]
    min_free = float(sys.argv[4]) if len(sys.argv) > 4 else 15.0
    env = dict(os.environ)
    env.update(probe.cargo_env(line_file)[0])
    env["RUSTC_BOOTSTRAP"] = "1"
    argv = probe.rustc_argv(line_file)
    # Windows limits a command line to 32767 characters; rustc reads the rest from an @argfile.
    argfile = out_file + ".args"
    with open(argfile, "w", encoding="utf-8") as f:
        f.write(chr(10).join(argv[1:]) + chr(10))
    argv = [argv[0], "@" + argfile]
    start = time.time()
    with open(out_file, "wb") as out, open(out_file + ".stderr", "wb") as err:
        proc = subprocess.Popen(argv, cwd=root, env=env, stdout=out, stderr=err)
        killed = False
        while proc.poll() is None:
            time.sleep(2)
            free = shutil.disk_usage(os.path.dirname(os.path.abspath(out_file))).free / 1e9
            if free < min_free:
                proc.kill()
                killed = True
                break
        proc.wait()
    size = os.path.getsize(out_file)
    print(f"exit={proc.returncode} killed_for_disk={killed} sec={time.time() - start:.1f} "
          f"GB={size / 1e9:.2f}")


if __name__ == "__main__":
    main()
