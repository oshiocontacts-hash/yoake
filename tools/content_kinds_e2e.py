#!/usr/bin/env python3
"""The registry-entry content kinds of a data-only mod (REM-16) in a real game: enchantments, damage types,
game rules, fluids and integer block states, from one fixture mod written to a scratch folder. One game,
offscreen and unfocused (`--offscreen --no-focus --no-pause`): nothing takes the mouse or the focus.

    python3 tools/content_kinds_e2e.py --bin-dir target/debug [--out .local/content-kinds-e2e]

Checks (all through the game's own commands, read back from their replies, `inventory`, `state`, `screen` and `block`):
- rule:     `/gamerule ruby:gem_rate 7` sets it, a bare `/gamerule ruby:gem_rate` reports 7, a value past the
            rule's max is refused, and vanilla's `keep_inventory` still answers.
- tab:      chat Tab completion completes `/gamerule ruby:gem_r` to `/gamerule ruby:gem_rate`.
- enchant:  `/enchant @s ruby:keen 2` on a held diamond sword puts `ruby:keen` on the stack.
- damage:   `/damage @s 1 ruby:shatter` in survival takes health (the damage type is known to the command and the
            health code).
- tag:      `ruby:shatter` is in `#ruby:shards` merged into `#minecraft:bypasses_armor`, while `ruby:crush` is not;
            on full diamond armor, `ruby:crush` is reduced and `ruby:shatter` deals full damage.
- state:    `/setblock ... ruby:crop[age=3]` places the block with its integer property at 3; `age=9` is refused.
- fluid:    `ruby:oil`, the fluid's block, places, and the game logged no refusal of the mod's fluids.
- flow:     a placed `ruby:oil` source spreads `ruby:flowing_oil` to its immediate horizontal neighbours and stops
            at distance 2 (per `viscosity: 4`, `tick_rate: 10`).
- bucket:   an empty `minecraft:bucket` used on the `ruby:oil` source picks it up into `ruby:oil_bucket`, and
            using `ruby:oil_bucket` elsewhere places `ruby:oil` and returns `minecraft:bucket`.
- reload:   updating `ruby:gem_limit` default and adding `ruby:gem_bonus` in the mod folder, then running `/reload`,
            updates `/gamerule ruby:gem_limit`, adds `/gamerule ruby:gem_bonus`, and updates tab completion.
- vanilla:  a second game without the mod knows none of the above (each of them is a refused command line).
"""
import argparse
import json
import os
import shutil
import struct
import sys
import tempfile
import time
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
from agent_control_e2e import Failed  # noqa: E402
from integration_env import require_server_reports  # noqa: E402
from lan_e2e import Game, complete, pos_of  # noqa: E402
from mob_sync_e2e import log_text, wait_for_quiet  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]

MANIFEST = '''id = "ruby"
name = "Ruby"
version = "0.1.0"
api = "0.2"
side = "both"
capabilities = ["registries"]
'''

FILES = {
    'data/ruby/horizon/game_rule/gem_rate.json': {'type': 'int', 'default': 3, 'min': 0, 'max': 10, 'category': 'misc'},
    'data/ruby/horizon/game_rule/gem_limit.json': {'type': 'int', 'default': 4, 'min': 0, 'max': 20, 'category': 'misc'},
    'data/ruby/horizon/damage_type/shatter.json': {
        'message_id': 'shatter', 'scaling': 'never', 'exhaustion': 0.1, 'effects': 'freezing'},
    'data/ruby/horizon/damage_type/crush.json': {
        'message_id': 'crush', 'scaling': 'never', 'exhaustion': 0.1, 'effects': 'hurt'},
    'data/ruby/tags/damage_type/shards.json': {'values': ['ruby:shatter']},
    'data/minecraft/tags/damage_type/bypasses_armor.json': {'replace': False, 'values': ['#ruby:shards']},
    'data/ruby/horizon/enchantment/keen.json': {
        'description': {'translate': 'enchantment.ruby.keen'},
        'supported_items': '#minecraft:enchantable/sharp_weapon',
        'weight': 5, 'max_level': 3,
        'min_cost': {'base': 5, 'per_level_above_first': 8},
        'max_cost': {'base': 25, 'per_level_above_first': 8},
        'anvil_cost': 2, 'slots': ['mainhand']},
    'data/ruby/horizon/block/crop.json': {
        'template': 'minecraft:stone', 'state_properties': {'age': {'type': 'int', 'min': 0, 'max': 7}},
        'default_state': {'age': '0'}},
    'data/ruby/horizon/block/oil.json': {'template': 'minecraft:water', 'item': False},
    'data/ruby/horizon/block/flowing_oil.json': {'template': 'minecraft:water', 'item': False},
    'data/ruby/horizon/item/oil_bucket.json': {'template': 'minecraft:water_bucket'},
    'data/ruby/horizon/fluid/oil.json': {'tint': 0x443322, 'viscosity': 4, 'tick_rate': 10},
    'assets/ruby/lang/en_us.json': {
        'enchantment.ruby.keen': 'Keen',
        'death.attack.shatter': '%1$s shattered',
        'death.attack.crush': '%1$s was crushed',
    },
}


