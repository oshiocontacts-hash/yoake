#!/usr/bin/env python3
"""A WebAssembly mod's level end to end (F3-8d): a LAN host runs the `level-generator` sample mod (a server module that
registers the level levelgen:terraces with a chunk generator written in WebAssembly), a Horizon guest enters it and gets the
terraced chunks.

  python tools/levelgen_wasm_e2e.py --bin-dir target/debug --wasm target/wasm-guests/wasm32-wasip2/release/yoake_wasm_level_generator.wasm \
      [--resources .local/horizon-client/26.3]

The mod is installed as the game reads installed packages (`yoake-mods.txt`, `yoake-mods/<id>/<version>/`) in the host's scratch
game folder, and the host runs with YOAKE_WASM_UNSIGNED=1 (developer mode: the package is not signed). Games run
`--offscreen --no-pause --no-focus` in scratch folders of their own on a port the OS picks, and the script refuses to start
while another horizon-client is running (unless --force), so it never overlaps another run.

Checks, in order:
- level:   the host lists levelgen:terraces beside the three vanilla levels.
- travel:  the guest runs `execute in levelgen:terraces run tp`, is in that level with its type's facts (y 0, 64 high), and
           the blocks it sees are the terraces the guest generated: bedrock at y 0, stone above it, and in every column
           sampled a grass block whose top is 8 + 4 * ((x div 8 + z div 8) mod 4) + b, with b (0 or 1, from the level's
           seed) the same everywhere; air above it. Neighbouring terraces differ by 4, so it is not a flat floor.
- back:    the guest goes back to the Overworld and has the vanilla facts.
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
import lan_e2e as lan  # noqa: E402
from agent_control_e2e import Failed  # noqa: E402
from integration_env import require_server_reports  # noqa: E402
import levels_e2e as lv  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
LEVEL = 'levelgen:terraces'
BASED = 'levelgen:based'
SAMPLE = ROOT / 'crates/yoake-wasm-sdk/examples/level-generator'


def install_mod(work, wasm):
    """The package where the game reads installed ones: the sample's yak.toml and its server module."""
    root = work / 'yoake-mods/level-generator/0.1.0'
    (root / 'bin/wasm').mkdir(parents=True)
    shutil.copyfile(SAMPLE / 'yak.toml', root / 'yak.toml')
    shutil.copyfile(wasm, root / 'bin/wasm/level-generator-server.wasm')
    (work / 'yoake-mods.txt').write_text('level-generator 0.1.0\n')


def install_pack(work):
    """A fixed gold-block feature in the based level's custom biome."""
    root = work / 'saves/Lan/datapacks/levelgen'
    files = {
        'pack.mcmeta': {'pack': {'pack_format': 1, 'description': 'level generator e2e'}},
        'data/levelgen/worldgen/feature/gold.json': {
            'type': 'minecraft:simple_block',
            'config': {'to_place': {'type': 'minecraft:simple_state_provider',
                                    'state': {'Name': 'minecraft:gold_block'}}},
        },
        'data/levelgen/worldgen/placed_feature/gold.json': {
            'feature': 'levelgen:gold',
            'placement': [{'type': 'minecraft:fixed_placement', 'positions': [[0, 48, 0]]}],
        },
        'data/levelgen/worldgen/biome/decorated.json': {
            'has_precipitation': False, 'temperature': 0.8, 'downfall': 0.4,
            'effects': {'grass_color': '#91bd59', 'foliage_color': '#77ab2f',
                        'water_color': '#3f76e4', 'water_fog_color': '#050533'},
            'features': [['levelgen:gold']], 'carvers': [],
        },
    }
    for name, value in files.items():
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value))


