#!/usr/bin/env python3
"""Behaviours of a WebAssembly mod's own content in a real game (REM-15): the `ruby-behaviors` sample package (a
server module that registers behaviour for its own block `ruby:lamp` and item `ruby:wand`, with their content files) runs
in one game, and a second game without the package stays vanilla. Offscreen and unfocused (`--offscreen --no-focus
--no-pause`): nothing takes the mouse or the focus.

    python tools/behaviors_e2e.py --bin-dir target/debug \
        --wasm target/wasm32-wasip2/release/yoake_wasm_ruby_behaviors.wasm [--out .local/behaviors-e2e]

The package is installed where the game reads installed ones (`yoake-mods.txt`, `yoake-mods/<id>/<version>/`) in the
game's scratch folder, and the game runs with YOAKE_WASM_UNSIGNED=1 (developer mode). The script waits while another
horizon-client runs.

Checks:
- registered: the game loaded the package (its objects register from `init`), and `ruby:lamp` places with its `lit` property.
             The game is a LAN host: a server module's hooks run on its integrated server.
- use:        right-clicking the lamp (`interact`) makes the guest flip `lit` and say so (block `use`): its lines alternate
             and the block ends as the last one says.
- wand:       using the wand in the air makes the guest tell the player a line (item `use`, the `tell` service).
- vanilla:    a game without the package refuses `ruby:lamp`, and right-clicking a vanilla block still works.
"""
import argparse
import json
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
from lan_e2e import Game, pos_of  # noqa: E402
from mob_sync_e2e import log_text, wait_for_quiet  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
SAMPLE = ROOT / 'crates/yoake-wasm-sdk/examples/ruby-behaviors'
PROBLEM = '問題箇所'  # "problem spot": the marker a refused command line carries


def install_package(game, wasm):
    """The sample as the game reads an installed package: yak.toml, the content folder and the server module."""
    root = game.work / 'yoake-mods/ruby/0.1.0'
    (root / 'bin/wasm').mkdir(parents=True)
    shutil.copyfile(SAMPLE / 'yak.toml', root / 'yak.toml')
    shutil.copytree(SAMPLE / 'content', root / 'content')
    shutil.copyfile(wasm, root / 'bin/wasm/ruby-behaviors-server.wasm')
    (game.work / 'yoake-mods.txt').write_text('ruby 0.1.0\n')


def accepted(game, line):
    """Runs a command; (accepted, its chat lines)."""
    chat = game('command', command=line).get('chat', [])
    return not any(PROBLEM in text for text in chat), chat


def lit_of(game, pos):
    """The `lit` property of the lamp at pos from the game's state string (`ruby:lamp[lit=true]`), or None."""
    state = str(game('block', x=pos[0], y=pos[1], z=pos[2]).get('state', ''))
    if not state.startswith('ruby:lamp'):
        return None
    return 'true' if 'lit=true' in state else 'false'


FACES = {
    'north': (0, 0, -1),
    'south': (0, 0, 1),
    'west': (-1, 0, 0),
    'east': (1, 0, 0),
    'up': (0, 1, 0),
    'down': (0, -1, 0),
}
AIR_BLOCKS = {'minecraft:air', 'minecraft:cave_air', 'minecraft:void_air'}


def parse_state_props(state):
    """Parse `mod:block[k1=v1,k2=v2]` into a dict of property strings."""
    if '[' not in state or not state.endswith(']'):
        return {}
    inner = state.split('[', 1)[1][:-1]
    props = {}
    for item in inner.split(','):
        if '=' in item:
            k, v = item.split('=', 1)
            props[k.strip()] = v.strip()
    return props


def is_air_block(game, pos):
    info = game('block', x=pos[0], y=pos[1], z=pos[2])
    return str(info.get('block', '')) in AIR_BLOCKS


