#!/usr/bin/env python3
"""Seats of a custom vehicle (F4M-4d, FR-F4M-4d.md section 5) end to end, as far as the harness reaches.

    PYTHONUTF8=1 python tools/custom_mob_seats_e2e.py --bin-dir target/debug [--out target/custom-mob-seats-e2e]

The Cinder Fox fixture is copied to a scratch folder and patched into a vehicle: three seats (seat 0 `controls: player`, `yaw_lock`,
at the centre; seats 1 and 2 `controls: none`, half a block to either side, all 0.8 blocks up). A real LAN host and a Horizon guest
(`--offscreen --no-focus --no-pause`, nothing takes the focus).

Checks (the numbers are those of FR-F4M-4d.md section 5):
- ride3:    (1) `/ride <chicken> mount <vehicle>` three times: the chickens sit at the three seats (horizontal distance 0, 0.5, 0.5 from
            the vehicle, 0.8 up); a fourth is not taken aboard (it stays where it was).
- stay:     (3) one rider killed: the other two stay at their seats (no shuffling).
- guest:    (5) the guest's client has the riders at the same distances (they were aboard before it joined: the snapshot).
- vehicle:  (3) the vehicle killed: no rider is left at a seat (all at the ground, away from the vehicle's seats).

Not covered here (needs a person or a different harness): 2 right click (no `use` on an entity through horizon-ctl), 4 save and reload, 6 cooldown_persists,
7 `seat-set` from a guest module, 8 to 12 (a guest taking seats, the claims on disconnect, a native mod cancelling the mount, a locator seat): see FR-MERGE7.md.
"""
import argparse
import json
import math
import os
import shutil
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
from agent_control_e2e import Failed  # noqa: E402
from integration_env import require_server_reports  # noqa: E402
from lan_e2e import Game, pos_of  # noqa: E402
from mob_sync_e2e import wait_for_quiet  # noqa: E402
import custom_mob_render_e2e as r  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
CHICKEN = 'minecraft:chicken'
VEHICLE = '@e[type=sample_mobs:cinder_fox,limit=1]'


def vehicle_fixture(scratch):
    dest = scratch / 'cinder_fox'
    shutil.copytree(r.FIXTURE, dest)
    path = dest / r.TYPE
    data = json.loads(path.read_text(encoding='utf-8'))
    data['seats'] = [
        {'offset': [0.0, 0.8, 0.0], 'controls': 'player', 'yaw_lock': True},
        {'offset': [0.5, 0.8, 0.0], 'controls': 'none'},
        {'offset': [-0.5, 0.8, 0.0], 'controls': 'none'},
    ]
    ai = data['ai']
    ai['goals'] = [g for g in ai['goals'] if g['type'] == 'swim']
    ai['targets'] = []
    ai.pop('script_goals', None)
    path.write_text(json.dumps(data, indent=2), encoding='utf-8')
    return dest


def chickens(game, radius=40):
    return [e for e in game.raw('entities', radius)['entities'] if e.get('type') == CHICKEN]


def vehicle(game):
    found = [e for e in game.raw('entities', 40)['entities'] if e.get('type') == r.FOX]
    return found[0] if found else None


def offsets(game):
    """Each chicken's (horizontal distance, height) from the vehicle, sorted by horizontal distance."""
    v = vehicle(game)
    if v is None:
        return None
    out = []
    for c in chickens(game):
        out.append((round(math.hypot(c['pos'][0] - v['pos'][0], c['pos'][2] - v['pos'][2]), 2), round(c['pos'][1] - v['pos'][1], 2)))
    return sorted(out)


