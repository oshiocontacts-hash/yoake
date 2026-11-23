#!/usr/bin/env python3
"""Levels end to end (F3-7): a LAN host whose world has two data pack levels (mypack:moon, mypack:hell) and a level
created while it runs, with real games and a protocol-level client, on the lan_e2e.py harness.

  python tools/levels_e2e.py --bin-dir target/debug [--resources .local/horizon-client/26.3]

Games run `--offscreen --no-pause --no-focus` only, in scratch folders of their own, on a port the OS picks: nothing opens a
window, takes the focus or touches the mouse, and it can run beside other harness runs (it refuses to start while another
horizon-client of this checkout is running, unless --force is given, so two e2e runs never overlap).

What it checks, in order:

- setup:    the host's world (flat pack levels, a type of their own each) opens to the LAN; the Overworld's level facts
            (shape, sky) are the vanilla ones.
- travel:   a Horizon guest runs `execute in mypack:moon run tp` and is in the moon: its chunks are 128 high from y 0
            (`status.level`), its type's sky (overworld sky with weather, ambient light 0.3) and sea level; it walks on the
            flat floor the server generated; `execute in mypack:hell` gives the Nether-like facts (no sky, Nether light);
            it goes back to the Overworld and has the vanilla facts again. The Overworld's blocks are where they were.
- custom portal: a pack-defined gold frame lights with fire, sends the guest to the moon at coordinate scale 4,
            generates a matching exit, returns to the source level through that link, and loses its fill when a frame block breaks.
- stock:    the protocol-level client (examples/level_probe.rs: the handshake, configuration and play a stock client
            does, nothing Horizon-only) joins, is sent the dimension type registry with the pack's types after the
            vanilla ones, the new moon biome in the biome registry, and the level keys in the login packet, runs the
            same `execute in` through chat, gets the respawn into the moon with the right type and chunks of the
            moon's section count carrying the new biome, and is not disconnected.
- created:  the host creates a level while the guest is connected (`horizon-ctl level create`, a flat level with a type
            of its own): the guest asking to enter it is refused with the reconnect message and stays where it is;
            after it reconnects it can enter, stands on the created level's floor, and the level's folder and
            data/yoake_levels.json exist; a client that joins after the creation can enter at once.
- lifecycle: unload (closed, registered, back with its edit), regenerate with a new seed (the edit is gone), delete
            (refused while someone is in it; then folder, registry entry and file entry gone).
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
import lan_e2e as lan  # noqa: E402
from agent_control_e2e import Failed  # noqa: E402
from integration_env import require_server_reports  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
OVERWORLD = 'minecraft:overworld'
MOON = 'mypack:moon'
HELL = 'mypack:hell'
MADE = 'made:arena'
MOON_BIOME = 'mypack:luminous_moon'
PACK = 'saves/Lan/datapacks/e2e'

MOON_TYPE = {
    'min_y': 0, 'height': 128, 'logical_height': 128, 'has_skylight': True, 'has_ceiling': False,
    'ambient_light': 0.3, 'coordinate_scale': 1.0, 'default_clock': 'minecraft:overworld', 'skybox': 'overworld',
    'cardinal_light': 'default', 'timelines': '#minecraft:in_overworld',
    'attributes': {'minecraft:visual/sky_color': '#336699', 'minecraft:visual/fog_color': '#223344'},
}
HELL_TYPE = {
    'min_y': 0, 'height': 128, 'logical_height': 128, 'has_skylight': False, 'has_ceiling': True,
    'has_fixed_time': True, 'ambient_light': 0.1, 'coordinate_scale': 8.0, 'skybox': 'none', 'cardinal_light': 'nether',
    'monster_spawn_block_light_limit': 15, 'monster_spawn_light_level': 7, 'infiniburn': '#minecraft:infiniburn_nether',
    'attributes': {'minecraft:gameplay/fast_lava': True, 'minecraft:gameplay/water_evaporates': True,
                   'minecraft:gameplay/piglins_zombify': False, 'minecraft:gameplay/respawn_anchor_works': True},
}
# A level of the type's own (inline), 64 high from y 0: a type no client has until it joins after the creation.
MADE_STEM = {
    'type': {'min_y': 0, 'height': 64, 'logical_height': 64, 'has_skylight': True, 'has_ceiling': False,
             'ambient_light': 0.0, 'skybox': 'overworld'},
    'generator': {'type': 'minecraft:flat', 'settings': {
        'biome': 'minecraft:plains', 'features': False, 'lakes': False,
        'layers': [{'block': 'minecraft:bedrock', 'height': 1}, {'block': 'minecraft:stone', 'height': 3},
                   {'block': 'minecraft:grass_block', 'height': 1}]}},
}


def portal_lines(game):
    """The host's custom-portal log lines (the trip, the exit found/built, the arrival)."""
    log = game.socket.with_suffix('.log')
    text = log.read_text(errors='replace') if log.exists() else ''
    return [l for l in text.splitlines() if 'Horizon custom portal:' in l][-12:]