def aim_candidates(game, pos):
    """Compute candidate `(face, stand_pos, look_target)` tuples for `pos` from its block state/facing and air around it."""
    x, y, z = pos
    state = str(game('block', x=x, y=y, z=z).get('state', ''))
    props = parse_state_props(state)
    horiz = ['north', 'south', 'west', 'east']
    face_prop = props.get('face')
    facing_prop = props.get('facing')
    if face_prop == 'wall' and facing_prop in horiz:
        order = [facing_prop] + [f for f in horiz if f != facing_prop]
    elif face_prop == 'floor':
        order = ['up'] + horiz
    elif face_prop == 'ceiling':
        order = ['down'] + horiz
    elif facing_prop in horiz:
        order = [facing_prop] + [f for f in horiz if f != facing_prop] + ['up']
    else:
        order = horiz + ['up']

    candidates = []
    for face in order:
        dx, dy, dz = FACES[face]
        if face in horiz:
            if not (is_air_block(game, (x + dx, y, z + dz)) and is_air_block(game, (x + dx, y + 1, z + dz))):
                continue
            for dist in (2, 1):
                sx, sy, sz = x + dist * dx, y, z + dist * dz
                if is_air_block(game, (sx, sy, sz)) and is_air_block(game, (sx, sy + 1, sz)):
                    candidates.append((
                        face,
                        (sx + 0.5, float(sy), sz + 0.5),
                        (x + 0.5 + dx * 0.5, y + 0.5, z + 0.5 + dz * 0.5),
                    ))
        elif face == 'up':
            if not is_air_block(game, (x, y + 1, z)):
                continue
            for hface in horiz:
                hx, _, hz = FACES[hface]
                sx, sy, sz = x + hx, y, z + hz
                if (is_air_block(game, (sx, sy, sz))
                        and is_air_block(game, (sx, sy + 1, sz))
                        and is_air_block(game, (x + hx, y + 1, z + hz))):
                    candidates.append((
                        'up',
                        (sx + 0.5, float(sy), sz + 0.5),
                        (x + 0.5, y + 1.0, z + 0.5),
                    ))
        elif face == 'down':
            if is_air_block(game, (x, y - 1, z)) and is_air_block(game, (x, y - 2, z)):
                candidates.append((
                    'down',
                    (x + 0.5, float(y - 2), z + 0.5),
                    (x + 0.5, float(y), z + 0.5),
                ))
    if not candidates:
        candidates.append(('north', (x + 0.5, float(y), z - 1.5), (x + 0.5, y + 0.5, z + 0.5)))
    return candidates


def aim_at(game, pos, stand=None, target=None):
    """Teleport to a clear stand point looking at the exposed face's centre and wait until the client position updates."""
    if stand is None or target is None:
        _, stand, target = aim_candidates(game, pos)[0]
    sx, sy, sz = stand
    tx, ty, tz = target
    game('command', command=f'tp @s {sx:.2f} {sy:.2f} {sz:.2f} facing {tx:.2f} {ty:.2f} {tz:.2f}')
    game.until(
        lambda: pos_of(game),
        lambda p: abs(p[0] - sx) < 0.25 and abs(p[1] - sy) < 0.5 and abs(p[2] - sz) < 0.25,
        ticks=80,
    )


def interact_when_ready(game, pos, tries=12):
    """Wait for the target block on the client, stand facing an exposed face, and `interact` with that exact face."""
    game.until(
        lambda: str(game('block', x=pos[0], y=pos[1], z=pos[2]).get('block', '')),
        lambda b: bool(b) and b not in AIR_BLOCKS,
        ticks=120,
    )
    candidates = aim_candidates(game, pos)
    for attempt in range(tries):
        face, stand, target = candidates[attempt % len(candidates)]
        aim_at(game, pos, stand, target)
        game.wait(4)
        try:
            return game.raw('interact', *pos, face)
        except Failed as e:
            msg = str(e)
            if ('visible' not in msg and 'reach' not in msg and 'no block' not in msg) or attempt == tries - 1:
                raise
            candidates = aim_candidates(game, pos) or candidates
        game.wait(5)


def wait_registered(game, pos, seconds=30):
    """Wait until the lamp is on the client and the WASM module's load line is flushed to the log."""
    end = time.monotonic() + seconds
    while True:
        lit = lit_of(game, pos)
        log = log_text(game)
        if lit == 'false' and ('ruby' in log or 'registering' in log):
            return lit, log
        if time.monotonic() > end:
            return lit, log
        game.wait(5)


def wait_chat(game, text, seconds=20):
    """Whether a chat line containing `text` shows within the time. `messages` lists what the game already has: the
    line a use puts in chat can come before the harness's `use-item` returns, so `wait-for chat` (which waits for
    a line from now on) would miss it."""
    end = time.monotonic() + seconds
    while True:
        if any(text in line for line in game.raw('messages', 50).get('chat', [])):
            return True
        if time.monotonic() > end:
            return False
        game.wait(5)


