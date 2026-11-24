#!/usr/bin/env python3
"""The FR-F2 freedoms of a WebAssembly mod in a real game (FR-REM05): files, an HTTP POST, a TCP echo, and a worker
whose blocking network calls leave the game's tick running.

    python3 tools/f2_e2e.py --bin-dir target/debug --wasm target/wasm-guests/wasm32-wasip2/release/yoake_wasm_f2_check_server.wasm \
        --worker target/wasm-guests/wasm32-wasip2/release/yoake_wasm_f2_check_worker.wasm [--resources .local/horizon-client/26.3]

The mod is `crates/yoake-wasm-sdk/examples/f2-check` (its yak.toml, a server module and a worker component). It is installed
as the game reads installed packages (`yoake-mods.txt`, `yoake-mods/<id>/<version>/`) in a scratch game folder, and the game
runs with YOAKE_WASM_UNSIGNED=1 (developer mode). One game, `--offscreen --no-focus --no-pause`: nothing takes the mouse or
the focus. Every server the checks talk to is started by this script on 127.0.0.1, and nothing else is reached.

The main instance writes `/data/x.txt` and reads it back, and writes `/game/f2-check-game.txt`. The worker polls
`/data/jobs.txt` (this script appends the jobs) and answers each on the game log as `f2-check job <id> ...`:
- post:   an HTTP POST (wasi:http) to this script's HTTP server; the server must get the body and the mod the reply;
- tcp:    a TCP connection to this script's echo server; the echo must come back;
- slow:   the same POST, to a path that waits 2 s before it answers: the game's tick must keep advancing meanwhile.

Checks (each one entry in the JSON result):
- files:        the main instance reads back what it wrote in /data, and the file is in the game folder.
- http_post:    the server got the POST body and the mod logged the 200 and the reply.
- tcp_echo:     the echo server got the text and the mod logged it back.
- nonblocking:  while the worker waits 2 s for the server, the client's tick count rises by at least 25 (20 per second),
                and each `state` query answers at once (the game thread never stops).
- https:        not tested (needs external network: trusting a local self-signed certificate is not supported).
"""
import argparse
import json
import os
import shutil
import socket
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
from agent_control_e2e import Failed  # noqa: E402
from integration_env import require_server_reports  # noqa: E402
from lan_e2e import Game  # noqa: E402
from levels_e2e import other_games_running  # noqa: E402
from mob_sync_e2e import log_text, wait_for_quiet  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
SAMPLE = ROOT / 'crates/yoake-wasm-sdk/examples/f2-check'
MOD = 'f2-check'
SLOW_SECONDS = 2.0


def read_chunked(stream):
    """A request body sent with chunked transfer coding, decoded."""
    body = b''
    while True:
        size = int(stream.readline().strip().split(b';')[0], 16)
        if size == 0:
            stream.readline()
            return body.decode()
        body += stream.read(size)
        stream.readline()


class HttpHandler(BaseHTTPRequestHandler):
    """POST echoes `got:<body>`; `/slow...` answers after SLOW_SECONDS and sets the server's events."""

    def do_POST(self):
        if 'chunked' in self.headers.get('Transfer-Encoding', ''):
            body = read_chunked(self.rfile)
        else:
            body = self.rfile.read(int(self.headers.get('Content-Length', 0))).decode()
        self.server.posts.append((self.path, body))
        if self.path.startswith('/slow'):
            self.server.arrived.set()
            time.sleep(SLOW_SECONDS)
        reply = f'got:{body}'.encode()
        self.send_response(200)
        self.send_header('Content-Type', 'text/plain')
        self.send_header('Content-Length', str(len(reply)))
        self.end_headers()
        self.wfile.write(reply)
        if self.path.startswith('/slow'):
            self.server.finished.set()

    def log_message(self, *args):
        pass


def echo_connection(conn):
    with conn:
        data = b''
        while chunk := conn.recv(4096):
            data += chunk
        conn.sendall(data)