def flat(type_id, biome='minecraft:plains'):
    return {'type': type_id, 'generator': {'type': 'minecraft:flat', 'settings': {
        'biome': biome, 'features': False, 'lakes': False,
        'layers': [{'block': 'minecraft:bedrock', 'height': 1}, {'block': 'minecraft:stone', 'height': 3},
                   {'block': 'minecraft:grass_block', 'height': 1}]}}}


def write_pack(work):
    """The pack of the host's world: the two types and the two flat levels."""
    files = {
        'pack.mcmeta': {'pack': {'pack_format': 1, 'description': 'levels e2e'}},
        'data/mypack/dimension_type/moon_type.json': MOON_TYPE,
        'data/mypack/dimension_type/hell_type.json': HELL_TYPE,
        'data/mypack/worldgen/biome/luminous_moon.json': {
            'has_precipitation': False, 'temperature': 0.2, 'downfall': 0.0,
            'effects': {'grass_color': '#34aa88', 'foliage_color': '#228866',
                        'water_color': '#3377aa', 'water_fog_color': '#112233'},
            'attributes': {'minecraft:visual/sky_color': '#112244',
                           'minecraft:visual/fog_color': '#223355'},
            'features': [], 'carvers': [],
        },
        'data/mypack/dimension/moon.json': flat('mypack:moon_type', MOON_BIOME),
        'data/mypack/dimension/hell.json': flat('mypack:hell_type'),
        'data/mypack/horizon/portal/moon.json': {
            'frame': 'minecraft:gold_block', 'ignite': {'fire': True},
            'size': {'min_width': 2, 'max_width': 4, 'min_height': 3, 'max_height': 4},
            'target': MOON, 'scale': 4.0,
            'look': {'ambient_sound': 'minecraft:block.portal.ambient',
                     'travel_sound': 'minecraft:block.portal.trigger', 'particle': 'minecraft:portal'},
        },
    }
    for name, value in files.items():
        path = work / PACK / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value))


def other_games_running():
    """Another horizon-client of this checkout running (another e2e's), so this run waits instead of overlapping."""
    if sys.platform != 'win32':
        out = subprocess.run(['pgrep', '-af', 'horizon-client'], capture_output=True, text=True).stdout
        return [line for line in out.splitlines() if 'pgrep' not in line and str(ROOT) in line]
    out = subprocess.run(['powershell', '-NoProfile', '-Command',
                          "Get-CimInstance Win32_Process -Filter \"Name='horizon-client.exe'\" | "
                          "Select-Object -ExpandProperty ExecutablePath"],
                         capture_output=True, text=True).stdout
    return [line for line in out.splitlines() if line.strip()]


def level(game):
    return game.raw('status')['level']


