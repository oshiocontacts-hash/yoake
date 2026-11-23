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


def wait_lit(game, pos, want, ticks=60):
    for _ in range(ticks // 5):
        if lit_of(game, pos) == want:
            return True
        game.wait(5)
    return lit_of(game, pos) == want


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
        ok, said = accepted(game, f'setblock {lamp[0]} {lamp[1]} {lamp[2]} ruby:lamp[lit=false]')
        log = log_text(game)
        results['registered'] = {
            'pass': ok and lit_of(game, lamp) == 'false' and 'ruby' in log and 'registering' not in log,
            'detail': {'placed': ok, 'said': said, 'block': json.dumps(game('block', x=lamp[0], y=lamp[1], z=lamp[2]))[:200]}}
        game.wait(10)  # the server has the new blocks
        # use: the block behaviour flips `lit` and tells the player what the lamp does now. A right-click of the
        # harness can reach the block more than once (the button is held across a frame), so the check is what
        # the guest said and what the block is now, not a count: it said so at least once, each line the other
        # way from the line before, and the block is what the last line says.
        game.raw('interact', *lamp)
        game.wait(20)
        game.raw('interact', *lamp)
        game.wait(20)
        lines = [m for m in reversed(game.raw('messages', 50).get('chat', [])) if 'The ruby lamp' in m]
        alternates = all(a != b for a, b in zip(lines, lines[1:]))
        now = lit_of(game, lamp)
        said = None if not lines else ('true' if 'lights' in lines[-1] else 'false')
        results['use'] = {'pass': bool(lines) and alternates and now == said,
                          'detail': {'lines': lines, 'alternate': alternates, 'block_now': now}}
        # wand: the item behaviour tells the player.
        ok, _ = accepted(game, 'give @s ruby:wand')
        game.raw('select', 'ruby:wand')
        game.wait(10)
        before = len(game.raw('messages', 50).get('messages', []))
        game.raw('use-item')
        game.raw('wait-for', 'chat', 'The ruby wand hums.', '--timeout', 20)
        messages = json.dumps(game.raw('messages', 50))
        results['wand'] = {'pass': ok and 'The ruby wand hums.' in messages, 'detail': {'given': ok, 'messages_before': before}}
        game.shot(out, 'end')
        # vanilla: a game without the package.
        plain.pid = plain.raw('launch', *args)['pid']
        plain.raw('open', 'Plain', 5, 'creative')
        plain('wait_ready', seconds=400)
        none_ok, _ = accepted(plain, 'setblock ~ ~2 ~ ruby:lamp')
        px, py, pz = (int(v // 1) for v in pos_of(plain))
        accepted(plain, f'setblock {px + 2} {py} {pz} minecraft:lever')
        plain.raw('interact', px + 2, py, pz)
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
