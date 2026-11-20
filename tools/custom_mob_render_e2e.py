#!/usr/bin/env python3
"""Client rendering of custom mobs (F4M-6) end to end: a LAN host with the Cinder Fox fixture, a Horizon guest, real
games (`--offscreen --no-focus --no-pause`, nothing takes the focus or the mouse), offscreen screenshots of the fox.

    PYTHONUTF8=1 python tools/custom_mob_render_e2e.py --bin-dir target/debug [--out target/custom-mob-render-e2e]
    ... --shots docs/yoake2/reports/img        # where the screenshots are copied (default)

The fixture (tools/fixtures/cinder_fox) is copied to a scratch folder and patched for the run: the `pounce`
animation gets a sound (`entity.fox.bite`) and a particle (`flame` at the `mouth` locator) at its `hit` keyframe (0.35 s); the
fox gets a `melee_attack` goal with `trigger: pounce`, `damage_frame: hit`, a 600 tick interval (one strike per run) and
`attack_damage` 2, and also targets players. The repository's fixture is not touched.

Checks:
- idle:    the fox, summoned with no goals two blocks in front of the host's camera, in daylight: screenshot `f4m6-idle-day.png`.
- glow:    the same at midnight: `f4m6-idle-night.png`; the glow mask makes the picture differ in a dark scene (pixels
           changed against the daylight shot are counted; the mask itself is judged by looking at the PNG).
- pounce:  `/horizon custommobs trigger` fires `pounce`; a screenshot six ticks later (`f4m6-pounce.png`) differs from idle; the
           server logs the `hit` frame once and the host's client logs the sound, the particle and the `hit` event
           in that tick or the next one (the local mirror plays a trigger from the tick after it was asked for);
           the guest's client logs the same events within a few ticks (through `horizon:mob_sync`), once.
- lod:     the fox at 30 and 70 blocks is still drawn (screenshots `f4m6-lod-30.png`, `f4m6-lod-70.png`: pixels differ
           from an empty scene).
- name:    a fox with `CustomName` shows a name tag (`f4m6-nametag.png`).
- strike:  a second fox with AI, a survival host in reach: one strike (the server `hit` frame logged once, the host loses
           exactly 2 health) in the window; its client events come in the same tick as the server frame (or the next).
- death:   `kill`: the fox is still there after 6 ticks and gone after 30 (its death animation is 0.6 s = 12 ticks).

Not covered here: parity golden C (cargo test), the comparison of the laptop's golden vertex hashes (see FR-MERGE6.md).
"""
import argparse
import json
import os
import re
import shutil
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
from agent_control_e2e import Failed  # noqa: E402
from integration_env import require_server_reports  # noqa: E402
from lan_e2e import Game, changed_pixels, pos_of  # noqa: E402
from mob_sync_e2e import wait_for_quiet  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / 'tools/fixtures/cinder_fox'
FOX = 'sample_mobs:cinder_fox'
FOXES = '@e[type=sample_mobs:cinder_fox]'
ANIM = 'assets/sample_mobs/animations/cinder_fox.animation.json'
TYPE = 'data/sample_mobs/horizon/entity_type/cinder_fox.json'

SERVER_FRAME = re.compile(r'Horizon custom-mob server frame: mob (\d+) "(\w+)" tick (\d+)')
EVENTS = re.compile(r'Horizon custom-mob events: mob (\d+) tick (\d+) count (\d+)')
SOUND = re.compile(r'Horizon custom-mob sound: mob (\d+) (\S+) at')
PARTICLE = re.compile(r'Horizon custom-mob particle: mob (\d+) (\S+) at')
EVENT = re.compile(r'Horizon custom-mob event: mob (\d+) "(\w+)" at')


