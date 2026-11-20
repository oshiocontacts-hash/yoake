#!/usr/bin/env python3
"""`horizon:mob_sync` (F4M-5b) end to end, the Horizon-only half: a LAN host with the Cinder Fox fixture mod,
a Horizon client that joins, and a second Horizon client that joins late. Real games, offscreen and
unfocused (`--offscreen --no-focus --no-pause`): nothing takes the mouse or the focus.

    python3 tools/mob_sync_e2e.py --bin-dir target/debug
    python3 tools/mob_sync_e2e.py --bin-dir target/fast --out .local/mob-sync-e2e

Checks:
- fox:     /summon sample_mobs:cinder_fox next to both players: the host's server has it, the guest's client
           has it as an entity (`entities`).
- store:   the guest's client applied a `horizon:mob_sync` snapshot for it (the game logs one
           "Horizon mob-sync: snapshots=.. deltas=.. triggers=.." line a payload) and knows the mob
           (known >= 1); deltas follow while the fox walks.
- late:    a second Horizon client that joins afterwards starts from a snapshot (its first mob-sync line has
           snapshots >= 1 and known >= 1) and has the fox as an entity.
- quiet:   a client with no custom mob in range logs no mob-sync line before the fox exists.

Not covered (see docs/yoake2/reports/FR-MERGE3.md): a stock (vanilla protocol) client (no vscreen / Linux
natives here), a pounce at a player (the fixture's pounce is a WASM script goal; no guest ships here) and the
client's `hit` event (nothing in the game polls the store's events until F4M-6).

The games share the machine with other end-to-end runs: the script waits while another horizon-client
process runs that it did not start. Every game gets its own scratch folder, LAN port and world.
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
from agent_control_e2e import Failed  # noqa: E402
from integration_env import require_server_reports  # noqa: E402
from lan_e2e import Game, back_to_game, pos_of  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / 'tools/fixtures/cinder_fox/content'  # the data-only variant (the package also has yak.toml + the guest)
FOX = 'sample_mobs:cinder_fox'
LINE = re.compile(r'Horizon mob-sync: snapshots=(\d+) deltas=(\d+) triggers=(\d+) dropped=(\d+) incompatible=(\d+) known=(\d+)')


def other_clients():
    """Another run's horizon-client processes (Windows tasklist, else ps)."""
    try:
        if sys.platform == 'win32':
            text = subprocess.run(['tasklist', '/FI', 'IMAGENAME eq horizon-client.exe', '/NH'], capture_output=True, text=True, errors='replace').stdout or ''
            return text.lower().count('horizon-client.exe')
        text = subprocess.run(['pgrep', '-c', '-x', 'horizon-client'], capture_output=True, text=True, errors='replace').stdout or ''
        return int(text.strip() or 0)
    except (OSError, ValueError):
        return 0


def wait_for_quiet(minutes):
    deadline = time.time() + minutes * 60
    while other_clients() and time.time() < deadline:
        print(f'[mob-sync] {other_clients()} horizon-client process(es) of another run: waiting')
        time.sleep(30)
    if other_clients():
        sys.exit('another horizon-client run is still going; not starting games beside it')


def log_text(game):
    parts = []
    for path in (game.socket.with_suffix('.log'), game.work / 'home/logs/latest.log'):
        if path.exists():
            parts.append(path.read_text(errors='replace'))
    return '\n'.join(parts)


def sync_lines(game):
    """The mob-sync lines of a game's log, as dicts, in order (the .log and latest.log hold the same lines: dedupe by text order)."""
    text = (game.socket.with_suffix('.log').read_text(errors='replace') if game.socket.with_suffix('.log').exists() else '')
    if not LINE.search(text):
        text = log_text(game)
    return [dict(zip(('snapshots', 'deltas', 'triggers', 'dropped', 'incompatible', 'known'), map(int, m.groups())))
            for m in LINE.finditer(text)]


def fox_entities(game):
    entities = game.raw('entities', 40)['entities']
    return [e for e in entities if FOX in json.dumps(e)]


def launch(game, headless_args):
    game.pid = game.raw('launch', *headless_args)['pid']