def stand_in(game, dimension, x, z, floor_y):
    """`execute in DIMENSION run tp` onto a spot above the flat floor, once the level has loaded around it."""
    game('command', command=f'execute in {dimension} run tp @s {x + 0.5} {floor_y + 1} {z + 0.5}')
    state = lan.wait_dimension(game, dimension, 600)
    game.wait(40)
    return state


def check(name, results, fn):
    try:
        detail = fn()
        results[name] = {'pass': True, 'detail': detail}
    except (Failed, AssertionError, KeyError, StopIteration, TypeError, IndexError, subprocess.SubprocessError) as e:
        results[name] = {'pass': False, 'detail': repr(e)}
    print(f'[levels] {name}: {"PASS" if results[name]["pass"] else "FAIL"} {results[name]["detail"]}')


def probe(path, address, name, resources, command, seconds=60):
    """The protocol-level client: its JSON lines, one object per line."""
    result = subprocess.run([str(path), address, name, str(resources), command, str(seconds)],
                            capture_output=True, text=True, timeout=seconds + 60)
    lines = [json.loads(line) for line in result.stdout.splitlines() if line.startswith('{')]
    print(f'[probe] {name} exit {result.returncode}: {lines}')
    return result.returncode, lines


def events_since(game, since):
    return game.raw('events', '--since', since)['events']


