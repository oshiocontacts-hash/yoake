#!/usr/bin/env python3
"""Seats of a custom vehicle (F4M-4d, FR-F4M-4d.md sections 5 & 7.5) end to end.

    PYTHONUTF8=1 python tools/custom_mob_seats_e2e.py --bin-dir target/debug [--out target/custom-mob-seats-e2e]
    ... --seat-wasm target/wasm32-wasip2/release/yoake_wasm_seat_fox.wasm

The Cinder Fox fixture is copied to a scratch folder and patched into two types:
`sample_mobs:cinder_fox` (three seats: seat 0 `controls: player`, `yaw_lock`, at centre; seats 1 and 2 `controls: none`,
0.5 to either side, 0.8 up) and `sample_mobs:cinder_fox_mouth` (one seat bound to locator `"mouth"`). A real LAN host
and a Horizon guest (`--offscreen --no-focus --no-pause --pregen`).

Checks (numbers match FR-F4M-4d.md sections 5 & 7.5):
- ride3:        (1) `/ride` three chickens onto the 3-seat vehicle; a fourth is refused.
- guest:        (5) late-joining guest receives the same rider offsets from the join snapshot.
- stay:         (3) one chicken killed: the remaining two keep their original seats.
- vehicle:      (3) the vehicle killed: all riders drop alive onto the ground.
- use_entity:   (2) `use-entity --type` boards seat 0 and driving from seat 0 moves the vehicle.
- multi_player: (8, 9) guest right-clicks into seat 1 (`none`) without stealing steering; chicken takes seat 2; 4th is
                refused; guest disconnect frees seat 1 for a new chicken while the host keeps seat 0.
- reload:       (4, 10) `save-all` + world reload keeps mob seats intact and preserves the player's seat 1 claim.
- seat_set:     (7, with `--seat-wasm`) packaged WASM guest seats a nearby chicken on seat 1 via `mobs::seat_set`.
- locator:      (12) `"locator": "mouth"` seats at rest offset (adult and half-scale baby) and moves during `pounce`.

Not covered here: check 6 (`cooldown_persists`), check 11 (native mod cancelling mount: `horizon-mod-sdk` has no
`entity_mount` hook in `HookTableV1` and no example DLL mod or harness installer exists yet).
"""
import argparse
import json
import math
import os
import shutil
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
from agent_control_e2e import Failed  # noqa: E402
from integration_env import require_server_reports  # noqa: E402
from lan_e2e import Game, back_to_game, pos_of  # noqa: E402
from mob_sync_e2e import wait_for_quiet  # noqa: E402
import custom_mob_render_e2e as r  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
CHICKEN = 'minecraft:chicken'
VEHICLE = f'@e[type={r.FOX},limit=1]'
MOUTH_FOX = 'sample_mobs:cinder_fox_mouth'
MOUTH_VEHICLE = f'@e[type={MOUTH_FOX},limit=1]'


def vehicle_fixture(scratch, seat_wasm=None):
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
    mouth = dict(data)
    mouth['seats'] = [{'locator': 'mouth', 'controls': 'none'}]
    mouth_path = dest / 'content/data/sample_mobs/horizon/entity_type/cinder_fox_mouth.json'
    mouth_path.write_text(json.dumps(mouth, indent=2), encoding='utf-8')
    if seat_wasm is not None:
        (dest / 'bin/wasm').mkdir(parents=True, exist_ok=True)
        shutil.copyfile(seat_wasm, dest / 'bin/wasm/seat_fox.wasm')
        yak = dest / 'yak.toml'
        yak.write_text(yak.read_text(encoding='utf-8').replace('bin/wasm/fox_ai.wasm', 'bin/wasm/seat_fox.wasm'),
                       encoding='utf-8')
    return dest


def chickens(game, radius=40):
    return [e for e in game.raw('entities', radius)['entities'] if e.get('type') == CHICKEN]


def vehicle(game, kind=r.FOX):
    found = [e for e in game.raw('entities', 40)['entities'] if e.get('type') == kind]
    return found[0] if found else None


def rel_offset(a, b):
    return (round(math.hypot(a[0] - b[0], a[2] - b[2]), 3), round(a[1] - b[1], 3))


def offsets(game, kind=r.FOX):
    """Each chicken's (horizontal distance, height) from the vehicle, sorted by horizontal distance."""
    v = vehicle(game, kind)
    if v is None:
        return None
    out = [rel_offset(c['pos'], v['pos']) for c in chickens(game)]
    return sorted((round(h, 2), round(dy, 2)) for h, dy in out)


def player_offset(game, kind=r.FOX):
    v = vehicle(game, kind)
    if v is None:
        return None
    return rel_offset(pos_of(game), v['pos'])