def echo_loop(listener):
    try:
        while True:
            conn, _ = listener.accept()
            threading.Thread(target=echo_connection, args=(conn,), daemon=True).start()
    except OSError:
        pass  # the listener was closed at the end of the run


def start_servers():
    http = ThreadingHTTPServer(('127.0.0.1', 0), HttpHandler)
    http.posts = []
    http.arrived = threading.Event()
    http.finished = threading.Event()
    threading.Thread(target=http.serve_forever, daemon=True).start()
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    listener.bind(('127.0.0.1', 0))
    listener.listen(8)
    threading.Thread(target=echo_loop, args=(listener,), daemon=True).start()
    return http, listener


def install_mod(work, server, worker):
    """The package where the game reads installed ones: the sample's yak.toml and its two wasm files."""
    root = work / 'yoake-mods' / MOD / '0.1.0'
    (root / 'bin/wasm').mkdir(parents=True)
    shutil.copyfile(SAMPLE / 'yak.toml', root / 'yak.toml')
    shutil.copyfile(server, root / 'bin/wasm/f2-check-server.wasm')
    shutil.copyfile(worker, root / 'bin/wasm/f2-check-worker.wasm')
    (work / 'yoake-mods.txt').write_text(f'{MOD} 0.1.0\n')
    (work / 'mod-data' / MOD).mkdir(parents=True, exist_ok=True)


def add_job(work, line):
    with open(work / 'mod-data' / MOD / 'jobs.txt', 'a', encoding='utf-8') as jobs:
        jobs.write(line + '\n')


def wait_log(game, needle, seconds):
    """The first log line with `needle`, waiting in game ticks (the game runs meanwhile)."""
    deadline = time.time() + seconds
    while True:
        hits = [line for line in log_text(game).splitlines() if needle in line]
        if hits:
            return hits[-1]
        if time.time() > deadline:
            raise Failed(f'no log line with {needle!r} within {seconds} s')
        game.wait(5)


def check(name, results, fn):
    try:
        detail = fn()
        results[name] = {'pass': True, 'detail': detail}
    except (Failed, AssertionError, KeyError, IndexError, TypeError, ValueError, OSError) as e:
        results[name] = {'pass': False, 'detail': repr(e)}
    print(f'[f2] {name}: {"PASS" if results[name]["pass"] else "FAIL"} {results[name]["detail"]}')


def run(ctl, out, resources, server, worker, force):
    if not force and (running := other_games_running()):
        sys.exit(f'another horizon-client is running ({running[0][:120]}): wait for it, or use --force')
    os.environ['YOAKE_WASM_UNSIGNED'] = '1'
    game = Game('f2', ctl, resources)
    http, listener = start_servers()
    http_url = f'http://127.0.0.1:{http.server_address[1]}'
    echo = f'127.0.0.1:{listener.getsockname()[1]}'
    results = {}
    try:
        install_mod(game.work, server, worker)
        game.pid = game.raw('launch', '--offscreen', '--no-pause', '--no-focus')['pid']
        game.raw('open', 'F2', 5, 'creative')
        game('wait_ready', seconds=400)
        check('files', results, lambda: files(game))
        check('http_post', results, lambda: http_post(game, http, http_url))
        check('tcp_echo', results, lambda: tcp_echo(game, echo))
        check('nonblocking', results, lambda: nonblocking(game, http, http_url))
        results['https'] = {'pass': True, 'skipped': True,
                            'detail': 'not tested (needs external network: a local self-signed certificate cannot be trusted)'}
        out.mkdir(parents=True, exist_ok=True)
        game.shot(out, 'end')
    except (Failed, AssertionError, KeyError, IndexError, TypeError, ValueError, OSError) as e:
        results.setdefault('error', {'pass': False, 'detail': repr(e)})
    finally:
        listener.close()
        text = game.close()
        (out / 'f2.log').write_text(text or '')
        shutil.rmtree(game.work, ignore_errors=True)
    return results