def png():
    """A 16 by 16 grey PNG (the fluid's two textures)."""
    def chunk(kind, data):
        body = kind + data
        return struct.pack('>I', len(data)) + body + struct.pack('>I', zlib.crc32(body))
    rows = b''.join(b'\0' + b'\x80\x80\x80\xff' * 16 for _ in range(16))
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 16, 16, 8, 6, 0, 0, 0)) \
        + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b'')


def write_mod(dest):
    dest.mkdir(parents=True)
    (dest / 'horizon-mod.toml').write_text(MANIFEST)
    for name, value in FILES.items():
        path = dest / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value))
    for name in ('oil_still', 'oil_flow'):
        path = dest / f'assets/ruby/textures/block/{name}.png'
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(png())


PROBLEM = '問題箇所'  # "problem spot": the marker a refused command line carries


def cmd(game, line):
    """Runs a command; returns (accepted, its chat lines). A refused line (unknown rule, bad argument, unknown
    id) comes back with the parser's marker, in whatever language the game runs."""
    chat = game('command', command=line).get('chat', [])
    return not any(PROBLEM in text for text in chat), chat


def close_chat(game):
    for _ in range(3):
        if game('screen').get('screen') != 'chat':
            break
        game.raw('key', 'esc', 'tap')
        game.wait(3)