def wait_lit(game, pos, want, ticks=60):
    for _ in range(ticks // 5):
        if lit_of(game, pos) == want:
            return True
        game.wait(5)
    return lit_of(game, pos) == want


def wait_in_inventory(game, item, seconds=20):
    """Whether the item shows in the inventory within the time. A `give` is applied by the server and reaches the
    client's inventory a few ticks later, so a `select` right after it can find no slot yet."""
    end = time.monotonic() + seconds
    while True:
        if game('inventory').get('counts', {}).get(item, 0) >= 1:
            return True
        if time.monotonic() > end:
            return False
        game.wait(5)


def run(ctl, out, resources, wasm):
    os.environ['YOAKE_WASM_UNSIGNED'] = '1'
    game = Game('behaviors', ctl, resources)
    plain = Game('plain', ctl, resources)
    args = ['--offscreen', '--no-focus', '--no-pause']
    results = {}
    try:
        install_package(game, wasm)
        game.pid = game.raw('launch', *args)['pid']
        game.raw('open', 'Behaviors', 5, 'creative')
        game('wait_ready', seconds=400)
        # A server module's hooks run where a server runs: the LAN host's integrated server (as the other WASM e2e runs).
        game.raw('lan', 'open', 0, 'commands')
        game('command', command='time set day')
        game('command', command='gamerule spawn_mobs false')
        x, y, z = (int(v // 1) for v in pos_of(game))
        lamp = (x + 2, y, z)
        game('command', command=f'fill {x - 3} {y - 1} {z - 3} {x + 5} {y - 1} {z + 3} minecraft:stone')
        game('command', command=f'fill {x - 3} {y} {z - 3} {x + 5} {y + 4} {z + 3} minecraft:air')
        ok, said = accepted(game, f'setblock {lamp[0]} {lamp[1]} {lamp[2]} ruby:lamp[lit=false]')
        lit, log = wait_registered(game, lamp, seconds=30)
        results['registered'] = {
            'pass': ok and lit == 'false' and 'ruby' in log and 'registering' not in log,
            'detail': {
                'placed': ok,
                'said': said,
                'log_has_ruby': 'ruby' in log,
                'log_has_registering': 'registering' in log,
                'block': json.dumps(game('block', x=lamp[0], y=lamp[1], z=lamp[2]))[:200],
            },
        }
        game.wait(10)  # the server has the new blocks
        # use: the block behaviour flips `lit` and tells the player what the lamp does now. A right-click of the
        # harness can reach the block more than once (the button is held across a frame), so the check is what
        # the guest said and what the block is now, not a count: it said so at least once, each line the other
        # way from the line before, and the block is what the last line says.
        interact_when_ready(game, lamp)
        game.wait(20)
        interact_when_ready(game, lamp)
        game.wait(20)
        lines = [m for m in reversed(game.raw('messages', 50).get('chat', [])) if 'The ruby lamp' in m]
        alternates = all(a != b for a, b in zip(lines, lines[1:]))
        now = lit_of(game, lamp)
        said = None if not lines else ('true' if 'lights' in lines[-1] else 'false')
        results['use'] = {'pass': bool(lines) and alternates and now == said,
                          'detail': {'lines': lines, 'alternate': alternates, 'block_now': now}}
        # wand: the item behaviour tells the player.
        ok, _ = accepted(game, 'give @s ruby:wand')
        in_inventory = wait_in_inventory(game, 'ruby:wand')
        if in_inventory:
            game.raw('select', 'ruby:wand')
        game.wait(10)
        before = len(game.raw('messages', 50).get('chat', []))
        game.raw('use-item')
        hummed = wait_chat(game, 'The ruby wand hums.')
        results['wand'] = {'pass': ok and in_inventory and hummed,
                           'detail': {'given': ok, 'in_inventory': in_inventory, 'chat_before': before, 'hummed': hummed}}
        game.shot(out, 'end')
        # vanilla: a game without the package.
        plain.pid = plain.raw('launch', *args)['pid']
        plain.raw('open', 'Plain', 5, 'creative')
        plain('wait_ready', seconds=400)
        none_ok, _ = accepted(plain, 'setblock ~ ~2 ~ ruby:lamp')
        px, py, pz = (int(v // 1) for v in pos_of(plain))
        plain('command', command=f'fill {px - 3} {py - 1} {pz - 3} {px + 5} {py - 1} {pz + 3} minecraft:stone')
        plain('command', command=f'fill {px - 3} {py} {pz - 3} {px + 5} {py + 4} {pz + 3} minecraft:air')
        accepted(plain, f'setblock {px + 2} {py} {pz + 1} minecraft:stone')
        accepted(plain, f'setblock {px + 2} {py} {pz} minecraft:lever[face=wall,facing=north]')
        interact_when_ready(plain, (px + 2, py, pz))
        lever = json.dumps(plain('block', x=px + 2, y=py, z=pz)).lower()
        results['vanilla'] = {'pass': not none_ok and 'lever' in lever, 'detail': {'lamp_refused': not none_ok, 'lever': lever[:200]}}
    except (Failed, AssertionError, KeyError, StopIteration, IndexError, TypeError) as e:
        results.setdefault('error', {'pass': False, 'detail': repr(e)})
    finally:
        for g in (plain, game):
            text = g.close()
            (out / f'{g.name}.log').write_text(text or '')
            shutil.rmtree(g.work, ignore_errors=True)
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', default=str(ROOT / 'target/debug'), help='folder with horizon-client and horizon-ctl')
    parser.add_argument('--wasm', required=True, help='the built ruby-behaviors guest (yoake_wasm_ruby_behaviors.wasm)')
    parser.add_argument('--out', default=str(ROOT / '.local/behaviors-e2e'))
    parser.add_argument('--resources', default=os.environ.get('HORIZON_TEST_RESOURCES') or str(lab.RESOURCES),
                        help='the prepared resources (default: $HORIZON_TEST_RESOURCES, else .local/horizon-client/<version>)')
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
    results = run(bins / f'horizon-ctl{ext}', out, resources, Path(args.wasm).resolve())
    (out / 'results.json').write_text(json.dumps({'checks': results}, indent=2) + '\n')
    print(json.dumps(results, indent=1))
    sys.exit(0 if results and all(r['pass'] for r in results.values()) else 1)


if __name__ == '__main__':
    main()