def run(ctl, probe_bin, out, resources, force):
    if not force and (running := other_games_running()):
        sys.exit(f'another horizon-client is running ({running[0][:120]}): wait for it, or use --force')
    host, guest, late = (lan.Game(n, ctl, resources) for n in ('lvhost', 'lvguest', 'lvlate'))
    launch = ['launch', '--offscreen', '--no-pause', '--no-focus']
    results = {}
    try:
        write_pack(host.work)
        host.pid = host.raw(*launch)['pid']
        host.raw('open', 'Lan', 5, 'creative')
        host('wait_ready', seconds=600)
        host('command', command='gamemode creative')
        host('command', command='time set day')
        host('command', command='gamerule spawn_mobs false')
        port = host.raw('lan', 'open', 0, 'commands')['port']
        address = f'127.0.0.1:{port}'
        lanname = host.raw('lan', 'status').get('host', 'Player')
        print(f'[levels] host {lanname} on {address} in {host.work}')

        def setup():
            first = level(host)
            assert (first['id'], first['min_y'], first['height'], first['sky']) == (OVERWORLD, -64, 384, 'overworld'), first
            listed = [l['id'] for l in host.raw('level', 'list')['levels']]
            assert listed == [OVERWORLD, 'minecraft:the_nether', 'minecraft:the_end', HELL, MOON], listed
            return first
        check('setup', results, setup)
        if not results['setup']['pass']:
            return results

        guest.pid = guest.raw(*launch)['pid']
        guest.raw('connect', address, lan.GUEST_NAME)
        guest.raw('wait-for', 'loaded', '--timeout', 300)
        guest.wait(40)
        guest('command', command='gamemode creative')
        home = lan.pos_of(guest)
        home_block = guest.raw('block', int(home[0]), int(home[1]) - 1, int(home[2]))

        def travel():
            seen = {}
            stand_in(guest, MOON, 8, 8, 4)
            moon = level(guest)
            assert (moon['id'], moon['min_y'], moon['height'], moon['sky'], moon['weather']) == (MOON, 0, 128, 'overworld', True), moon
            assert abs(moon['ambient'] - 0.3) < 1e-6, moon
            floor = guest.raw('block', 8, 4, 8)
            assert floor.get('block') == 'minecraft:grass_block', floor
            assert guest.raw('block', 8, 0, 8).get('block') == 'minecraft:bedrock', 'bedrock at the moon floor'
            seen['moon'] = moon
            guest.shot(out, 'moon')
            stand_in(guest, HELL, 8, 8, 4)
            hell = level(guest)
            assert (hell['id'], hell['sky'], hell['weather'], hell['nether_light']) == (HELL, 'none', False, True), hell
            seen['hell'] = hell
            # Back to the Overworld, with the vanilla facts and the blocks where they were.
            guest('command', command=f'execute in {OVERWORLD} run tp @s {home[0]:.2f} {home[1]:.2f} {home[2]:.2f}')
            state = lan.wait_dimension(guest, OVERWORLD, 600)
            guest.wait(40)
            back = level(guest)
            assert (back['id'], back['min_y'], back['height'], back['sky'], back['weather']) == (OVERWORLD, -64, 384, 'overworld', True), back
            assert back['ambient'] == 0.0 and state['dimension'] == OVERWORLD, back
            assert guest.raw('block', int(home[0]), int(home[1]) - 1, int(home[2])) == home_block
            # The host stayed in the Overworld all along.
            assert host('state').get('dimension') == OVERWORLD
            seen['overworld'] = back
            return seen
        check('travel', results, travel)

        def custom_portal():
            # Whatever happens, the later checks start with the guest in the Overworld.
            try:
                return custom_portal_steps()
            finally:
                guest('command', command=f'execute in {OVERWORLD} run tp @s 0.5 100 0.5')
                lan.wait_dimension(guest, OVERWORLD, 600)
                guest.wait(20)

        def custom_portal_steps():
            guest('command', command='gamerule players_nether_portal_creative_delay 100')
            stand_in(guest, OVERWORLD, 24, 24, 4)
            for x in (22, 25):
                for y in range(4, 9):
                    guest('command', command=f'setblock {x} {y} 24 minecraft:gold_block')
            for x in (23, 24):
                for y in (4, 8):
                    guest('command', command=f'setblock {x} {y} 24 minecraft:gold_block')
                for y in range(5, 8):
                    guest('command', command=f'setblock {x} {y} 24 minecraft:air')
            # Let the server apply the frame first; then light it, read the server's block (the host), retry the
            # fire up to three times (a fire can go out before it is read).
            guest.wait(10)
            lit = None
            for _ in range(3):
                guest('command', command='setblock 23 5 24 minecraft:fire')
                guest.wait(5)
                lit = host.raw('block', 23, 5, 24)
                if lit.get('block') == 'minecraft:nether_portal':
                    break
            assert lit.get('block') == 'minecraft:nether_portal', {'block': lit, 'portal_lines': portal_lines(host)}
            # The guest fell to the world's real floor while the frame was built (the frame stands in the air): put it
            # inside the lit portal, on the frame's bottom, so it really enters it.
            # (No block read after this: the guest may already be in the moon.)
            guest('command', command='tp @s 23.5 5 24.5')
            arrived = lan.wait_dimension(guest, MOON, 600)
            guest.wait(40)
            now = level(guest)
            assert now['id'] == MOON, {'level': now, 'state_after_wait': {k: arrived.get(k) for k in ('dimension', 'player')},
                                       'pos': lan.pos_of(guest), 'portal_lines': portal_lines(host)}
            exit_pos = lan.pos_of(guest)
            assert abs(exit_pos[0] - 24.5 / 4.0) < 20, exit_pos
            assert abs(exit_pos[2] - 24.5 / 4.0) < 20, exit_pos
            guest('command', command=f'execute in {MOON} run tp @s 0.5 10 0.5')
            guest.wait(30)
            guest('command', command=f'execute in {MOON} run tp @s {exit_pos[0]:.3f} {exit_pos[1]:.3f} {exit_pos[2]:.3f}')
            lan.wait_dimension(guest, OVERWORLD, 600)
            guest.wait(20)
            returned = lan.pos_of(guest)
            guest('command', command=f'execute in {OVERWORLD} run tp @s 0.5 100 0.5')
            guest('command', command='setblock 22 5 24 minecraft:air')
            guest.wait(5)
            broken = guest.raw('block', 23, 5, 24)
            assert broken.get('block') == 'minecraft:air', broken
            return {'moon_exit': exit_pos, 'overworld_return': returned, 'broken_fill': broken}
        check('custom_portal', results, custom_portal)

        def stock():
            code, lines = probe(probe_bin, address, 'Stock', resources,
                                f'execute in {MOON} run tp @s 8 5 8')
            assert code == 0, f'the probe was disconnected or failed: exit {code} {lines}'
            by = {k: v for line in lines for k, v in line.items()}
            types = by['dimension_types']
            assert types[:4] == ['minecraft:overworld', 'minecraft:overworld_caves', 'minecraft:the_end', 'minecraft:the_nether'] \
                or 'minecraft:overworld' in types, types
            assert 'mypack:moon_type' in types and 'mypack:hell_type' in types, types
            assert types.index('mypack:moon_type') >= 4, 'the pack types come after the vanilla ones'
            login = by['login']
            assert OVERWORLD in login['levels'] and MOON in login['levels'] and HELL in login['levels'], login
            assert login['dimension'] == OVERWORLD and login['type'] == 'minecraft:overworld', login
            respawn = by['respawn']
            assert respawn['dimension'] == MOON and respawn['type'] == 'mypack:moon_type', respawn
            assert by['chunks_after_respawn'] > 0, by
            # 128 blocks: 8 sections, in the type's own range (the Overworld's would be 24).
            assert by['sections'] == [8], by
            assert MOON_BIOME in by['biomes'], 'the new biome joined the synchronized registry'
            assert by['chunk_biomes'] == [MOON_BIOME], by
            assert 'disconnected' not in by
            return {'types': types, 'login': login, 'respawn': respawn, 'sections': by['sections'],
                    'biome': MOON_BIOME}
        check('stock', results, stock)

        def created():
            host.raw('level', 'create', MADE, json.dumps(MADE_STEM, separators=(',', ':')))
            listed = [l['id'] for l in host.raw('level', 'list')['levels']]
            assert MADE in listed, listed
            assert (host.work / 'saves/Lan/data/yoake_levels.json').exists()
            # The guest joined before: its registry lacks the type. It is refused and stays.
            since = guest.raw('events')['last']
            guest('command', command=f'execute in {MADE} run tp @s 4.5 6 4.5')
            heard = ''
            for _ in range(60):
                heard = ' '.join(e.get('text', '') for e in events_since(guest, since) if e.get('event') == 'chat')
                if 'reconnect' in heard:
                    break
                guest.wait(5)
            assert 'reconnect to enter' in heard and MADE in heard, f'the guest was told: {heard!r}'
            assert guest('state').get('dimension') == OVERWORLD, 'it stays in the Overworld'
            # It reconnects and enters.
            guest.raw('disconnect')
            time.sleep(5)
            guest.raw('connect', address, lan.GUEST_NAME)
            guest.raw('wait-for', 'loaded', '--timeout', 300)
            guest.wait(40)
            guest('command', command='gamemode creative')
            stand_in(guest, MADE, 4, 4, 4)
            made = level(guest)
            assert (made['id'], made['min_y'], made['height'], made['sky']) == (MADE, 0, 64, 'overworld'), made
            assert guest.raw('block', 4, 4, 4).get('block') == 'minecraft:grass_block'
            guest.shot(out, 'made')
            folder = host.work / 'saves/Lan/dimensions/made/arena'
            guest('command', command='setblock 4 20 4 minecraft:gold_block')
            guest.wait(20)
            # A client that joins after the creation is sent the type and the level key and can enter at once.
            late.pid = late.raw(*launch)['pid']
            late.raw('connect', address, 'Late')
            late.raw('wait-for', 'loaded', '--timeout', 300)
            late.wait(40)
            late('command', command='gamemode creative')
            stand_in(late, MADE, 8, 8, 4)
            assert level(late)['id'] == MADE and level(late)['height'] == 64
            # And the protocol-level client of a stock player is told the level in its login.
            code, lines = probe(probe_bin, address, 'Stock2', resources, f'execute in {MADE} run tp @s 8 5 8')
            by = {k: v for line in lines for k, v in line.items()}
            assert code == 0 and MADE in by['login']['levels'], (code, lines)
            assert by['respawn']['dimension'] == MADE and by['sections'] == [4], by
            return {'level': made, 'login_levels': by['login']['levels'], 'folder': str(folder)}
        check('created', results, created)

        def lifecycle():
            # Occupied: guest and late are in it.
            try:
                host.raw('level', 'delete', MADE)
                raise AssertionError('deleted a level with players in it')
            except Failed as e:
                assert 'has players in it' in str(e), e
            for g in (guest, late):
                g('command', command=f'execute in {OVERWORLD} run tp @s 0.5 100 0.5')
                lan.wait_dimension(g, OVERWORLD, 600)
            host.wait(60)
            folder = host.work / 'saves/Lan/dimensions/made/arena'
            host.raw('level', 'unload', MADE)
            opened = {l['id']: l['open'] for l in host.raw('level', 'list')['levels']}
            assert opened[MADE] is False, opened
            assert folder.exists(), 'unloading keeps the level\'s folder'
            stand_in(guest, MADE, 4, 4, 4)
            assert guest.raw('block', 4, 20, 4).get('block') == 'minecraft:gold_block', 'the edit came back'
            guest('command', command=f'execute in {OVERWORLD} run tp @s 0.5 100 0.5')
            lan.wait_dimension(guest, OVERWORLD, 600)
            guest.wait(60)
            host.raw('level', 'regenerate', MADE, 77)
            stand_in(guest, MADE, 4, 4, 4)
            assert guest.raw('block', 4, 20, 4).get('block') == 'minecraft:air', 'regenerated: the edit is gone'
            guest('command', command=f'execute in {OVERWORLD} run tp @s 0.5 100 0.5')
            lan.wait_dimension(guest, OVERWORLD, 600)
            guest.wait(60)
            host.raw('level', 'delete', MADE)
            assert not folder.exists()
            assert MADE not in [l['id'] for l in host.raw('level', 'list')['levels']]
            assert not (host.work / 'saves/Lan/data/yoake_levels.json').exists()
            for vanilla in (HELL, MOON):
                assert vanilla in [l['id'] for l in host.raw('level', 'list')['levels']], 'pack levels stay'
            return 'unload, regenerate and delete'
        check('lifecycle', results, lifecycle)
    finally:
        logs = {g.name: g.close() for g in (late, guest, host)}
        for name, text in logs.items():
            (out / f'{name}.log').write_text(text)
        for g in (host, guest, late):
            shutil.rmtree(g.work, ignore_errors=True)
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', required=True, help='the folder with horizon-client, horizon-ctl and examples/level_probe')
    parser.add_argument('--resources', default=os.environ.get('HORIZON_TEST_RESOURCES') or str(lab.RESOURCES))
    parser.add_argument('--out', default=str(ROOT / '.local/levels-e2e'))
    parser.add_argument('--force', action='store_true', help='run even when another horizon-client is running')
    args = parser.parse_args()
    bins = Path(args.bin_dir).resolve()
    exe = '.exe' if sys.platform == 'win32' else ''
    resources = Path(args.resources).resolve()
    if not (resources / 'horizon-resources.json').exists():
        sys.exit(f'no prepared resources in {resources}')
    try:
        require_server_reports(resources)
    except ValueError as error:
        sys.exit(str(error))
    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    results = run(bins / f'horizon-ctl{exe}', bins / 'examples' / f'level_probe{exe}', out, resources, args.force)
    (out / 'results.json').write_text(json.dumps({'checks': results}, indent=2) + '\n')
    print(json.dumps(results, indent=1))
    sys.exit(0 if results and all(r['pass'] for r in results.values()) else 1)


if __name__ == '__main__':
    main()