def run(ctl, out, resources, scratch):
    mod = scratch / 'ruby'
    write_mod(mod)
    results = {}
    game = Game('content', ctl, resources)
    plain = Game('plain', ctl, resources)
    args = ['--offscreen', '--no-focus', '--no-pause']
    try:
        game.pid = game.raw('launch', *args, '--dev-mod', str(mod))['pid']
        game.raw('open', 'Kinds', 5, 'creative')
        game('wait_ready', seconds=400)
        game('command', command='time set day')
        game('command', command='gamerule spawn_mobs false')
        # rule
        set_ok, _ = cmd(game, 'gamerule ruby:gem_rate 7')
        query_ok, query = cmd(game, 'gamerule ruby:gem_rate')
        over_ok, over = cmd(game, 'gamerule ruby:gem_rate 99')
        vanilla_ok, _ = cmd(game, 'gamerule keep_inventory')
        results['rule'] = {'pass': set_ok and query_ok and 'gem_rate' in ' '.join(query) and '7' in ' '.join(query)
                           and not over_ok and vanilla_ok,
                           'detail': {'set': set_ok, 'query': query, 'past_max_refused': not over_ok, 'vanilla': vanilla_ok}}
        # tab completion
        completed = complete(game, 'gamerule ruby:gem_r')
        close_chat(game)
        results['tab'] = {'pass': completed == '/gamerule ruby:gem_rate', 'detail': {'completed': completed}}
        # enchant
        cmd(game, 'give @s minecraft:diamond_sword')
        ok, said = cmd(game, 'enchant @s ruby:keen 2')
        inventory = json.dumps(game('inventory')).lower()
        results['enchant'] = {'pass': ok and 'ruby:keen' in inventory, 'detail': {'accepted': ok, 'said': said,
                                                                                    'stack': inventory[:400]}}
        # damage
        cmd(game, 'gamemode survival')
        ok, said = cmd(game, 'damage @s 1 ruby:shatter')
        game.wait(15)
        results['damage'] = {'pass': ok, 'detail': said}
        # tag: #ruby:shards merged into #minecraft:bypasses_armor ignores full diamond armor
        cmd(game, 'gamerule natural_health_regeneration false')
        cmd(game, 'effect give @s minecraft:instant_health 1 10')
        for slot, item in (('head', 'helmet'), ('chest', 'chestplate'), ('legs', 'leggings'), ('feet', 'boots')):
            cmd(game, f'item replace entity @s armor.{slot} with minecraft:diamond_{item}')
        game.wait(15)
        h0 = game('state')['player']['health']
        ok_crush, _ = cmd(game, 'damage @s 5 ruby:crush')
        game.wait(15)
        h1 = game('state')['player']['health']
        ok_shatter, _ = cmd(game, 'damage @s 5 ruby:shatter')
        game.wait(15)
        h2 = game('state')['player']['health']
        crush_taken = h0 - h1
        shatter_taken = h1 - h2
        results['tag'] = {
            'pass': ok_crush and ok_shatter and 0.0 < crush_taken < 4.5 and abs(shatter_taken - 5.0) < 1e-3,
            'detail': {'h0': h0, 'h1': h1, 'h2': h2, 'crush_taken': crush_taken, 'shatter_taken': shatter_taken},
        }
        cmd(game, 'gamerule natural_health_regeneration true')
        for slot in ('head', 'chest', 'legs', 'feet'):
            cmd(game, f'item replace entity @s armor.{slot} with minecraft:air')
        cmd(game, 'gamemode creative')
        # block state
        x, y, z = (int(v // 1) for v in pos_of(game))
        ok3, said3 = cmd(game, f'setblock {x + 3} {y} {z} ruby:crop[age=3]')
        block = json.dumps(game('block', x=x + 3, y=y, z=z)).lower()
        ok9, _ = cmd(game, f'setblock {x + 4} {y} {z} ruby:crop[age=9]')
        results['state'] = {'pass': ok3 and 'ruby:crop' in block and 'age' in block and '3' in block and not ok9,
                            'detail': {'age3': block[:300], 'age9_refused': not ok9, 'said': said3}}
        # fluid + flow + bucket on a flat stone platform
        fx, fy, fz = x + 4, y + 6, z + 4
        cmd(game, f'fill {fx - 3} {fy - 1} {fz - 3} {fx + 3} {fy - 1} {fz + 3} minecraft:stone')
        cmd(game, f'fill {fx - 3} {fy} {fz - 3} {fx + 3} {fy + 3} {fz + 3} minecraft:air')
        game.wait(5)
        okf, _ = cmd(game, f'setblock {fx} {fy} {fz} ruby:oil')
        oil = json.dumps(game('block', x=fx, y=fy, z=fz)).lower()
        log = log_text(game)
        results['fluid'] = {'pass': okf and 'ruby:oil' in oil and 'fluids are not registered' not in log,
                            'detail': oil[:300]}
        # flow: tick_rate=10, viscosity=4 -> spreads 1 block horizontally and stops before distance 2
        game.wait(25)
        adj = json.dumps(game('block', x=fx + 1, y=fy, z=fz)).lower()
        far = json.dumps(game('block', x=fx + 2, y=fy, z=fz)).lower()
        results['flow'] = {'pass': 'ruby:flowing_oil' in adj and 'minecraft:air' in far,
                           'detail': {'adj': adj[:200], 'far': far[:200]}}
        # bucket: pick up source with minecraft:bucket, then empty ruby:oil_bucket elsewhere
        cmd(game, f'tp @s {fx - 1.5} {fy} {fz + 2.5}')
        cmd(game, 'gamemode survival')
        cmd(game, 'clear @s')
        cmd(game, 'give @s minecraft:bucket 1')
        game.wait(10)
        game.raw('select', 'minecraft:bucket')
        game.wait(2)
        game.raw('look-at', fx + 0.5, fy + 0.5, fz + 0.5)
        game.wait(2)
        game.raw('use-item')
        game.wait(10)
        after_pick_block = json.dumps(game('block', x=fx, y=fy, z=fz)).lower()
        inv_after_pick = json.dumps(game('inventory')).lower()
        game.raw('select', 'ruby:oil_bucket')
        game.wait(2)
        game.raw('look-at-block', fx, fy - 1, fz + 2, 'up')
        game.wait(2)
        game.raw('use-item')
        game.wait(10)
        placed_block = json.dumps(game('block', x=fx, y=fy, z=fz + 2)).lower()
        inv_after_place = json.dumps(game('inventory')).lower()
        results['bucket'] = {
            'pass': (
                'ruby:oil_bucket' in inv_after_pick
                and 'ruby:oil' not in after_pick_block
                and 'ruby:oil' in placed_block
                and 'minecraft:bucket' in inv_after_place
                and 'ruby:oil_bucket' not in inv_after_place
            ),
            'detail': {
                'after_pick_block': after_pick_block[:150],
                'inv_after_pick': inv_after_pick[:250],
                'placed_block': placed_block[:150],
                'inv_after_place': inv_after_place[:250],
            },
        }
        cmd(game, 'gamemode creative')
        # reload: change a fixture rule default and add a new rule, then /reload
        before_reload_ok, before_reload = cmd(game, 'gamerule ruby:gem_limit')
        time.sleep(0.05)
        (mod / 'data/ruby/horizon/game_rule/gem_limit.json').write_text(
            json.dumps({'type': 'int', 'default': 12, 'min': 0, 'max': 20, 'category': 'misc'})
        )
        (mod / 'data/ruby/horizon/game_rule/gem_bonus.json').write_text(
            json.dumps({'type': 'int', 'default': 9, 'min': 0, 'max': 20, 'category': 'misc'})
        )
        # Run /reload through chat so unoptimized debug rebuilds past 10 s do not hit Integrated::call's timeout,
        # then wait until the server's resent command tree exposes ruby:gem_bonus in chat completion.
        game.raw('key', 'slash', 'tap')
        game.until(lambda: game('screen').get('screen'), lambda s: s == 'chat', ticks=3)
        game.raw('type', 'reload')
        game.raw('key', 'enter', 'tap')
        close_chat(game)
        reload_ok = False
        bonus_tab = ''
        for _ in range(30):
            time.sleep(1.5)
            try:
                bonus_tab = complete(game, 'gamerule ruby:gem_b')
                close_chat(game)
                reload_ok = bonus_tab == '/gamerule ruby:gem_bonus'
                if reload_ok:
                    break
            except AssertionError:
                close_chat(game)
        reload_chat = game.raw('messages', 5).get('chat', [])
        limit_ok, limit_chat = cmd(game, 'gamerule ruby:gem_limit')
        bonus_ok, bonus_chat = cmd(game, 'gamerule ruby:gem_bonus')
        reload_log = log_text(game)
        results['reload'] = {
            'pass': (
                before_reload_ok and '4' in ' '.join(before_reload)
                and reload_ok
                and limit_ok and '12' in ' '.join(limit_chat)
                and bonus_ok and '9' in ' '.join(bonus_chat)
                and bonus_tab == '/gamerule ruby:gem_bonus'
                and 'content mods not loaded' not in reload_log
                and 'need a game restart' not in reload_log
            ),
            'detail': {
                'before': before_reload,
                'reload': reload_chat,
                'limit': limit_chat,
                'bonus': bonus_chat,
                'bonus_tab': bonus_tab,
            },
        }
        game.shot(out, 'end')
        # vanilla: no mod, none of it
        plain.pid = plain.raw('launch', *args)['pid']
        plain.raw('open', 'Plain', 5, 'creative')
        plain('wait_ready', seconds=400)
        none_ok, none = cmd(plain, 'gamerule ruby:gem_rate')
        enchant_ok, _ = cmd(plain, 'enchant @s ruby:keen 1')
        damage_ok, _ = cmd(plain, 'damage @s 1 ruby:shatter')
        block_ok, _ = cmd(plain, 'setblock ~ ~2 ~ ruby:crop')
        keep_ok, _ = cmd(plain, 'gamerule keep_inventory')
        results['vanilla'] = {'pass': not (none_ok or enchant_ok or damage_ok or block_ok) and keep_ok,
                              'detail': {'rule': none_ok, 'enchant': enchant_ok, 'damage': damage_ok, 'block': block_ok}}
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
    parser.add_argument('--out', default=str(ROOT / '.local/content-kinds-e2e'))
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
    scratch = Path(tempfile.mkdtemp(prefix='hz-kinds-'))
    ext = '.exe' if sys.platform == 'win32' else ''
    try:
        results = run(bins / f'horizon-ctl{ext}', out, resources, scratch)
    finally:
        shutil.rmtree(scratch, ignore_errors=True)
    (out / 'results.json').write_text(json.dumps({'checks': results}, indent=2) + '\n')
    print(json.dumps(results, indent=1))
    sys.exit(0 if results and all(r['pass'] for r in results.values()) else 1)


if __name__ == '__main__':
    main()