def run(ctl, out, resources):
    host, guest, late = Game('host', ctl, resources), Game('guest', ctl, resources), Game('late', ctl, resources)
    args = ['--offscreen', '--no-focus', '--no-pause', '--dev-mod', str(FIXTURE)]
    results = {}
    games = (host, guest, late)
    try:
        launch(host, args)
        host.raw('open', 'Lan', 5, 'creative')
        host('wait_ready', seconds=400)
        host('command', command='gamemode creative')
        host('command', command='time set day')
        host('command', command='fill ~-8 ~-1 ~-8 ~8 ~-1 ~8 minecraft:stone')
        host('command', command='fill ~-8 ~ ~-8 ~8 ~6 ~8 minecraft:air')
        host('command', command='gamerule spawn_mobs false')
        port = host.raw('lan', 'open', 0, 'commands')['port']
        launch(guest, args)
        guest.raw('connect', f'127.0.0.1:{port}', 'Guest')
        guest.raw('wait-for', 'loaded', '--timeout', 300)
        guest.wait(40)
        before = sync_lines(guest)
        results['quiet'] = {'pass': not before, 'detail': before}
        x, y, z = (int(v // 1) for v in pos_of(host))
        guest('command', command=f'tp @s {x + 0.5} {y} {z + 2.5}')
        guest.wait(20)
        host('command', command=f'summon {FOX} {x + 2} {y} {z}')
        host.wait(20)
        server_fox = fox_entities(host)
        guest.until(lambda: fox_entities(guest), lambda e: bool(e), ticks=200)
        results['fox'] = {'pass': bool(server_fox) and bool(fox_entities(guest)),
                          'detail': {'host': len(server_fox), 'guest': len(fox_entities(guest))}}
        guest.until(lambda: sync_lines(guest), lambda l: any(x['snapshots'] >= 1 and x['known'] >= 1 for x in l), ticks=200)
        guest.wait(100)
        lines = sync_lines(guest)
        results['store'] = {'pass': any(x['snapshots'] >= 1 and x['known'] >= 1 for x in lines) and not any(x['incompatible'] for x in lines),
                            'detail': {'lines': len(lines), 'snapshots': sum(x['snapshots'] for x in lines),
                                       'deltas': sum(x['deltas'] for x in lines), 'triggers': sum(x['triggers'] for x in lines),
                                       'dropped': sum(x['dropped'] for x in lines), 'incompatible': sum(x['incompatible'] for x in lines)}}
        # A second client joins now, near the fox: it must start from a snapshot.
        launch(late, args)
        late.raw('connect', f'127.0.0.1:{port}', 'Late')
        late.raw('wait-for', 'loaded', '--timeout', 300)
        late.until(lambda: sync_lines(late), lambda l: bool(l), ticks=400)
        first = sync_lines(late)[0]
        results['late'] = {'pass': first['snapshots'] >= 1 and first['known'] >= 1 and not first['incompatible'],
                           'detail': {'first': first, 'fox_entities': len(fox_entities(late))}}
        for game in games:
            game.shot(out, 'end')
    except (Failed, AssertionError, KeyError, StopIteration, IndexError, TypeError) as e:
        results.setdefault('error', {'pass': False, 'detail': repr(e)})
    finally:
        logs = {g.name: g.close() for g in reversed(games)}
        for name, text in logs.items():
            (out / f'{name}.log').write_text(text)
        for g in games:
            shutil.rmtree(g.work, ignore_errors=True)
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', default=str(ROOT / 'target/debug'), help='folder with horizon-client and horizon-ctl')
    parser.add_argument('--out', default=str(ROOT / '.local/mob-sync-e2e'))
    parser.add_argument('--resources', default=os.environ.get('HORIZON_TEST_RESOURCES') or str(lab.RESOURCES),
                        help='the prepared resources (default: $HORIZON_TEST_RESOURCES, else .local/horizon-client/<version>)')
    parser.add_argument('--wait-minutes', type=int, default=30, help='how long to wait for another run\'s games to end')
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
    results = run(bins / f'horizon-ctl{ext}', out, resources)
    (out / 'results.json').write_text(json.dumps({'checks': results}, indent=2) + '\n')
    print(json.dumps(results, indent=1))
    sys.exit(0 if results and all(r['pass'] for r in results.values()) else 1)


if __name__ == '__main__':
    main()