def files(game):
    line = wait_log(game, 'f2-check files ', 180)
    assert 'data=ok' in line and 'game=ok' in line, line
    game_file = game.work / 'f2-check-game.txt'
    assert game_file.read_text() == 'hello-game', f'{game_file} is not the file the mod wrote'
    return {'log': line, 'game_folder_file': str(game_file)}


def http_post(game, http, url):
    add_job(game.work, f'1 post {url}/echo hello-f2')
    line = wait_log(game, 'f2-check job 1 post ', 60)
    assert line.endswith('status=200 reply=got:hello-f2'), line
    assert ('/echo', 'hello-f2') in http.posts, http.posts
    return {'log': line, 'server_got': http.posts[-1]}


def tcp_echo(game, echo):
    add_job(game.work, f'2 tcp {echo} hello-tcp')
    line = wait_log(game, 'f2-check job 2 tcp ', 60)
    assert line.endswith('echo=hello-tcp'), line
    return {'log': line, 'address': echo}


def nonblocking(game, http, url):
    http.arrived.clear()
    http.finished.clear()
    before = game('state')['ticks']
    started = time.time()
    add_job(game.work, f'3 post {url}/slow hello-slow')
    assert http.arrived.wait(60), 'the slow request never reached the server'
    samples = [(time.time() - started, game('state')['ticks'])]
    while not http.finished.is_set() and time.time() - started < 30:
        game.wait(2)
        samples.append((time.time() - started, game('state')['ticks']))
    assert http.finished.is_set(), 'the slow request was never answered'
    elapsed = time.time() - started
    line = wait_log(game, 'f2-check job 3 post ', 60)
    ticks = [t for _, t in samples]
    gained = ticks[-1] - ticks[0]
    gaps = [b[0] - a[0] for a, b in zip(samples, samples[1:])]
    assert elapsed >= SLOW_SECONDS - 0.2, f'the server answered in {elapsed:.2f} s, not after {SLOW_SECONDS} s'
    assert gained >= 25, f'the tick moved {gained} (from {before}) during the {elapsed:.1f} s wait: the game stalled'
    assert line.endswith('status=200 reply=got:hello-slow'), line
    return {'ticks_during_wait': gained, 'wait_seconds': round(elapsed, 2), 'samples': len(samples),
            'longest_state_gap_seconds': round(max(gaps, default=0.0), 2), 'log': line}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', default=str(ROOT / 'target/debug'), help='folder with horizon-client and horizon-ctl')
    parser.add_argument('--wasm', required=True, help='the built server module (yoake_wasm_f2_check_server.wasm)')
    parser.add_argument('--worker', required=True, help='the built worker component (yoake_wasm_f2_check_worker.wasm)')
    parser.add_argument('--out', default=str(ROOT / '.local/f2-e2e'))
    parser.add_argument('--resources', default=os.environ.get('HORIZON_TEST_RESOURCES') or str(lab.RESOURCES),
                        help='the prepared resources (default: $HORIZON_TEST_RESOURCES, else .local/horizon-client/<version>)')
    parser.add_argument('--force', action='store_true', help='run even when another horizon-client is running')
    parser.add_argument('--wait-minutes', type=int, default=30, help="how long to wait for another run's games to end")
    args = parser.parse_args()
    bins = Path(args.bin_dir).resolve()
    resources = Path(args.resources).resolve()
    if not (resources / 'horizon-resources.json').exists():
        sys.exit(f'no prepared resources in {resources}')
    try:
        require_server_reports(resources)
    except ValueError as error:
        sys.exit(str(error))
    wait_for_quiet(args.wait_minutes)
    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    ext = '.exe' if sys.platform == 'win32' else ''
    results = run(bins / f'horizon-ctl{ext}', out, resources, Path(args.wasm).resolve(),
                  Path(args.worker).resolve(), args.force)
    (out / 'results.json').write_text(json.dumps({'checks': results}, indent=2) + '\n')
    print(json.dumps(results, indent=1))
    sys.exit(0 if results and all(r['pass'] for r in results.values()) else 1)


if __name__ == '__main__':
    main()