def patched_fixture(scratch):
    """A copy of the fixture with a sound and a particle at the pounce's hit, and a melee goal that pounces."""
    dest = scratch / 'cinder_fox'
    shutil.copytree(FIXTURE, dest)
    path = dest / ANIM
    data = json.loads(path.read_text(encoding='utf-8'))
    pounce = data['animations']['animation.cinder_fox.pounce']
    pounce['sound_effects'] = {'0.35': {'effect': 'entity.fox.bite'}}
    pounce['particle_effects'] = {'0.35': {'effect': 'flame', 'locator': 'mouth'}}
    path.write_text(json.dumps(data, indent=2), encoding='utf-8')
    path = dest / TYPE
    data = json.loads(path.read_text(encoding='utf-8'))
    data['attributes']['attack_damage'] = 2
    data['model']['particles'] = {'flame': 'minecraft:flame'}
    ai = data['ai']
    # No wandering or looking about: the fox stands still until it has a target (a creative player is none).
    ai['goals'] = [g for g in ai['goals'] if g['type'] in ('swim',)]
    ai['goals'].append({'type': 'melee_attack', 'priority': 2, 'speed': 1.0, 'interval': 600,
                        'trigger': 'pounce', 'damage_frame': 'hit'})
    ai.pop('script_goals', None)
    ai['targets'] = [{'type': 'nearest_target', 'priority': 1, 'types': ['minecraft:chicken', 'minecraft:player']}]
    path.write_text(json.dumps(data, indent=2), encoding='utf-8')
    return dest


def log_of(game):
    log = game.socket.with_suffix('.log')
    return log.read_text(errors='replace') if log.exists() else ''


def fox_list(game, radius=100):
    return [e for e in game.raw('entities', radius)['entities'] if e.get('type') == FOX]


def clear_foxes(game):
    """No fox is left: `kill` until the entity list holds none (the e2e needs exactly one in every phase)."""
    for _ in range(5):
        game('command', command=f'kill {FOXES}')
        game.wait(25)
        if not fox_list(game):
            return
    raise Failed('foxes are still there after five kills')


def look_at_fox(game, fox, back=2.0, dy=0.6):
    """Put the player `back` blocks south of the fox, at its height, looking at it."""
    x, y, z = fox['pos']
    game('command', command=f'tp @s {x:.2f} {y:.2f} {z + back:.2f}')
    game.wait(10)
    game.raw('look-at', f'{x:.2f}', f'{y + dy:.2f}', f'{z:.2f}')
    game.wait(6)


