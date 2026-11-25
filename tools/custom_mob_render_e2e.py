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
- reload (F4M-13, scenario 7, without --wasm): the ear of the running fox is moved in the geo file of the dev folder: within
           about two seconds both games log the reload and the picture changes; a truncated geo keeps the old fox and logs
           one warning; the restored file and `/yoake mob reload` bring the first picture back.
- debug (F4M-12, without --wasm): `/yoake mobdebug on` changes the picture (boxes, bones, texts), `/yoake mobs` lists the
           fox, `/yoake mobdebug off` brings the picture back.

Not covered here: parity golden C (cargo test), the comparison of the laptop's golden vertex hashes (see FR-MERGE6.md).
"""
import argparse
import json
import math
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
FIXTURE = ROOT / 'tools/fixtures/cinder_fox'  # the package folder: yak.toml and content/
FOX = 'sample_mobs:cinder_fox'
FOXES = '@e[type=sample_mobs:cinder_fox]'
ANIM = 'content/assets/sample_mobs/animations/cinder_fox.animation.json'
TYPE = 'content/data/sample_mobs/horizon/entity_type/cinder_fox.json'

SERVER_FRAME = re.compile(r'Horizon custom-mob server frame: mob (\d+) "(\w+)" tick (\d+)')
BRAIN_SWITCH = re.compile(r'Horizon custom-mob brain: mob (\d+) activity (.*?) -> (.*?) tick (\d+)')
EVENTS = re.compile(r'Horizon custom-mob events: mob (\d+) tick (\d+) count (\d+)')
SOUND = re.compile(r'Horizon custom-mob sound: mob (\d+) (\S+) at')
PARTICLE = re.compile(r'Horizon custom-mob particle: mob (\d+) (\S+) at')
EVENT = re.compile(r'Horizon custom-mob event: mob (\d+) "(\w+)" at')
RELOADED = re.compile(r'Horizon custom-mob reload: sample_mobs:cinder_fox reloaded \(')
RELOADED_BY_COMMAND = re.compile(r'Horizon custom-mob reload: sample_mobs:cinder_fox reloaded by command')
NOT_RELOADED = re.compile(r'Horizon custom-mob reload: sample_mobs:cinder_fox not reloaded, the old version keeps running')
SWAPPED = re.compile(r'Horizon custom-mob reload: sample_mobs:cinder_fox: (\d+) mob\(s\) now use the new version')
GEO = 'content/assets/sample_mobs/geo/cinder_fox.geo.json'
FOX_TEXTURE_LINE = re.compile(r'Horizon custom-mob texture: mob (\d+) slot "(\w*)" texture Some\((\d+)\)')


def patched_fixture(scratch, wasm=None, client_wasm=None):
    """A copy of the package with a sound and a particle at the pounce's hit. With `wasm` (the built fox_ai guest) the
    real script goal stays and the player becomes a target; without it the script goal is replaced by a data
    `melee_attack` that pounces. With `client_wasm` (the built fox_texture guest) the package also loads the
    client-full texture module."""
    dest = scratch / 'cinder_fox'
    shutil.copytree(FIXTURE, dest)
    if wasm is not None:
        (dest / 'bin/wasm').mkdir(parents=True, exist_ok=True)
        shutil.copyfile(wasm, dest / 'bin/wasm/fox_ai.wasm')
    if client_wasm is not None:
        (dest / 'bin/wasm').mkdir(parents=True, exist_ok=True)
        shutil.copyfile(client_wasm, dest / 'bin/wasm/fox_texture.wasm')
        yak_path = dest / 'yak.toml'
        yak_text = yak_path.read_text(encoding='utf-8')
        yak_text = yak_text.replace(
            'interface = "1.0"\n',
            'interface = "1.0"\nworld = "client-full"\nclient = "bin/wasm/fox_texture.wasm"\n',
            1,
        )
        yak_path.write_text(yak_text, encoding='utf-8')
    path = dest / ANIM
    data = json.loads(path.read_text(encoding='utf-8'))
    pounce = data['animations']['animation.cinder_fox.pounce']
    pounce['sound_effects'] = {'0.35': {'effect': 'entity.fox.bite'}}
    pounce['particle_effects'] = {'0.35': {'effect': 'flame', 'locator': 'mouth'}}
    path.write_text(json.dumps(data, indent=2), encoding='utf-8')
    path = dest / TYPE
    data = json.loads(path.read_text(encoding='utf-8'))
    data['attributes']['attack_damage'] = 2
    # One strike per run: the 600-tick interval must outlast the chase goal's restarts (vanilla clears it on restart).
    data['cooldown_persists'] = True
    data['model']['particles'] = {'flame': 'minecraft:flame'}
    ai = data['ai']
    if wasm is not None:
        # The real script goal beside swimming only (no wandering, sitting or following), so the fox stands until it has a target.
        ai['goals'] = [g for g in ai['goals'] if g['type'] in ('swim', 'script')]
        ai['targets'] = [{'type': 'nearest_target', 'priority': 1, 'types': ['minecraft:chicken', 'minecraft:player']}]
        path.write_text(json.dumps(data, indent=2), encoding='utf-8')
        return dest
    # No wandering or looking about: the fox stands still until it has a target (a creative player is none).
    ai['goals'] = [g for g in ai['goals'] if g['type'] in ('swim',)]
    ai['goals'].append({'type': 'melee_attack', 'priority': 2, 'speed': 1.0, 'interval': 600,
                        'trigger': 'pounce', 'damage_frame': 'hit'})
    ai.pop('script_goals', None)
    ai['targets'] = [{'type': 'nearest_target', 'priority': 1, 'types': ['minecraft:chicken', 'minecraft:player']}]
    path.write_text(json.dumps(data, indent=2), encoding='utf-8')
    return dest


def install_package(game, package):
    """The package where the game reads installed ones (yoake-mods.txt, yoake-mods/<id>/<version>/)."""
    root = game.work / 'yoake-mods/sample_mobs/0.1.0'
    shutil.copytree(package, root)
    (game.work / 'yoake-mods.txt').write_text('sample_mobs 0.1.0' + chr(10))


STRIKE_LINE = re.compile(r'^.*(?:Horizon custom-mob (?:strike|death|script goal|target|goal|texture):).*$', re.M)



def prey_chickens(host, x, z, within=4.0):
    """Chickens near (x, z), nearest first: chickens spawned with new chunks are not the prey."""
    found = []
    for e in host.raw('entities', 20)['entities']:
        if e.get('type') == 'minecraft:chicken' and 'pos' in e:
            d = math.dist((e['pos'][0], e['pos'][2]), (x, z))
            if d <= within:
                found.append((d, e))
    return [e for _, e in sorted(found, key=lambda p: p[0])]

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


def move_ear(path, dy):
    """The left ear `dy` model pixels up: its pivot and its cubes (a visible change of the geo)."""
    data = json.loads(path.read_text(encoding='utf-8'))
    for bone in data['minecraft:geometry'][0]['bones']:
        if bone['name'] == 'ear_l':
            bone['pivot'][1] += dy
            for cube in bone.get('cubes', []):
                cube['origin'][1] += dy
    path.write_text(json.dumps(data, indent=2), encoding='utf-8')


def typed(game, line):
    """A line typed in the chat box (`/yoake ...` is answered by the window's own game; the control `command` runs on the
    server side): the newest chat lines as one text."""
    game.raw('chat', '/' + line)
    game.wait(2)
    return ' '.join(game.raw('chat-log', 6).get('chat', []))


EAR_BOX = (600, 40, 1100, 360)  # above the head, where the ear floats once it is half a block up (clear of the toasts at the right)


def box_changed(a, b, box=EAR_BOX):
    """Pixels that differ between two screenshots inside `box`."""
    from PIL import Image, ImageChops
    diff = ImageChops.difference(Image.open(a).convert('RGB').crop(box), Image.open(b).convert('RGB').crop(box))
    return diff.convert('L').point(lambda v: 255 if v > 24 else 0).histogram()[255]


def debug_pixels(path):
    """Pixels of the debug view's own colours (hitbox green, bone yellow, locator cyan) above the chat lines: the fox's
    fur has none of them."""
    from PIL import Image
    im = Image.open(path).convert('RGB').crop((0, 0, 1600, 570))
    colours = [(63, 255, 0), (255, 255, 0), (0, 255, 255)]
    return sum(1 for px in im.getdata() if any(all(abs(px[i] - c[i]) <= 16 for i in range(3)) for c in colours))


def reload_and_debug(host, guest, shot, mod, hx, hy, hz):
    """F4M-13 scenario 7 and the F4M-12 debug view on the running fox; returns the two results."""
    clear_foxes(host)
    host('command', command='kill @e[type=minecraft:item]')  # the drops bob about and would blur the pixel counts
    # NoAI: a fox that walks or turns between the shots blurs the before/restored count (seen: 897 px).
    host('command', command=f'summon {FOX} {hx + 0.5} {hy} {hz - 6.5} {{NoAI:1b}}')
    host.wait(20)
    look_at_fox(host, fox_list(host)[0], back=2.0, dy=0.6)
    geo = mod / GEO
    original = geo.read_text(encoding='utf-8')
    first = str(shot(host, 'reload-before'))
    mark_h, mark_g = len(log_of(host)), len(log_of(guest))
    move_ear(geo, 12)  # three quarters of a block up: clear of the head
    host.wait(90)  # the watcher polls once a second and wants two equal looks
    moved = str(shot(host, 'reload-ear'))
    h_moved, g_moved = log_of(host)[mark_h:], log_of(guest)[mark_g:]
    ear_ok = (len(RELOADED.findall(h_moved)) >= 1 and len(RELOADED.findall(g_moved)) >= 1
              and any(int(n) >= 1 for n in SWAPPED.findall(h_moved)) and any(int(n) >= 1 for n in SWAPPED.findall(g_moved)))
    # a malformed edit: the old model keeps running and one warning says so.
    mark_h, mark_g = len(log_of(host)), len(log_of(guest))
    geo.write_text(original[:len(original) // 2], encoding='utf-8')
    host.wait(90)
    broken = str(shot(host, 'reload-broken'))
    h_broken, g_broken = log_of(host)[mark_h:], log_of(guest)[mark_g:]
    broken_ok = (len(NOT_RELOADED.findall(h_broken)) == 1 and len(NOT_RELOADED.findall(g_broken)) == 1
                 and len(fox_list(host)) == 1)
    # the file is right again; the command reads it at once.
    mark_h = len(log_of(host))
    geo.write_text(original, encoding='utf-8')
    reply_text = typed(host, 'yoake mob reload')
    host.wait(15)
    restored = str(shot(host, 'reload-restored'))
    command_ok = len(RELOADED_BY_COMMAND.findall(log_of(host)[mark_h:])) == 1 and 'Reloaded sample_mobs:cinder_fox' in reply_text
    moved_px, broken_px, restored_px = box_changed(first, moved), box_changed(moved, broken), box_changed(first, restored)
    reload_result = {
        'pass': (ear_ok and broken_ok and command_ok and moved_px > 400 and broken_px < moved_px / 3 and restored_px < moved_px / 3),
        'detail': {'ear_logged': ear_ok, 'broken_warned_once': broken_ok, 'command_ok': command_ok, 'command_reply': reply_text,
                   'ear_box_changed_before_to_ear': moved_px, 'ear_box_changed_ear_to_broken': broken_px, 'ear_box_changed_before_to_restored': restored_px,
                   'host_lines': RELOADED.findall(h_moved) and h_moved.count('custom-mob reload'),
                   'png': [first, moved, broken, restored]},
    }
    # the debug view on the same fox.
    on = typed(host, 'yoake mobdebug on')
    host.wait(10)
    debug_on = str(shot(host, 'debug-on'))
    host.wait(40)
    first_mobs = typed(host, 'yoake mobs')
    host.wait(60)
    mobs = typed(host, 'yoake mobs')
    typed(host, 'yoake mobs stop')
    off = typed(host, 'yoake mobdebug off')
    host.wait(10)
    debug_off = str(shot(host, 'debug-off'))
    text = str
    shown, back = debug_pixels(debug_on), debug_pixels(debug_off)
    # the key chord F3+M is the same toggle.
    host.script(['key f3 down', 'wait 2', 'key m tap', 'wait 2', 'key f3 up', 'wait 4'])
    host.wait(6)
    chord_on = str(shot(host, 'debug-chord'))
    host.script(['key f3 down', 'wait 2', 'key m tap', 'wait 2', 'key f3 up', 'wait 4'])
    host.wait(6)
    chord_off = str(shot(host, 'debug-chord-off'))
    chord_shown, chord_back = debug_pixels(chord_on), debug_pixels(chord_off)
    debug_result = {
        'pass': ('on: hitboxes, bones, anim, ai' in text(on) and 'sample_mobs:cinder_fox: 1 live' in text(mobs)
                 and 'view off' in text(off) and shown > 100 and back < shown / 10
                 and chord_shown > 100 and chord_back < chord_shown / 10),
        'detail': {'on': text(on), 'mobs_first': text(first_mobs), 'mobs': text(mobs), 'off': text(off),
                   'debug_pixels_with_view': shown, 'debug_pixels_after_off': back,
                   'debug_pixels_chord': chord_shown, 'debug_pixels_chord_off': chord_back,
                   'png': [debug_on, debug_off, chord_on, chord_off]},
    }
    clear_foxes(host)
    return reload_result, debug_result


def look_at_fox(game, fox, back=2.0, dy=0.6):
    """Put the player `back` blocks south of the fox, at its height, looking at it."""
    x, y, z = fox['pos']
    game('command', command=f'tp @s {x:.2f} {y:.2f} {z + back:.2f}')
    game.wait(10)
    game.raw('look-at', f'{x:.2f}', f'{y + dy:.2f}', f'{z:.2f}')
    game.wait(6)


def run(ctl, out, shots, resources, wasm=None, client_wasm=None):
    scratch = Path(tempfile.mkdtemp(prefix='hzfox-'))
    mod = patched_fixture(scratch, wasm, client_wasm)
    os.environ['YOAKE_WASM_UNSIGNED'] = '1'  # developer mode: the package is not signed (and its folder reloads, F4M-13)
    host, guest = Game('host', ctl, resources), Game('guest', ctl, resources)
    args = ['--offscreen', '--no-focus', '--no-pause', '--pregen']  # --pregen turns Horizon's own commands on
    if wasm is None:
        args += ['--dev-mod', str(mod / 'content')]
    else:
        install_package(host, mod)
        install_package(guest, mod)
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
        named = host('command', command=f"data merge entity @e[type=sample_mobs:cinder_fox,limit=1] {{CustomName:\"Foxy\",CustomNameVisible:1b}}")
        host.wait(10)
        fox = fox_list(host)[0]
        look_at_fox(host, fox, back=3.0, dy=0.9)
        tag = shot(host, 'nametag')
        clear_foxes(host)
        named_ok = not any('←' in line for line in named.get('chat', []))  # the parser marks the error position with an arrow
        results['name'] = {'pass': named_ok and Path(tag).stat().st_size > 5000, 'detail': {
            'data_merge_reply': named, 'png': saved['nametag'], 'note': 'the tag is judged by looking at the PNG (a pixel check cannot tell text)'}}

        # brain: nearest_living sensor detects a chicken -> `prey_nearby` -> activity switches idle -> hunt -> idle.
        host('command', command='kill @e[type=minecraft:chicken]')
        host('command', command=f'summon {FOX} {hx + 0.5} {hy} {hz - 6.5}')
        host.wait(15)
        mark_b = len(log_of(host))
        host('command', command=f'summon minecraft:chicken {hx + 3.5} {hy} {hz - 6.5}')
        host.wait(15)
        host('command', command='kill @e[type=minecraft:chicken]')
        host.wait(15)
        b_switches = [(m.group(2), m.group(3)) for m in BRAIN_SWITCH.finditer(log_of(host)[mark_b:])]
        clear_foxes(host)
        results['brain'] = {
            'pass': ('Some("idle")', 'Some("hunt")') in b_switches and ('Some("hunt")', 'Some("idle")') in b_switches,
            'detail': {'switches': b_switches},
        }

        if wasm is None:
            # strike-mob: a fox next to a chicken (creative host: no player target): the blow lands on the chicken.
            host('command', command=f'kill @e[type=minecraft:chicken]')
            host('command', command=f'summon minecraft:chicken {hx + 1.5} {hy} {hz - 1.5}')
            host.wait(5)
            mark_h = len(log_of(host))
            host('command', command=f'summon {FOX} {hx + 1.5} {hy} {hz - 0.8}')
            chicken_hp = []
            for _ in range(100):
                host.wait(2)
                chickens = prey_chickens(host, hx + 1.5, hz - 1.5)
                chicken_hp.append(chickens[0]['health'] if chickens else None)
                if SERVER_FRAME.search(log_of(host)[mark_h:]):
                    break
            host.wait(10)
            chickens = prey_chickens(host, hx + 1.5, hz - 1.5)
            chicken_hp.append(chickens[0]['health'] if chickens else None)
            results['strike_mob'] = {'pass': len(SERVER_FRAME.findall(log_of(host)[mark_h:])) == 1 and (chicken_hp[-1] is None or chicken_hp[-1] < 4.0),
                                     'detail': {'chicken_health_samples': chicken_hp[-6:]}}
            host('command', command='kill @e[type=minecraft:chicken]')
            clear_foxes(host)
        if wasm is None:
            results['reload'], results['debug'] = reload_and_debug(host, guest, shot, mod, hx, hy, hz)
        # strike: a fox with AI and survival players in reach. Data goal: the fox next to the host; real guest: the
        # script goal needs the target 3 to 6 blocks away. Both players' health is recorded (the fox may pick the guest).
        host('command', command='gamemode survival')
        guest('command', command='gamemode survival')
        # No regeneration: saturated food heals 5/6 of a point every 10 ticks, and the lowest health the loop samples
        # (it can miss the trough on a loaded machine) would read as a smaller blow than the fox's fixed 3.0.
        host('command', command='gamerule natural_health_regeneration false')
        host('command', command=f'tp @s {hx + 0.5} {hy} {hz + 0.5}')
        guest('command', command=f'tp @s {hx + 3.5} {hy} {hz + 3.5}')
        host.wait(20)
        before = {'host': host('state')['player']['health'], 'guest': guest('state')['player']['health']}
        mark_h, mark_g = len(log_of(host)), len(log_of(guest))
        fx, fz = (hx + 0.5, hz + 4.5) if wasm is not None else (hx + 1.5, hz + 0.5)
        host('command', command=f'summon {FOX} {fx} {hy} {fz}')
        health = {'host': [before['host']], 'guest': [before['guest']]}
        landed = re.compile(r'custom-mob strike: mob \d+ (?:hit|dropped)')
        for _ in range(200):
            host.wait(2)
            for name, game in (('host', host), ('guest', guest)):
                health[name].append(game('state')['player']['health'])
            if landed.search(log_of(host)[mark_h:]):
                break
        for _ in range(10):
            host.wait(2)
            for name, game in (('host', host), ('guest', guest)):
                health[name].append(game('state')['player']['health'])
        # The first blow's trough: the first sample below the starting health (a later blow, with regeneration off,
        # would lower it again).
        after = {k: next((h for h in v if h < v[0]), v[0]) for k, v in health.items()}
        h_text, g_text = log_of(host)[mark_h:], log_of(guest)[mark_g:]
        server = list(SERVER_FRAME.finditer(h_text))
        st = int(server[0].group(3)) if server else None
        ht = [int(m.group(2)) for m in EVENTS.finditer(h_text)]
        gt = [int(m.group(2)) for m in EVENTS.finditer(g_text)]
        # fox_ai hardcodes 3.0 damage (mobs::attack); the non-wasm melee_attack fixture sets attack_damage = 2.
        expected_damage = 3.0 if wasm is not None else 2.0
        hurt = sorted(k for k in before if after[k] < before[k])
        results['strike'] = {'pass': (len(server) >= 1 and len(hurt) == 1 and before[hurt[0]] - after[hurt[0]] == expected_damage and ht and st is not None
                                      and 0 <= ht[0] - st <= 1),
                             'detail': {'server_frames': len(server), 'server_tick': st, 'health_before_after': {k: [before[k], after[k]] for k in before},
                                        'host_event_ticks': ht, 'guest_event_ticks': gt,
                                        'strike_lines': STRIKE_LINE.findall(h_text)[:30]}}
        host('command', command='gamemode creative')
        guest('command', command='gamemode creative')
        host('command', command='gamerule natural_health_regeneration true')

        clear_foxes(host)
        # runtime_texture (F4M-7, with client_wasm) + death: summon a fresh fox in daylight.
        host('command', command=f'summon {FOX} {hx + 0.5} {hy} {hz - 6.5}')
        host.wait(20)
        fox = fox_list(host)
        look_at_fox(host, fox[0])
        if client_wasm is not None:
            tex_before = shot(host, 'tex-before')
            mark_th, mark_tg = len(log_of(host)), len(log_of(guest))
            for _ in range(3):
                near = fox_list(host)[0]  # a blow knocks the fox back: step up to it again
                look_at_fox(host, near, back=1.5)
                host.raw('attack', near['id'])
                host.wait(14)
            host.wait(10)
            tex_after = shot(host, 'tex-after')
            for _ in range(4):
                near = fox_list(host)[0]  # a blow knocks the fox back: step up to it again
                look_at_fox(host, near, back=1.5)
                host.raw('attack', near['id'])
                host.wait(14)
            host.wait(10)
            tex_after2 = shot(host, 'tex-after2')
            h_tex = [(m.group(2), int(m.group(3))) for m in FOX_TEXTURE_LINE.finditer(log_of(host)[mark_th:])]
            g_tex = [(m.group(2), int(m.group(3))) for m in FOX_TEXTURE_LINE.finditer(log_of(guest)[mark_tg:])]
            diff1 = changed_pixels(tex_before, tex_after)
            diff2 = changed_pixels(tex_after, tex_after2)
            results['runtime_texture'] = {
                'pass': (
                    len(h_tex) >= 2 and len(g_tex) >= 2
                    and {slot for slot, _ in h_tex} == {'default'}
                    and {slot for slot, _ in g_tex} == {'default'}
                    and diff1 > 300 and diff2 > 50
                ),
                'detail': {
                    'host_bindings': h_tex,
                    'guest_bindings': g_tex,
                    'changed_before_to_w3': diff1,
                    'changed_w3_to_w8': diff2,
                    'png': [saved['tex-before'], saved['tex-after'], saved['tex-after2']],
                },
            }
        # model commands (REM-17): the client guest draws a gold cube and an apple over the fox in
        # `mob-layer`; the game logs the first of each kind it applies (cube, item).
        if client_wasm is not None:
            applied = {}
            for game in (host, guest):
                for kind in ('cube', 'item'):
                    if f'wasm: model command {kind} applied' in log_of(game):
                        applied[kind] = applied.get(kind, 0) + 1
            near = fox_list(host)[0]
            look_at_fox(host, near, back=3.0, dy=1.2)
            host.wait(6)
            shot(host, 'model-commands')
            results['model_commands'] = {
                'pass': applied.get('cube', 0) >= 1 and applied.get('item', 0) >= 1,
                'detail': {'applied_in_logs': applied, 'png': saved.get('model-commands')},
            }
        # death: the death animation (0.6 s = 12 ticks) keeps the body, then it is removed.
        mark_death = len(log_of(host))
        host('command', command=f'damage @e[type=sample_mobs:cinder_fox,limit=1] 100 minecraft:generic')
        presence = []
        shot_taken = False
        for i in range(1, 16):
            host.wait(2)
            presence.append(len(fox_list(host)))
            if i == 2 and not shot_taken:
                shot(host, 'death'); shot_taken = True
        results['death'] = {'pass': presence[0] == 1 and presence[-1] == 0,
                            'detail': {'foxes_every_2_ticks_after_damage': presence, 'png': saved.get('death'),
                                       'death_lines': STRIKE_LINE.findall(log_of(host)[mark_death:])}}
        for game in games:
            game.shot(out, 'end')
    except (Failed, AssertionError, KeyError, StopIteration, IndexError, TypeError) as e:
        results.setdefault('error', {'pass': False, 'detail': repr(e)})
    finally:
        logs = {g.name: g.close() for g in reversed(games)}
        lines = [f'[{n}] {l}' for n, t in logs.items() for l in STRIKE_LINE.findall(t)]
        (out / 'custom-mob-log-lines.txt').write_text(chr(10).join(lines) + chr(10), encoding='utf-8')
        saved['log_lines'] = str(out / 'custom-mob-log-lines.txt')
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
    parser.add_argument('--wasm', help='the built fox_ai guest: the real script goal runs (default: a data melee_attack goal instead)')
    parser.add_argument('--client-wasm', help='the built fox_texture client guest (defaults to yoake_wasm_fox_texture.wasm beside --wasm when present)')
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
    wasm_path = Path(args.wasm).resolve() if args.wasm else None
    client_wasm_path = Path(args.client_wasm).resolve() if args.client_wasm else None
    if client_wasm_path is None and wasm_path is not None:
        sibling = wasm_path.with_name('yoake_wasm_fox_texture.wasm')
        if sibling.exists():
            client_wasm_path = sibling
    results = run(Path(args.bin_dir).resolve() / f'horizon-ctl{ext}', out, Path(args.shots).resolve(), resources, wasm_path, client_wasm_path)
    (out / 'results.json').write_text(json.dumps({'checks': results}, indent=2) + '\n')
    print(json.dumps(results, indent=1))
    sys.exit(0 if all(r['pass'] for r in results.values()) else 1)


if __name__ == '__main__':
    main()