def run(ctl, out, resources):
    scratch = Path(tempfile.mkdtemp(prefix='hzseat-'))
    mod = vehicle_fixture(scratch)
    host, guest = Game('host', ctl, resources), Game('guest', ctl, resources)
    args = ['--offscreen', '--no-focus', '--no-pause', '--pregen', '--dev-mod', str(mod / 'content')]
    results = {}
    games = (host, guest)
    try:
        host.pid = host.raw('launch', *args)['pid']
        host.raw('open', 'Lan', 5, 'creative')
        host('wait_ready', seconds=400)
        host('command', command='gamemode creative')
        host('command', command='difficulty peaceful')
        host('command', command='time set day')
        host('command', command='gamerule spawn_mobs false')
        hx, hy, hz = (int(v // 1) for v in pos_of(host))
        host('command', command=f'fill {hx - 6} {hy - 1} {hz - 10} {hx + 6} {hy - 1} {hz + 4} minecraft:stone')
        host('command', command=f'fill {hx - 6} {hy} {hz - 10} {hx + 6} {hy + 5} {hz + 4} minecraft:air')
        host.wait(5)
        host('command', command=f'summon {r.FOX} {hx + 0.5} {hy} {hz - 4.5}')
        for i in range(1, 5):
            host('command', command=f'summon {CHICKEN} {hx - 3.5 + i} {hy} {hz - 1.5} {{Tags:["c{i}"]}}')
        host.wait(20)
        replies = []
        for i in range(1, 5):
            rep = host('command', command=f'ride @e[tag=c{i},limit=1] mount {VEHICLE}')
            replies.append(rep.get('chat'))
            host.wait(4)
        host.wait(10)
        three = offsets(host)
        taken = [o for o in three if o[0] < 1.0]
        results['ride3'] = {'pass': len(taken) == 3 and [o[0] for o in taken] == [0.0, 0.5, 0.5] and all(abs(o[1] - 0.8) < 0.15 for o in taken)
                            and len([o for o in three if o[0] >= 1.0]) == 1,
                            'detail': {'offsets_from_vehicle': three, 'ride_replies': replies}}
        # the guest joins after the riders are aboard: the snapshot
        guest.pid = guest.raw('launch', *args)['pid']
        guest.raw('connect', f"127.0.0.1:{host.raw('lan', 'open', 0, 'commands')['port']}", 'Guest')
        guest.raw('wait-for', 'loaded', '--timeout', 300)
        guest('command', command=f'tp @s {hx + 0.5} {hy} {hz - 1.5}')
        guest.wait(60)
        g_three = offsets(guest)
        g_taken = [o for o in (g_three or []) if o[0] < 1.0]
        results['guest'] = {'pass': bool(g_three) and [o[0] for o in g_taken] == [0.0, 0.5, 0.5],
                            'detail': {'offsets_on_the_guest': g_three}}
        # kill the chicken on seat 1: the others stay
        v = vehicle(host)
        seat1 = [c for c in chickens(host) if abs(math.hypot(c['pos'][0] - v['pos'][0], c['pos'][2] - v['pos'][2]) - 0.5) < 0.1]
        before = [o for o in offsets(host) if o[0] < 1.0]
        host('command', command=f'kill @e[type={CHICKEN},sort=nearest,limit=1,x={seat1[0]["pos"][0]},y={seat1[0]["pos"][1]},z={seat1[0]["pos"][2]},distance=..0.3]')
        host.wait(40)
        after = [o for o in offsets(host) if o[0] < 1.0]
        results['stay'] = {'pass': len(before) == 3 and len(after) == 2 and set(after) <= set(before),
                           'detail': {'before': before, 'after': after}}
        # the vehicle killed: the riders are off
        host('command', command=f'kill {VEHICLE}')
        host.wait(60)
        left = [c for c in chickens(host) if c['health'] > 0]
        v_gone = vehicle(host) is None
        results['vehicle'] = {'pass': v_gone and len(left) >= 1,
                              'detail': {'vehicle_removed': v_gone, 'chickens_left': len(left)}}
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
        shutil.rmtree(scratch, ignore_errors=True)
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', default=str(ROOT / 'target/debug'))
    parser.add_argument('--out', default=str(ROOT / 'target/custom-mob-seats-e2e'))
    parser.add_argument('--resources', default=os.environ.get('HORIZON_TEST_RESOURCES') or str(lab.RESOURCES))
    parser.add_argument('--wait-minutes', type=int, default=30)
    args = parser.parse_args()
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
    results = run(Path(args.bin_dir).resolve() / f'horizon-ctl{ext}', out, resources)
    (out / 'results.json').write_text(json.dumps({'checks': results}, indent=2) + '\n')
    print(json.dumps(results, indent=1))
    sys.exit(0 if results and all(x['pass'] for x in results.values()) else 1)


if __name__ == '__main__':
    main()