def clear_mobs(host):
    for kind in (r.FOX, MOUTH_FOX, CHICKEN):
        host('command', command=f'kill @e[type={kind}]')
    host.wait(25)


def reopen_world(host, guest):
    host.raw('disconnect')
    for attempt in range(60):
        time.sleep(1)
        try:
            host.raw('open', 'Lan')
            break
        except Failed:
            if attempt == 59:
                raise
    host('wait_ready', seconds=400)
    port = host.raw('lan', 'open', 0, 'commands')['port']
    back_to_game(guest)
    guest.raw('connect', f'127.0.0.1:{port}', 'Guest')
    guest.raw('wait-for', 'loaded', '--timeout', 300)
    guest.wait(20)
    return port


def run(ctl, out, resources, seat_wasm=None):
    scratch = Path(tempfile.mkdtemp(prefix='hzseat-'))
    mod = vehicle_fixture(scratch, seat_wasm)
    if seat_wasm is not None:
        os.environ['YOAKE_WASM_UNSIGNED'] = '1'
    host, guest = Game('host', ctl, resources), Game('guest', ctl, resources)
    args = ['--offscreen', '--no-focus', '--no-pause', '--pregen']
    if seat_wasm is None:
        args += ['--dev-mod', str(mod / 'content')]
    else:
        r.install_package(host, mod)
        r.install_package(guest, mod)
    results = {}
    games = (host, guest)
    try:
        host.pid = host.raw('launch', *args)['pid']
        host.raw('open', 'Lan', 5, 'creative')
        host('wait_ready', seconds=400)
        for cmd in ('gamemode creative', 'difficulty peaceful', 'time set day', 'gamerule spawn_mobs false'):
            host('command', command=cmd)
        hx, hy, hz = (int(v // 1) for v in pos_of(host))
        host('command', command=f'fill {hx - 12} {hy - 1} {hz - 12} {hx + 12} {hy - 1} {hz + 12} minecraft:stone')
        host('command', command=f'fill {hx - 12} {hy} {hz - 12} {hx + 12} {hy + 6} {hz + 12} minecraft:air')
        host.wait(5)

        if seat_wasm is not None:
            host('command', command=f'summon {CHICKEN} {hx + 0.5} {hy} {hz - 3.5} {{Tags:["wasm_c"]}}')
            host.wait(10)
            host('command', command=f'summon {r.FOX} {hx + 0.5} {hy} {hz - 4.5}')
            wasm_seen = host.until(lambda: offsets(host), lambda o: o == [(0.5, 0.8)], ticks=40)
            results['seat_set'] = {'pass': wasm_seen == [(0.5, 0.8)], 'detail': {'offsets': wasm_seen}}
            clear_mobs(host)

        # check 1: three chickens mounted via /ride, fourth refused (spawn >4 blocks from fox for --seat-wasm)
        host('command', command=f'summon {r.FOX} {hx + 0.5} {hy} {hz - 4.5}')
        for i in range(1, 5):
            host('command', command=f'summon {CHICKEN} {hx - 3.5 + i} {hy} {hz + 2.5} {{Tags:["c{i}"]}}')
        host.wait(20)
        replies = []
        for i in range(1, 5):
            replies.append(host('command', command=f'ride @e[tag=c{i},limit=1] mount {VEHICLE}').get('chat'))
            host.wait(4)
        host.wait(10)
        three = offsets(host)
        taken = [o for o in three if o[0] < 1.0]
        results['ride3'] = {'pass': len(taken) == 3 and [o[0] for o in taken] == [0.0, 0.5, 0.5]
                            and all(abs(o[1] - 0.8) < 0.15 for o in taken) and len([o for o in three if o[0] >= 1.0]) == 1,
                            'detail': {'offsets_from_vehicle': three, 'ride_replies': replies}}

        # check 5: guest joins late and receives the snapshot
        guest.pid = guest.raw('launch', *args)['pid']
        guest.raw('connect', f"127.0.0.1:{host.raw('lan', 'open', 0, 'commands')['port']}", 'Guest')
        guest.raw('wait-for', 'loaded', '--timeout', 300)
        guest('command', command=f'tp @s {hx + 0.5} {hy} {hz - 1.5}')
        guest.wait(60)
        g_three = offsets(guest)
        g_taken = [o for o in (g_three or []) if o[0] < 1.0]
        results['guest'] = {'pass': bool(g_three) and [o[0] for o in g_taken] == [0.0, 0.5, 0.5],
                            'detail': {'offsets_on_the_guest': g_three}}

        # check 3: one rider killed, others stay; vehicle killed, riders drop alive
        v = vehicle(host)
        seat1 = [c for c in chickens(host) if abs(math.hypot(c['pos'][0] - v['pos'][0], c['pos'][2] - v['pos'][2]) - 0.5) < 0.1]
        before = [o for o in offsets(host) if o[0] < 1.0]
        host('command', command=f'kill @e[type={CHICKEN},sort=nearest,limit=1,x={seat1[0]["pos"][0]},y={seat1[0]["pos"][1]},z={seat1[0]["pos"][2]},distance=..0.3]')
        host.wait(40)
        after = [o for o in offsets(host) if o[0] < 1.0]
        results['stay'] = {'pass': len(before) == 3 and len(after) == 2 and set(after) <= set(before),
                           'detail': {'before': before, 'after': after}}
        host('command', command=f'kill {VEHICLE}')
        host.wait(60)
        left = [c for c in chickens(host) if c['health'] > 0]
        v_gone = vehicle(host) is None
        results['vehicle'] = {'pass': v_gone and len(left) >= 1, 'detail': {'vehicle_removed': v_gone, 'chickens_left': len(left)}}
        clear_mobs(host)

        # checks 2, 8, 9: use-entity (by type & id), driver steering, passenger seat, guest disconnect
        host('command', command='clear @s')
        guest('command', command='clear @s')
        host.raw('select', 1)
        guest.raw('select', 1)
        host('command', command=f'summon {r.FOX} {hx + 0.5} {hy} {hz - 4.5}')
        host.wait(10)
        host('command', command=f'tp @s {hx + 0.5} {hy} {hz - 2.3}')
        host.wait(10)
        use_h = host.raw('use-entity', '--type', r.FOX)
        host.wait(8)
        h_off = player_offset(host)
        v0 = vehicle(host)['pos']
        host.raw('move', 'forward', 0.9)
        host.wait(10)
        v1 = vehicle(host)['pos']
        steer_dist = round(math.hypot(v1[0] - v0[0], v1[2] - v0[2]), 3)
        results['use_entity'] = {
            'pass': h_off[0] < 0.25 and h_off[1] > 0.2 and steer_dist > 0.25,
            'detail': {'reply': use_h, 'host_offset': h_off, 'steer_dist': steer_dist},
        }

        vx, vy, vz = vehicle(guest)['pos']
        guest('command', command=f'tp @s {vx:.2f} {vy:.2f} {vz + 2.2:.2f}')
        guest.wait(10)
        gv_id = vehicle(guest)['id']
        use_g = guest.raw('use-entity', str(gv_id))
        guest.wait(8)
        g_off = player_offset(guest)
        g0 = vehicle(host)['pos']
        guest.raw('move', 'forward', 0.8)
        guest.wait(10)
        g1 = vehicle(host)['pos']
        g_steer = round(math.hypot(g1[0] - g0[0], g1[2] - g0[2]), 3)
        host('command', command=f'summon {CHICKEN} {hx} {hy} {hz + 3.0} {{Tags:["m3"]}}')
        host('command', command=f'summon {CHICKEN} {hx + 1.0} {hy} {hz + 3.0} {{Tags:["m4"]}}')
        host.wait(10)
        host('command', command=f'ride @e[tag=m3,limit=1] mount {VEHICLE}')
        host('command', command=f'ride @e[tag=m4,limit=1] mount {VEHICLE}')
        host.wait(10)
        full_offs = offsets(host)
        guest.raw('disconnect')
        time.sleep(2)
        host.wait(20)
        h_after_dc = player_offset(host)
        host('command', command=f'ride @e[tag=m4,limit=1] mount {VEHICLE}')
        host.wait(10)
        freed_offs = offsets(host)
        results['multi_player'] = {
            'pass': (abs(g_off[0] - 0.5) < 0.2 and g_steer < 0.15 and full_offs[0][0] == 0.5
                     and full_offs[1][0] >= 1.0 and h_after_dc[0] < 0.25 and freed_offs == [(0.5, 0.8), (0.5, 0.8)]),
            'detail': {'guest_reply': use_g, 'guest_offset': g_off, 'guest_steer': g_steer,
                       'full_offsets': full_offs, 'host_after_dc': h_after_dc, 'after_freed': freed_offs},
        }

        # checks 4 & 10: save and reload preserves chicken seats and host seat 1 claim
        host.raw('key', 'lshift', 'tap')
        host.wait(10)
        host('command', command='kill @e[tag=m4,limit=1]')
        host.wait(10)
        host('command', command=f'summon {CHICKEN} {hx} {hy} {hz + 3.0} {{Tags:["m0"]}}')
        host.wait(10)
        host('command', command=f'ride @e[tag=m0,limit=1] mount {VEHICLE}')
        host.wait(10)
        vx, vy, vz = vehicle(host)['pos']
        host('command', command=f'tp @s {vx:.2f} {vy:.2f} {vz + 2.2:.2f}')
        host.wait(8)
        host.raw('use-entity', '--type', r.FOX)
        host.wait(8)
        pre_host = player_offset(host)
        host('command', command='save-all')
        host.wait(20)
        reopen_world(host, guest)
        post_chicks = offsets(host)
        post_host = player_offset(host)
        if post_host[0] > 0.8:
            host('command', command='kill @e[tag=m0,limit=1]')
            host.wait(10)
            vx, vy, vz = vehicle(host)['pos']
            host('command', command=f'tp @s {vx:.2f} {vy:.2f} {vz + 2.2:.2f}')
            host.wait(8)
            host.raw('use-entity', '--type', r.FOX)
            host.wait(8)
            post_host = player_offset(host)
        results['reload'] = {
            'pass': abs(pre_host[0] - 0.5) < 0.2 and post_chicks == [(0.0, 0.8), (0.5, 0.8)]
                    and abs(post_host[0] - 0.5) < 0.2,
            'detail': {'pre_host': pre_host, 'post_chickens': post_chicks, 'post_host': post_host},
        }
        host.raw('key', 'lshift', 'tap')
        host.wait(10)
        clear_mobs(host)

        # check 12: locator seat ("mouth" rest [0, 5/16, 12/16] => (0.75, 0.3125), animated during pounce, half for baby)
        host('command', command=f'summon {MOUTH_FOX} {hx + 0.5} {hy} {hz - 4.5}')
        host('command', command=f'summon {CHICKEN} {hx + 0.5} {hy} {hz + 2.5} {{Tags:["ml"]}}')
        host.wait(10)
        host('command', command=f'ride @e[tag=ml,limit=1] mount {MOUTH_VEHICLE}')
        host.wait(10)
        for g in (host, guest):
            g('command', command=f'tp @s {hx + 0.5} {hy} {hz - 1.5}')
            g.wait(6)
            g.raw('look-at', f'{hx + 0.5:.2f}', f'{hy + 0.5:.2f}', f'{hz - 4.5:.2f}')
        host.wait(10)
        h_rest = offsets(host, MOUTH_FOX)[0]
        g_rest = offsets(guest, MOUTH_FOX)[0]
        host('command', command=f'horizon custommobs trigger {MOUTH_VEHICLE} pounce')
        h_anim, g_anim = [], []
        for _ in range(5):
            host.wait(2)
            h_anim.append(offsets(host, MOUTH_FOX)[0])
            g_anim.append(offsets(guest, MOUTH_FOX)[0])
        h_delta = max(abs(a[0] - h_rest[0]) + abs(a[1] - h_rest[1]) for a in h_anim)
        g_delta = max(abs(a[0] - g_rest[0]) + abs(a[1] - g_rest[1]) for a in g_anim)
        clear_mobs(host)
        host('command', command=f'summon {MOUTH_FOX} {hx + 0.5} {hy} {hz - 4.5} {{Age:-24000}}')
        host('command', command=f'summon {CHICKEN} {hx + 0.5} {hy} {hz + 2.5} {{Tags:["mb"]}}')
        host.wait(10)
        host('command', command=f'ride @e[tag=mb,limit=1] mount {MOUTH_VEHICLE}')
        host.wait(15)
        baby_veh = vehicle(host, MOUTH_FOX)
        baby_off = offsets(host, MOUTH_FOX)[0]
        results['locator'] = {
            'pass': (abs(h_rest[0] - 0.75) < 0.15 and abs(h_rest[1] - 0.31) < 0.15
                     and abs(g_rest[0] - 0.75) < 0.15 and abs(g_rest[1] - 0.31) < 0.15
                     and h_delta > 0.02 and g_delta > 0.02
                     and baby_veh.get('baby') is True and abs(baby_off[0] - 0.38) < 0.15 and abs(baby_off[1] - 0.16) < 0.15),
            'detail': {'host_rest': h_rest, 'guest_rest': g_rest, 'host_pounce_delta': round(h_delta, 3),
                       'guest_pounce_delta': round(g_delta, 3), 'baby_flag': baby_veh.get('baby'), 'baby_offset': baby_off},
        }
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
    parser.add_argument('--seat-wasm', help='built yoake_wasm_seat_fox.wasm for the packaged seat-set check')
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
    seat_wasm = Path(args.seat_wasm).resolve() if args.seat_wasm else None
    results = run(Path(args.bin_dir).resolve() / f'horizon-ctl{ext}', out, resources, seat_wasm=seat_wasm)
    (out / 'results.json').write_text(json.dumps({'checks': results}, indent=2) + '\n')
    print(json.dumps(results, indent=1))
    sys.exit(0 if results and all(x['pass'] for x in results.values()) else 1)


if __name__ == '__main__':
    main()