def run(ctl, out, shots, resources):
    scratch = Path(tempfile.mkdtemp(prefix='hzfox-'))
    mod = patched_fixture(scratch)
    host, guest = Game('host', ctl, resources), Game('guest', ctl, resources)
    args = ['--offscreen', '--no-focus', '--no-pause', '--pregen', '--dev-mod', str(mod)]  # --pregen turns Horizon's own commands on
    results = {}
    games = (host, guest)
    saved = {}

    def shot(game, label):
        path = game.shot(out, label)
        target = shots / f'f4m6-{label}.png'
        shots.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(path, target)
        saved[label] = str(target)
        return path

    try:
        host.pid = host.raw('launch', *args)['pid']
        host.raw('open', 'Lan', 5, 'creative')
        host('wait_ready', seconds=400)
        host('command', command='gamemode creative')
        host('command', command='time set day')
        host('command', command='difficulty normal')
        host('command', command='gamerule spawn_mobs false')
        host('command', command='fill ~-40 ~-1 ~-40 ~40 ~-1 ~40 minecraft:stone')
        host('command', command='fill ~-40 ~ ~-40 ~40 ~12 ~40 minecraft:air')
        port = host.raw('lan', 'open', 0, 'commands')['port']
        guest.pid = guest.raw('launch', *args)['pid']
        guest.raw('connect', f'127.0.0.1:{port}', 'Guest')
        guest.raw('wait-for', 'loaded', '--timeout', 300)
        guest.wait(40)
        hx, hy, hz = (int(v // 1) for v in pos_of(host))
        guest('command', command=f'tp @s {hx + 3.5} {hy} {hz + 3.5}')
        host('command', command='gamerule spawn_mobs false')

        host('command', command=f'fill {hx - 4} {hy - 1} {hz - 10} {hx + 4} {hy - 1} {hz + 4} minecraft:stone')
        host('command', command=f'fill {hx - 4} {hy} {hz - 10} {hx + 4} {hy + 5} {hz + 4} minecraft:air')
        host.wait(5)
        # idle: a fox that stands still (no goals but swimming), two blocks from the camera.
        host('command', command=f'summon {FOX} {hx + 0.5} {hy} {hz - 6.5}')
        host.wait(20)
        fox = fox_list(host)[0]
        look_at_fox(host, fox)
        day = shot(host, 'idle-day')
        host('command', command='time set midnight')
        host.wait(10)
        night = shot(host, 'idle-night')
        host('command', command='time set day')
        host.wait(10)
        results['idle'] = {'pass': Path(day).stat().st_size > 5000 and len(fox_list(host)) == 1,
                           'detail': {'png': saved['idle-day'], 'fox': fox['pos']}}
        results['glow'] = {'pass': changed_pixels(day, night) > 500, 'detail': {
            'changed_vs_day': changed_pixels(day, night), 'png': saved['idle-night']}}

        # pounce: the trigger fires the animation; its hit frame plays the sound and the particle.
        resummoned = 0
        if not fox_list(host):
            resummoned = 1
            host('command', command=f'summon {FOX} {fox["pos"][0]:.1f} {fox["pos"][1]:.1f} {fox["pos"][2]:.1f}')
            host.wait(20)
        mark_h, mark_g = len(log_of(host)), len(log_of(guest))
        host('command', command=f'horizon custommobs trigger {FOXES} pounce')
        host.wait(6)
        mid = shot(host, 'pounce')
        host.wait(30)
        server = [m for m in SERVER_FRAME.finditer(log_of(host)[mark_h:])]
        h_text, g_text = log_of(host)[mark_h:], log_of(guest)[mark_g:]
        def tick_of(text, pattern_event):
            ticks = [int(m.group(2)) for m in EVENTS.finditer(text)]
            return ticks
        host_sounds = len(SOUND.findall(h_text)); host_particles = len(PARTICLE.findall(h_text)); host_hits = len(EVENT.findall(h_text))
        guest_sounds = len(SOUND.findall(g_text)); guest_particles = len(PARTICLE.findall(g_text)); guest_hits = len(EVENT.findall(g_text))
        server_tick = int(server[0].group(3)) if server else None
        host_ticks, guest_ticks = tick_of(h_text, None), tick_of(g_text, None)
        results['pounce'] = {'resummoned': resummoned, 'pass': (len(server) == 1 and (host_sounds, host_particles, host_hits) == (1, 1, 1)
                                      and (guest_sounds, guest_particles, guest_hits) == (1, 1, 1)
                                      and changed_pixels(day, mid) > 300
                                      and host_ticks and server_tick is not None and 0 <= host_ticks[0] - server_tick <= 1),
                             'detail': {'server_frames': len(server), 'server_tick': server_tick, 'host_event_ticks': host_ticks,
                                        'guest_event_ticks': guest_ticks,
                                        'host': [host_sounds, host_particles, host_hits], 'guest': [guest_sounds, guest_particles, guest_hits],
                                        'changed_vs_idle': changed_pixels(day, mid), 'png': saved['pounce']}}

        # lod: far away the fox is still drawn.
        far = {}
        empty = None
        for dist in (30, 70):
            x, y, z = fox['pos']
            host('command', command=f'tp @s {x:.2f} {y:.2f} {z + dist:.2f}')
            host.wait(20)
            host.raw('look-at', f'{x:.2f}', f'{y + 0.4:.2f}', f'{z:.2f}')
            host.wait(10)
            far[dist] = shot(host, f'lod-{dist}')
        clear_foxes(host)
        empty = host.shot(out, 'lod-empty')
        far_changes = {d: changed_pixels(p, empty) for d, p in far.items()}
        results['lod'] = {'pass': all(c > 15 for c in far_changes.values()), 'detail': {
            'changed_vs_empty_scene': far_changes, 'png': [saved['lod-30'], saved['lod-70']]}}

        # name tag
        host('command', command=f'summon {FOX} {hx + 0.5} {hy} {hz - 6.5}')
        host.wait(20)
        named = host('command', command=f"data merge entity @e[type=sample_mobs:cinder_fox,limit=1] {{CustomName:\"Cinder\",CustomNameVisible:1b}}")
        host.wait(10)
        fox = fox_list(host)[0]
        look_at_fox(host, fox, back=3.0, dy=0.9)
        tag = shot(host, 'nametag')
        clear_foxes(host)
        named_ok = not any('←' in line for line in named.get('chat', []))  # the parser marks the error position with an arrow
        results['name'] = {'pass': named_ok and Path(tag).stat().st_size > 5000, 'detail': {
            'data_merge_reply': named, 'png': saved['nametag'], 'note': 'the tag is judged by looking at the PNG (a pixel check cannot tell text)'}}

        # strike-mob: a fox next to a chicken (creative host: no player target): the blow lands on the chicken.
        host('command', command=f'kill @e[type=minecraft:chicken]')
        host('command', command=f'summon minecraft:chicken {hx + 1.5} {hy} {hz - 1.5}')
        host.wait(5)
        mark_h = len(log_of(host))
        host('command', command=f'summon {FOX} {hx + 1.5} {hy} {hz - 0.8}')
        chicken_hp = []
        for _ in range(100):
            host.wait(2)
            chickens = [e for e in host.raw('entities', 20)['entities'] if e.get('type') == 'minecraft:chicken']
            chicken_hp.append(chickens[0]['health'] if chickens else None)
            if SERVER_FRAME.search(log_of(host)[mark_h:]):
                break
        host.wait(10)
        chickens = [e for e in host.raw('entities', 20)['entities'] if e.get('type') == 'minecraft:chicken']
        chicken_hp.append(chickens[0]['health'] if chickens else None)
        results['strike_mob'] = {'pass': len(SERVER_FRAME.findall(log_of(host)[mark_h:])) == 1 and (chicken_hp[-1] is None or chicken_hp[-1] < 4.0),
                                 'detail': {'chicken_health_samples': chicken_hp[-6:]}}
        host('command', command='kill @e[type=minecraft:chicken]')
        clear_foxes(host)
        # strike: a fox with AI, the host in survival within reach: one hit.
        host('command', command='gamemode survival')
        host('command', command=f'tp @s {hx + 0.5} {hy} {hz + 0.5}')
        host.wait(20)
        before = host('state')['player']['health']
        mark_h, mark_g = len(log_of(host)), len(log_of(guest))
        host('command', command=f'summon {FOX} {hx + 1.5} {hy} {hz + 0.5}')
        health = [before]
        gap = []
        for _ in range(160):
            host.wait(2)
            me = host('state')['player']
            health.append(me['health'])
            foxes = fox_list(host)
            if foxes:
                gap.append(round(((foxes[0]['pos'][0] - me['position'][0]) ** 2 + (foxes[0]['pos'][2] - me['position'][2]) ** 2) ** 0.5, 2))
            if SERVER_FRAME.search(log_of(host)[mark_h:]):
                break
        for _ in range(10):
            host.wait(2)
            health.append(host('state')['player']['health'])
        after = min(health)
        h_text, g_text = log_of(host)[mark_h:], log_of(guest)[mark_g:]
        server = list(SERVER_FRAME.finditer(h_text))
        st = int(server[0].group(3)) if server else None
        ht = [int(m.group(2)) for m in EVENTS.finditer(h_text)]
        gt = [int(m.group(2)) for m in EVENTS.finditer(g_text)]
        results['strike'] = {'pass': (len(server) == 1 and before - after == 2.0 and ht and st is not None
                                      and 0 <= ht[0] - st <= 1),
                             'detail': {'server_frames': len(server), 'server_tick': st, 'health': [before, after], 'health_samples': health[-14:], 'fox_distance': gap[-6:],
                                        'host_event_ticks': ht, 'guest_event_ticks': gt}}
        host('command', command='gamemode creative')

        clear_foxes(host)
        # death: the death animation (0.6 s = 12 ticks) keeps the body, then it is removed.
        host('command', command=f'summon {FOX} {hx + 0.5} {hy} {hz - 6.5}')
        host.wait(20)
        fox = fox_list(host)
        look_at_fox(host, fox[0])
        host('command', command=f'damage @e[type=sample_mobs:cinder_fox,limit=1] 100 minecraft:generic')
        presence = []
        shot_taken = False
        for i in range(1, 16):
            host.wait(2)
            presence.append(len(fox_list(host)))
            if i == 2 and not shot_taken:
                shot(host, 'death'); shot_taken = True
        results['death'] = {'pass': presence[0] == 1 and presence[-1] == 0,
                            'detail': {'foxes_every_2_ticks_after_damage': presence, 'png': saved.get('death')}}
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
    results['_screenshots'] = {'pass': True, 'detail': saved}
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', default=str(ROOT / 'target/debug'))
    parser.add_argument('--out', default=str(ROOT / 'target/custom-mob-render-e2e'))
    parser.add_argument('--shots', default=str(ROOT / 'docs/yoake2/reports/img'))
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
    results = run(Path(args.bin_dir).resolve() / f'horizon-ctl{ext}', out, Path(args.shots).resolve(), resources)
    (out / 'results.json').write_text(json.dumps({'checks': results}, indent=2) + '\n')
    print(json.dumps(results, indent=1))
    sys.exit(0 if all(r['pass'] for r in results.values()) else 1)


if __name__ == '__main__':
    main()