def terrace(x, z, b):
    return 8 + 4 * ((x // 8 + z // 8) % 4) + b


def top_of(game, x, z):
    """The y of the highest grass block of a column, looking down from y 40."""
    for y in range(40, 0, -1):
        if game.raw('block', x, y, z).get('block') == 'minecraft:grass_block':
            return y
    raise AssertionError(f'no grass in the column {x} {z}')


def run(ctl, out, resources, wasm, force):
    if not force and (running := lv.other_games_running()):
        sys.exit(f'another horizon-client is running ({running[0][:120]}): wait for it, or use --force')
    os.environ['YOAKE_WASM_UNSIGNED'] = '1'
    host, guest = (lan.Game(n, ctl, resources) for n in ('lgwhost', 'lgwguest'))
    launch = ['launch', '--offscreen', '--no-pause', '--no-focus']
    results = {}
    try:
        install_mod(host.work, wasm)
        install_pack(host.work)
        host.pid = host.raw(*launch)['pid']
        host.raw('open', 'Lan', 5, 'creative')
        host('wait_ready', seconds=600)
        host('command', command='gamemode creative')
        host('command', command='time set day')
        host('command', command='gamerule spawn_mobs false')
        port = host.raw('lan', 'open', 0, 'commands')['port']
        address = f'127.0.0.1:{port}'
        print(f'[levelgen] host on {address} in {host.work}')

        def listed():
            ids = [l['id'] for l in host.raw('level', 'list')['levels']]
            assert ids[:3] == [lv.OVERWORLD, 'minecraft:the_nether', 'minecraft:the_end'], ids
            assert LEVEL in ids and BASED in ids, ids
            return ids
        lv.check('level', results, listed)
        if not results['level']['pass']:
            return results

        guest.pid = guest.raw(*launch)['pid']
        guest.raw('connect', address, lan.GUEST_NAME)
        guest.raw('wait-for', 'loaded', '--timeout', 300)
        guest.wait(40)
        guest('command', command='gamemode creative')
        home = lan.pos_of(guest)

        def travel():
            lv.stand_in(guest, LEVEL, 4, 4, 20)
            facts = lv.level(guest)
            assert (facts['id'], facts['min_y'], facts['height']) == (LEVEL, 0, 64), facts
            columns = [(4, 4), (12, 4), (4, 12), (20, 4), (28, 28), (40, 17), (3, 35)]
            tops = {c: top_of(guest, *c) for c in columns}
            b = tops[columns[0]] - terrace(*columns[0], 0)
            assert b in (0, 1), f'the offset of the seed is {b}: {tops}'
            for (x, z), top in tops.items():
                assert top == terrace(x, z, b), f'column {x} {z}: grass at {top}, terrace says {terrace(x, z, b)} (b={b})'
                assert guest.raw('block', x, top, z).get('block') == 'minecraft:grass_block'
                assert guest.raw('block', x, top + 1, z).get('block') == 'minecraft:air', 'air above the terrace'
                assert guest.raw('block', x, 0, z).get('block') == 'minecraft:bedrock', 'bedrock at y 0'
                assert guest.raw('block', x, 1, z).get('block') == 'minecraft:stone', 'stone above it'
            assert len(set(tops.values())) > 1, f'a flat floor: {tops}'
            guest.shot(out, 'terraces')
            return {'facts': facts, 'tops': {f'{x},{z}': y for (x, z), y in tops.items()}, 'b': b}
        lv.check('travel', results, travel)

        def based_and_decorated():
            lv.stand_in(guest, BASED, 4, 4, 80)
            facts = lv.level(guest)
            assert (facts['id'], facts['min_y'], facts['height']) == (BASED, -64, 384), facts
            blocks = {
                'base': guest.raw('block', 0, -64, 0).get('block'),
                'source': guest.raw('block', 2, 48, 0).get('block'),
                'feature': guest.raw('block', 0, 48, 0).get('block'),
                'cleared': guest.raw('block', 1, 48, 0).get('block'),
            }
            assert blocks == {
                'base': 'minecraft:bedrock',
                'source': 'minecraft:diamond_block',
                'feature': 'minecraft:gold_block',
                'cleared': 'minecraft:air',
            }, blocks
            guest.shot(out, 'based')
            return {'facts': facts, 'blocks': blocks}
        lv.check('based', results, based_and_decorated)

        def back():
            guest('command', command=f'execute in {lv.OVERWORLD} run tp @s {home[0]:.2f} {home[1]:.2f} {home[2]:.2f}')
            lan.wait_dimension(guest, lv.OVERWORLD, 600)
            guest.wait(40)
            facts = lv.level(guest)
            assert (facts['id'], facts['min_y'], facts['height']) == (lv.OVERWORLD, -64, 384), facts
            return facts
        lv.check('back', results, back)
    finally:
        logs = {g.name: g.close() for g in (guest, host)}
        for name, text in logs.items():
            if text:
                (out / f'{name}.log').write_text(text)
        for g in (host, guest):
            shutil.rmtree(g.work, ignore_errors=True)
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', required=True, help='the folder with horizon-client and horizon-ctl')
    parser.add_argument('--wasm', required=True, help="the built sample module (yoake_wasm_level_generator.wasm)")
    parser.add_argument('--resources', default=os.environ.get('HORIZON_TEST_RESOURCES') or str(lab.RESOURCES))
    parser.add_argument('--out', default=str(ROOT / '.local/levelgen-wasm-e2e'))
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
    results = run(bins / f'horizon-ctl{exe}', out, resources, Path(args.wasm).resolve(), args.force)
    (out / 'results.json').write_text(json.dumps({'checks': results}, indent=2) + '\n')
    print(json.dumps(results, indent=1))
    sys.exit(0 if results and all(r['pass'] for r in results.values()) else 1)


if __name__ == '__main__':
    main()
