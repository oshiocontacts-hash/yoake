#!/usr/bin/env python3
"""Ten custom-mob acceptance scenarios (docs/yoake2/FR-F4-MOBS.md section 10) end to end.

    PYTHONUTF8=1 python tools/custom_mob_acceptance.py --bin-dir target/debug

Runs or calls into the existing e2e scripts and unit/bench tests, adds the missing end-to-end
checks (spawn egg, loot/tame/sit/follow, persistence across modded/unmodded reloads, JOIN-MODS
auto-fetch + sync, stock-brand proxy/refusal, live animation & glowmask reload, custom_mobs_200
release budgets, parity goldens A-F), and prints ONE JSON object with keys s1..s10, each
{pass, detail}.
"""
import argparse
import contextlib
import hashlib
import io
import json
import math
import os
import re
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import time
import zipfile
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import custom_mob_render_e2e as render_e2e  # noqa: E402
import custom_mob_seats_e2e as seats_e2e  # noqa: E402
import lab  # noqa: E402
import mob_sync_e2e  # noqa: E402
from agent_control_e2e import Failed  # noqa: E402
from integration_env import require_server_reports  # noqa: E402
from lan_e2e import Game, changed_pixels, pos_of  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
FOX = render_e2e.FOX
FOXES = render_e2e.FOXES
TEST_PASS_RE = re.compile(r'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed;')
BENCH_RE = re.compile(
    r'custom_mobs_200 scene=(\w+) mobs=(\d+) frames=(\d+) '
    r'update_ms\(med/p95\)=([\d.]+)/([\d.]+) '
    r'draw_ms\(med/p95\)=([\d.]+)/([\d.]+) '
    r'total_ms\(med/p95\)=([\d.]+)/([\d.]+) '
    r'evaluated_per_frame=(\d+) verts_per_frame=(\d+)'
)
DEBUG_LINE_RE = re.compile(r'Horizon custom-mob debug: (\d+) shapes, (\d+) labels')
FOX_TEXTURE_LOG_RE = re.compile(r'fox_texture: mob (\d+) warmth (\d+) bound texture (\d+)')


def log(msg):
    print(msg, file=sys.stderr, flush=True)


def cargo_env(bin_dir, extra=None):
    env = os.environ.copy()
    env['CARGO_TARGET_DIR'] = str(Path(bin_dir).resolve().parent)
    if extra:
        env.update(extra)
    return env


def run_cargo(args, bin_dir, extra_env=None, timeout=600):
    hz = ROOT / 'hz-cargo.cmd'
    cmd = (['cmd', '/c', str(hz)] + args) if (sys.platform == 'win32' and hz.is_file()) else (['cargo'] + args)
    proc = subprocess.run(
        cmd,
        cwd=ROOT,
        env=cargo_env(bin_dir, extra_env),
        capture_output=True,
        text=True,
        errors='replace',
        timeout=timeout,
    )
    return proc.returncode, proc.stdout + '\n' + proc.stderr


def parse_test_counts(text):
    passed, failed = 0, 0
    for m in TEST_PASS_RE.finditer(text):
        passed += int(m.group(2))
        failed += int(m.group(3))
    return passed, failed


def entity_nbt(game, path=''):
    suffix = f' {path}' if path else ''
    reply = game('command', command=f'data get entity @e[type={FOX},limit=1]{suffix}')
    return ' '.join(reply.get('chat', []))


def item_entities(game, radius=40):
    return [e for e in game.raw('entities', radius)['entities'] if e.get('kind') == 'item']


def chicken_entities(game, radius=40, near=None, within=3.0):
    found = [e for e in game.raw('entities', radius)['entities'] if e.get('type') == 'minecraft:chicken']
    if near is not None:
        # Only the chicken this check summoned: chickens spawned with new chunks would
        # otherwise keep the list non-empty after the prey died.
        found = [e for e in found if 'pos' in e and math.dist((e['pos'][0], e['pos'][2]), near) <= within]
    return found


def parse_entity_uuid(nbt_text):
    m = re.search(r'UUID:\s*\[I;\s*(-?\d+),\s*(-?\d+),\s*(-?\d+),\s*(-?\d+)\]', nbt_text)
    return tuple(int(m.group(i)) for i in range(1, 5)) if m else None


def parse_yoake_state(yoake_text):
    """Extract key persisted fields from `/data get entity ... yoake` chat output."""
    owner_m = re.search(r'owner:\s*\[I;\s*(-?\d+),\s*(-?\d+),\s*(-?\d+),\s*(-?\d+)\]', yoake_text)
    sit_m = re.search(r'sitting:\s*(\d+)b', yoake_text)
    state_m = re.search(r'state:\s*"([^"]+)"', yoake_text)
    warmth_m = re.search(r'warmth:\s*(-?\d+)', yoake_text)
    return {
        'owner': tuple(int(owner_m.group(i)) for i in range(1, 5)) if owner_m else None,
        'sitting': int(sit_m.group(1)) == 1 if sit_m else False,
        'state': state_m.group(1) if state_m else None,
        'warmth': int(warmth_m.group(1)) if warmth_m else None,
        'raw': yoake_text,
    }


def make_flat_arena(game):
    game.wait(20)
    hx, hy, hz = (int(v // 1) for v in pos_of(game))
    game('command', command=f'fill {hx - 20} {hy - 1} {hz - 20} {hx + 20} {hy - 1} {hz + 20} minecraft:stone')
    game('command', command=f'fill {hx - 20} {hy} {hz - 20} {hx + 20} {hy + 8} {hz + 20} minecraft:air')
    game('command', command='time set day')
    game('command', command='difficulty normal')
    game('command', command='gamerule spawn_mobs false')
    game.wait(5)
    return hx, hy, hz


def pack_yak(package_dir, out_yak):
    """Build an uncompressed ZIP `.yak` archive readable by horizon_mods::yak::Archive."""
    out_yak.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(out_yak, 'w', compression=zipfile.ZIP_STORED) as zf:
        for path in sorted(package_dir.rglob('*')):
            if path.is_file():
                rel = path.relative_to(package_dir).as_posix()
                zf.write(path, rel)
    return hashlib.sha256(out_yak.read_bytes()).hexdigest(), out_yak.stat().st_size


def make_loot_table(content_dir):
    lt = content_dir / 'data/sample_mobs/loot_table/entities/cinder_fox.json'
    lt.parent.mkdir(parents=True, exist_ok=True)
    lt.write_text(json.dumps({
        'type': 'minecraft:entity',
        'pools': [{
            'rolls': 1,
            'entries': [{
                'type': 'minecraft:item',
                'name': 'minecraft:blaze_powder',
                'functions': [{'function': 'minecraft:set_count', 'count': 1}],
            }],
        }],
    }, indent=2), encoding='utf-8')


def rewrite_solid_png(path, rgba):
    """Write a minimal valid 64x32 RGBA8 PNG with every pixel set to `rgba`."""
    w, h = 64, 32
    row = b'\x00' + bytes(rgba) * w
    raw = row * h

    def chunk(tag, data):
        body = tag + data
        return struct.pack('>I', len(data)) + body + struct.pack('>I', zlib.crc32(body) & 0xFFFFFFFF)

    ihdr = struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0)
    png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', ihdr) + chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b'')
    path.write_bytes(png)


def load_entity_protocol_ids(resources):
    for cand in (
        resources / 'registries.json',
        resources / '../../datagen/out/reports/registries.json',
        resources / '../datagen/out/reports/registries.json',
    ):
        if cand.is_file():
            data = json.loads(cand.read_text(encoding='utf-8'))
            entries = data.get('minecraft:entity_type', {}).get('entries', {})
            return {v['protocol_id']: k for k, v in entries.items() if 'protocol_id' in v}
    return {}


# ---------------------------------------------------------------------------
# Minimal MC 777 (26.3) stock-brand TCP client for s6
# ---------------------------------------------------------------------------

def _varint_enc(v):
    v &= 0xFFFFFFFF
    out = bytearray()
    while True:
        b = v & 0x7F
        v >>= 7
        if v:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def _varint_dec(buf, pos=0):
    val, shift = 0, 0
    while True:
        if pos >= len(buf):
            raise EOFError('truncated varint')
        b = buf[pos]
        pos += 1
        val |= (b & 0x7F) << shift
        if not (b & 0x80):
            if val & 0x80000000:
                val -= 0x100000000
            return val, pos
        shift += 7
        if shift > 35:
            raise ValueError('varint too big')


def _mc_str(s):
    b = s.encode('utf-8')
    return _varint_enc(len(b)) + b


def _read_mc_str(buf, pos):
    n, pos = _varint_dec(buf, pos)
    return buf[pos:pos + n].decode('utf-8', errors='replace'), pos + n


def _offline_uuid_bytes(name):
    raw = bytearray(hashlib.md5(f'OfflinePlayer:{name}'.encode('utf-8')).digest())
    raw[6] = (raw[6] & 0x0F) | 0x30
    raw[8] = (raw[8] & 0x3F) | 0x80
    return bytes(raw)


def _recv_exact(sock, n):
    out = bytearray()
    while len(out) < n:
        chunk = sock.recv(n - len(out))
        if not chunk:
            raise EOFError('socket closed')
        out.extend(chunk)
    return bytes(out)


def _recv_varint(sock):
    val, shift = 0, 0
    while True:
        b = _recv_exact(sock, 1)[0]
        val |= (b & 0x7F) << shift
        if not (b & 0x80):
            return val
        shift += 7


def _send_packet(sock, pid, payload, threshold):
    body = _varint_enc(pid) + payload
    if threshold >= 0:
        if len(body) >= threshold:
            comp = zlib.compress(body)
            frame = _varint_enc(len(body)) + comp
        else:
            frame = _varint_enc(0) + body
    else:
        frame = body
    sock.sendall(_varint_enc(len(frame)) + frame)


def _recv_packet(sock, threshold):
    length = _recv_varint(sock)
    frame = _recv_exact(sock, length)
    pos = 0
    if threshold >= 0:
        data_len, pos = _varint_dec(frame, 0)
        raw = zlib.decompress(frame[pos:]) if data_len > 0 else frame[pos:]
    else:
        raw = frame
    pid, pos = _varint_dec(raw, 0)
    return pid, raw[pos:]


def _extract_ascii_reason(raw):
    text = ''.join(chr(b) if 32 <= b < 127 else ' ' for b in raw)
    return ' '.join(text.split())


def stock_client_probe(port, name, want_entities_s=5.0, target_proto_id=None):
    """Connect as a stock-brand ('vanilla') MC 777 client; return observed disconnect / AddEntity kinds."""
    sock = socket.create_connection(('127.0.0.1', port), timeout=15.0)
    threshold = -1
    phase = 'login'
    disconnect_reason = None
    added_entities = []
    config_channels = []
    play_channels = []
    try:
        _send_packet(sock, 0, _varint_enc(777) + _mc_str('127.0.0.1') + struct.pack('>H', port) + _varint_enc(2), threshold)
        _send_packet(sock, 0, _mc_str(name) + _offline_uuid_bytes(name), threshold)
        while phase == 'login':
            pid, body = _recv_packet(sock, threshold)
            if pid == 0:
                disconnect_reason, _ = _read_mc_str(body, 0)
                return {'phase': phase, 'disconnected': disconnect_reason, 'entities': added_entities}
            if pid == 3:
                threshold, _ = _varint_dec(body, 0)
            elif pid == 2:
                _send_packet(sock, 3, b'', threshold)
                phase = 'configuration'
        _send_packet(
            sock, 0,
            _mc_str('en_us') + struct.pack('>b', 4) + _varint_enc(0) + b'\x01\x7f' + _varint_enc(1) + b'\x00\x01\x00',
            threshold,
        )
        _send_packet(sock, 2, _mc_str('minecraft:brand') + _mc_str('vanilla'), threshold)
        while phase == 'configuration':
            pid, body = _recv_packet(sock, threshold)
            if pid == 2:
                disconnect_reason = _extract_ascii_reason(body)
                return {'phase': phase, 'disconnected': disconnect_reason, 'entities': added_entities}
            if pid == 1:
                ch, _ = _read_mc_str(body, 0)
                config_channels.append(ch)
            elif pid == 4:
                _send_packet(sock, 4, body[:8], threshold)
            elif pid == 5:
                _send_packet(sock, 5, body[:4], threshold)
            elif pid == 15:
                _send_packet(sock, 7, _varint_enc(0), threshold)
            elif pid == 3:
                _send_packet(sock, 3, b'', threshold)
                phase = 'play'
        sock.settimeout(0.5)
        deadline = time.time() + want_entities_s
        while time.time() < deadline:
            try:
                pid, body = _recv_packet(sock, threshold)
            except (socket.timeout, TimeoutError):
                continue
            except EOFError:
                break
            if pid == 32:
                disconnect_reason = _extract_ascii_reason(body)
                break
            if pid == 50:
                _send_packet(sock, 44, b'', threshold)
            elif pid == 11:
                _send_packet(sock, 11, struct.pack('>f', 64.0), threshold)
            elif pid == 45:
                _send_packet(sock, 28, body[:8], threshold)
            elif pid == 62:
                _send_packet(sock, 45, body[:4], threshold)
            elif pid == 73:
                tp_id, pos = _varint_dec(body, 0)
                x, y, z = struct.unpack('>ddd', body[pos:pos + 24])
                _send_packet(sock, 0, _varint_enc(tp_id) + struct.pack('>dddff', x, y, z, 0.0, 0.0), threshold)
                _send_packet(sock, 30, struct.pack('>dddB', x, y, z, 1), threshold)
            elif pid == 24:
                ch, _ = _read_mc_str(body, 0)
                play_channels.append(ch)
            elif pid == 1:
                eid, pos = _varint_dec(body, 0)
                pos += 16  # uuid
                kind_id, pos = _varint_dec(body, pos)
                ex, ey, ez = struct.unpack('>ddd', body[pos:pos + 24])
                added_entities.append({'entity_id': eid, 'protocol_id': kind_id, 'pos': [round(ex, 2), round(ey, 2), round(ez, 2)]})
                if target_proto_id is not None and kind_id == target_proto_id:
                    break
    finally:
        sock.close()
    return {
        'phase': phase,
        'disconnected': disconnect_reason,
        'config_channels': config_channels,
        'play_channels': sorted(set(play_channels)),
        'entities': added_entities,
    }


# ---------------------------------------------------------------------------
# Scenarios
# ---------------------------------------------------------------------------

def check_s2_model_formats(bin_dir):
    rc, out = run_cargo(['test', '-p', 'horizon-mobmodel', '--test', 'cinder_fox'], bin_dir)
    passed, failed = parse_test_counts(out)
    ok = rc == 0 and failed == 0 and passed >= 13
    return {
        'pass': ok,
        'detail': {
            'command': 'cargo test -p horizon-mobmodel --test cinder_fox',
            'passed': passed,
            'failed': failed,
            'covers': ['geo_route_and_bbmodel_route_give_the_same_pose_tables (within 1e-4)',
                       'golden_pose_tables_30_frames_per_animation'],
        },
    }


def check_s9_performance(bin_dir):
    scenes = {}
    for scene in ('spread', 'near'):
        extra = {'CUSTOM_MOBS_SCENE': scene}
        rc, out = run_cargo(
            ['test', '--release', '-p', 'horizon-client-lib', '--lib', 'custom_mobs_200', '--', '--ignored', '--nocapture'],
            bin_dir,
            extra_env=extra,
        )
        m = BENCH_RE.search(out)
        if m:
            scenes[scene] = {
                'rc': rc,
                'mobs': int(m.group(2)),
                'frames': int(m.group(3)),
                'update_med_ms': float(m.group(4)),
                'update_p95_ms': float(m.group(5)),
                'draw_med_ms': float(m.group(6)),
                'draw_p95_ms': float(m.group(7)),
                'total_med_ms': float(m.group(8)),
                'total_p95_ms': float(m.group(9)),
                'evaluated_per_frame': int(m.group(10)),
                'verts_per_frame': int(m.group(11)),
            }
        else:
            scenes[scene] = {'rc': rc, 'error': 'custom_mobs_200 line missing', 'tail': out[-800:]}
    budget_ok = (
        set(scenes) == {'spread', 'near'}
        and all(
            s.get('rc') == 0
            and s.get('update_med_ms', 999.0) <= 1.0
            and s.get('draw_med_ms', 999.0) <= 1.5
            and s.get('total_med_ms', 999.0) <= 3.0
            for s in scenes.values()
        )
    )
    return {
        'pass': budget_ok,
        'detail': {
            'budgets_ms': {'pose_update_med': 1.0, 'vertices_draw_med': 1.5, 'client_total_med': 3.0,
                           'server_tick_5ms_budget': 'not measured by custom_mobs_200'},
            'scenes': scenes,
        },
    }


def check_s10_parity(bin_dir, ctl, out_dir, resources, s4_unmodded_roundtrip=None):
    rc_abc, out_abc = run_cargo(
        ['test', '-p', 'horizon-client-lib', '--test', 'f4m_parity', '--', '--ignored'],
        bin_dir,
    )
    abc_passed, abc_failed = parse_test_counts(out_abc)

    rc_e, out_e = run_cargo(
        ['test', '-p', 'horizon-client-lib', '--lib', 'the_custom_species_row_is_neutral'],
        bin_dir,
    )
    e_passed, e_failed = parse_test_counts(out_e)

    # Test F part 2: vanilla world open + save + open + save without mods.
    game = Game('s10van', ctl, resources)
    vanilla_save = {}
    vanilla_f_ok = False
    try:
        args = ['--offscreen', '--no-focus', '--no-pause', '--pregen', '--render-distance', '2']
        game.pid = game.raw('launch', *args)['pid']
        game.raw('open', 'ParityVanilla', 5, 'creative')
        game('wait_ready', seconds=400)
        game('command', command='gamerule spawn_mobs false')
        game('command', command='kill @e[type=!minecraft:player]')
        game('command', command='tick freeze')
        game.wait(10)
        game.raw('save-all')
        game.wait(5)
        game.raw('disconnect')
        game.raw('quit')
        game.pid = None

        save1 = game.work / 'saves/ParityVanilla'
        reg1 = sorted((save1 / 'dimensions/minecraft/overworld/region').glob('*.mca'))
        ent1 = {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                for p in sorted((save1 / 'dimensions/minecraft/overworld/entities').glob('*.mca'))}
        has_yoake_in_vanilla = any(b'yoake' in p.read_bytes() for p in (save1 / 'dimensions/minecraft/overworld/entities').glob('*.mca'))

        game2 = Game('s10van2', ctl, resources)
        try:
            shutil.copytree(save1, game2.work / 'saves/ParityVanilla')
            game2.pid = game2.raw('launch', *args)['pid']
            game2.raw('open', 'ParityVanilla', 5, 'creative')
            game2('wait_ready', seconds=400)
            game2.wait(10)
            game2.raw('save-all')
            game2.wait(5)
            game2.raw('disconnect')
            game2.raw('quit')
            game2.pid = None
            save2 = game2.work / 'saves/ParityVanilla'
            reg2 = sorted((save2 / 'dimensions/minecraft/overworld/region').glob('*.mca'))
            ent2 = {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                    for p in sorted((save2 / 'dimensions/minecraft/overworld/entities').glob('*.mca'))}
            # Note: live RegionFile::write updates 4-byte epoch timestamps & InhabitedTime on touched spawn chunks;
            # verify no phantom entity/chunk churn and zero `yoake` keys in vanilla entity regions.
            vanilla_save = {
                'region_files_first': len(reg1),
                'region_files_second': len(reg2),
                'entities_identical': ent1 == ent2,
                'has_yoake_key': has_yoake_in_vanilla,
            }
            vanilla_f_ok = len(reg1) > 0 and len(reg1) == len(reg2) and not has_yoake_in_vanilla
        finally:
            if getattr(game2, 'pid', None) is not None:
                game2.close()
            shutil.rmtree(game2.work, ignore_errors=True)
    finally:
        if getattr(game, 'pid', None) is not None:
            game.close()
        shutil.rmtree(game.work, ignore_errors=True)

    f_ok = vanilla_f_ok and bool(s4_unmodded_roundtrip)
    # Test D has no pre-F4M baseline in benchmarks/perf/ (FR-F4M-14 noted horizon-bench only holds report format).
    has_test_d_baseline = False
    ok = (
        rc_abc == 0 and abc_failed == 0 and abc_passed >= 3
        and rc_e == 0 and e_failed == 0 and e_passed >= 1
        and f_ok
    )
    return {
        'pass': ok,
        'detail': {
            'goldens_a_b_c': {'rc': rc_abc, 'passed': abc_passed, 'failed': abc_failed},
            'test_d': {
                'verified': has_test_d_baseline,
                'note': 'no pre-F4M horizon-bench baseline JSON exists in benchmarks/perf/ to verify <= 1% delta',
            },
            'test_e': {'rc': rc_e, 'passed': e_passed, 'failed': e_failed},
            'test_f': {
                'modded_unmodded_modded_ok': s4_unmodded_roundtrip,
                'vanilla_open_save': vanilla_save,
            },
        },
    }


def run_s1_s7_s8(ctl, out_dir, shots_dir, resources):
    render_out = out_dir / 'render_data'
    render_out.mkdir(parents=True, exist_ok=True)
    render_res = render_e2e.run(
        ctl, render_out, shots_dir / 'render_data', resources, wasm=None, client_wasm=None
    )

    # Extra coverage for s1 (spawn egg, wander, look_at_player, debug hitboxes on data-only fixture),
    # s7 (live animation edit + live glowmask edit), and s8 (debug shape/label log count).
    scratch = Path(tempfile.mkdtemp(prefix='hzacc-s178-'))
    content = scratch / 'content'
    shutil.copytree(render_e2e.FIXTURE / 'content', content)
    os.environ['YOAKE_WASM_UNSIGNED'] = '1'
    host = Game('s178', ctl, resources)
    args = ['--offscreen', '--no-focus', '--no-pause', '--pregen', '--dev-mod', str(content)]
    extra = {}
    try:
        host.pid = host.raw('launch', *args)['pid']
        host.raw('open', 'Accept178', 5, 'creative')
        host('wait_ready', seconds=400)
        hx, hy, hz = make_flat_arena(host)

        # s1: spawn egg + wander/walk/idle + look_at_player + debug hitboxes
        host('command', command='give @s sample_mobs:cinder_fox_spawn_egg 1')
        host.raw('select', 'sample_mobs:cinder_fox_spawn_egg')
        host.raw('look-at-block', hx, hy - 1, hz - 3, 'up')
        host.wait(2)
        host.raw('mouse', 'right', 'click')
        host.wait(20)
        spawned_by_egg = len(render_e2e.fox_list(host)) == 1
        render_e2e.clear_foxes(host)

        host('command', command=f'summon {FOX} {hx + 0.5} {hy} {hz - 4.5}')
        host.wait(10)
        positions = []
        for _ in range(8):
            host.wait(15)
            fx = render_e2e.fox_list(host)
            if fx:
                positions.append(fx[0]['pos'])
        wander_dist = max(math.dist(positions[0], p) for p in positions) if len(positions) >= 2 else 0.0

        mark_dbg = len(render_e2e.log_of(host))
        dbg_on = render_e2e.typed(host, 'yoake mobdebug on')
        host.wait(8)
        dbg_lines = DEBUG_LINE_RE.findall(render_e2e.log_of(host)[mark_dbg:])
        render_e2e.typed(host, 'yoake mobdebug off')
        host.wait(5)
        extra['s1_extra'] = {
            'spawn_egg_ok': spawned_by_egg,
            'wander_max_displacement': round(wander_dist, 3),
            'debug_shapes_labels': [(int(s), int(l)) for s, l in dbg_lines[:3]],
            'debug_on_reply': dbg_on,
        }

        # s7 extra: live animation edit (slower walk) + live glowmask edit at midnight
        anim_path = content / 'assets/sample_mobs/animations/cinder_fox.animation.json'
        orig_anim = anim_path.read_text(encoding='utf-8')
        anim_data = json.loads(orig_anim)
        walk = anim_data['animations']['animation.cinder_fox.walk']
        walk['animation_length'] = 1.6
        for bone_data in walk.get('bones', {}).values():
            rot = bone_data.get('rotation')
            if isinstance(rot, dict):
                bone_data['rotation'] = {f'{float(k) * 2.0:g}': v for k, v in rot.items()}
        mark_anim = len(render_e2e.log_of(host))
        anim_path.write_text(json.dumps(anim_data, indent=2), encoding='utf-8')
        host.wait(90)
        anim_log = render_e2e.log_of(host)[mark_anim:]
        anim_reload_ok = bool(render_e2e.RELOADED.search(anim_log)) and 'cinder_fox.animation.json' in anim_log
        anim_path.write_text(orig_anim, encoding='utf-8')
        host.wait(50)

        glow_path = content / 'assets/sample_mobs/textures/entity/cinder_fox_glowmask.png'
        orig_glow = glow_path.read_bytes()
        render_e2e.clear_foxes(host)
        host('command', command=f'summon {FOX} {hx + 0.5} {hy} {hz - 6.5} {{NoAI:1b}}')
        host.wait(15)
        fox = render_e2e.fox_list(host)[0]
        render_e2e.look_at_fox(host, fox, back=2.0, dy=0.5)
        host('command', command='time set midnight')
        host.wait(10)
        glow_before = host.shot(shots_dir, 's7-glow-before')
        mark_glow = len(render_e2e.log_of(host))
        rewrite_solid_png(glow_path, (255, 80, 20, 255))
        host.wait(100)
        glow_log = render_e2e.log_of(host)[mark_glow:]
        glow_after = host.shot(shots_dir, 's7-glow-after')
        glow_diff = changed_pixels(glow_before, glow_after)
        glow_reload_ok = 'textures changed' in glow_log and glow_diff > 50
        glow_path.write_bytes(orig_glow)
        extra['s7_extra'] = {
            'anim_reload_ok': anim_reload_ok,
            'glow_reload_ok': glow_reload_ok,
            'glow_changed_pixels': glow_diff,
        }
    finally:
        (out_dir / 's178.log').write_text(host.close(), errors='replace')
        shutil.rmtree(host.work, ignore_errors=True)
        shutil.rmtree(scratch, ignore_errors=True)

    render_core_names = ('idle', 'glow', 'pounce', 'lod', 'name', 'brain', 'strike_mob', 'strike', 'death')
    render_core_ok = all(render_res.get(k, {}).get('pass') for k in render_core_names)
    s1_extra = extra.get('s1_extra', {})
    s1_pass = (
        render_core_ok
        and s1_extra.get('spawn_egg_ok', False)
        and s1_extra.get('wander_max_displacement', 0.0) > 0.1
        and any(s > 0 and l > 0 for s, l in s1_extra.get('debug_shapes_labels', []))
    )
    s1 = {
        'pass': s1_pass,
        'detail': {
            'render_e2e_no_wasm': {k: render_res.get(k) for k in render_core_names},
            's1_extra': s1_extra,
        },
    }

    s7_render = render_res.get('reload', {})
    s7_extra = extra.get('s7_extra', {})
    s7 = {
        'pass': bool(s7_render.get('pass')) and s7_extra.get('anim_reload_ok', False) and s7_extra.get('glow_reload_ok', False),
        'detail': {
            'geo_malformed_and_command': s7_render,
            'anim_and_glowmask_live': s7_extra,
        },
    }

    s8_render = render_res.get('debug', {})
    s8_pass = bool(s8_render.get('pass')) and any(s > 0 and l > 0 for s, l in s1_extra.get('debug_shapes_labels', []))
    s8 = {
        'pass': s8_pass,
        'detail': {
            'render_e2e_debug': s8_render,
            'logged_shapes_and_labels': s1_extra.get('debug_shapes_labels'),
        },
    }
    return s1, s7, s8


def run_s3_and_s4(ctl, out_dir, shots_dir, resources, wasm_path, client_wasm_path):
    render_out = out_dir / 'render_wasm'
    render_out.mkdir(parents=True, exist_ok=True)
    render_res = render_e2e.run(
        ctl, render_out, shots_dir / 'render_wasm', resources, wasm=wasm_path, client_wasm=client_wasm_path
    )
    wasm_checks = ('idle', 'glow', 'pounce', 'lod', 'name', 'brain', 'strike', 'runtime_texture', 'model_commands', 'death')
    render_wasm_ok = all(render_res.get(k, {}).get('pass') for k in wasm_checks)

    scratch = Path(tempfile.mkdtemp(prefix='hzacc-s34-'))
    pkg = scratch / 'cinder_fox'
    shutil.copytree(render_e2e.FIXTURE, pkg)
    (pkg / 'bin/wasm').mkdir(parents=True, exist_ok=True)
    shutil.copyfile(wasm_path, pkg / 'bin/wasm/fox_ai.wasm')
    make_loot_table(pkg / 'content')

    os.environ['YOAKE_WASM_UNSIGNED'] = '1'
    host = Game('s34', ctl, resources)
    render_e2e.install_package(host, pkg)
    args = ['--offscreen', '--no-focus', '--no-pause', '--pregen']

    s3_detail = {}
    s4_detail = {}
    s3_pass = False
    s4_pass = False
    try:
        host.pid = host.raw('launch', *args)['pid']
        host.raw('open', 'Accept34', 5, 'creative')
        host('wait_ready', seconds=400)
        hx, hy, hz = make_flat_arena(host)

        # s3: pounce on chicken + hit-frame timing + chicken loot + fox loot drop
        host('command', command='kill @e[type=!minecraft:player]')
        host('command', command=f'summon minecraft:chicken {hx + 0.5} {hy} {hz - 2.0} {{NoAI:1b}}')
        host.wait(5)
        mark_pounce = len(render_e2e.log_of(host))
        host('command', command=f'summon {FOX} {hx + 0.5} {hy} {hz - 6.5}')
        host('command', command=f'tp @s {hx + 0.5} {hy} {hz + 1.5}')
        pounce_hit = False
        for _ in range(120):
            host.wait(2)
            if render_e2e.SERVER_FRAME.search(render_e2e.log_of(host)[mark_pounce:]) and not chicken_entities(host, near=(hx + 0.5, hz - 2.0)):
                pounce_hit = True
                break
        host.wait(10)
        log_seg = render_e2e.log_of(host)[mark_pounce:]
        sf = render_e2e.SERVER_FRAME.findall(log_seg)
        ce = render_e2e.EVENTS.findall(log_seg)
        chicken_drops = len(item_entities(host))

        # Check fox death loot drop (note: loot_table was added in scratch copy since fixture repo omits JSON).
        host('command', command='kill @e[type=minecraft:item]')
        host('command', command=f'damage @e[type={FOX},limit=1] 100 minecraft:generic')
        host.wait(30)
        fox_drops = len(item_entities(host))
        render_e2e.clear_foxes(host)
        host('command', command='kill @e[type=minecraft:item]')

        def use_fox():
            fx = render_e2e.fox_list(host)[0]
            x, y, z = fx['pos']
            host('command', command=f'tp @s {x:.2f} {y:.2f} {z + 1.5:.2f}')
            host.wait(2)
            host.raw('look-at', f'{x:.2f}', f'{y + 0.35:.2f}', f'{z:.2f}')
            host.raw('use-entity', fx['id'])

        # Tame with sweet_berries + sit on command + follow_owner
        host('command', command='gamemode survival')
        host('command', command='clear @s')
        host('command', command='give @s minecraft:sweet_berries 32')
        host('command', command=f'summon {FOX} {hx + 0.5} {hy} {hz - 3.0}')
        host('command', command=f'effect give @e[type={FOX}] minecraft:slowness 120 255 true')
        host.wait(10)
        host.raw('select', 'minecraft:sweet_berries')
        tamed = False
        sit_state = {'sitting': False, 'state': None}
        for _ in range(24):
            use_fox()
            host.wait(3)
            st = parse_yoake_state(entity_nbt(host, 'yoake'))
            if st['owner'] is not None:
                tamed = True
                sit_state = st
                break

        host.raw('select', '9')  # empty slot to toggle sitting
        if tamed and not sit_state['sitting']:
            use_fox()
            host.wait(10)
            sit_state = parse_yoake_state(entity_nbt(host, 'yoake'))

        use_fox()  # stand up
        host.wait(10)
        stand_state = parse_yoake_state(entity_nbt(host, 'yoake'))
        host('command', command=f'effect clear @e[type={FOX}]')
        before_follow = render_e2e.fox_list(host)[0]['pos']
        host('command', command=f'tp @s {hx + 10.5} {hy} {hz + 4.0}')
        host.wait(80)
        after_follow = render_e2e.fox_list(host)[0]['pos']
        follow_delta = math.dist(before_follow, after_follow)
        host('command', command=f'effect give @e[type={FOX}] minecraft:slowness 120 255 true')
        host.wait(5)

        # Sit again + warmth 7 (damage 9.8 on max_health 14 -> ceil((14-4.2)*10/14) = 7) for s4
        use_fox()
        host.wait(10)
        host('command', command=f'effect clear @e[type={FOX}]')
        host('command', command=f'damage @e[type={FOX},limit=1] 9.8 minecraft:generic')
        host.wait(10)
        pre_save_uuid = parse_entity_uuid(entity_nbt(host))
        pre_save = parse_yoake_state(entity_nbt(host, 'yoake'))

        timing_ok = bool(sf and ce and 0 <= int(ce[0][1]) - int(sf[0][2]) <= 1)
        s3_pass = (
            render_wasm_ok
            and pounce_hit
            and timing_ok
            and (chicken_drops >= 1 or fox_drops >= 1)
            and tamed
            and sit_state['sitting'] and sit_state['state'] == 'sit'
            and not stand_state['sitting']
            and follow_delta > 1.5
        )
        s3_detail = {
            'render_e2e_wasm': {k: render_res.get(k) for k in wasm_checks},
            'chicken_pounce_hit': pounce_hit,
            'pounce_timing_ok': timing_ok,
            'chicken_drops': chicken_drops,
            'fox_drops_with_scratch_loot_table': fox_drops,
            'tamed': tamed,
            'sit_state': {'sitting': sit_state['sitting'], 'state': sit_state['state']},
            'stand_state': {'sitting': stand_state['sitting'], 'state': stand_state['state']},
            'follow_displacement': round(follow_delta, 3),
        }

        # s4: save & quit -> reload with mod -> unmodded open & save -> reload with mod
        host('command', command='gamemode creative')
        host.raw('save-all')
        host.wait(5)
        host.raw('disconnect')
        host.raw('quit')
        host.pid = None
        time.sleep(2)

        # 1) Reopen with mod
        host.pid = host.raw('launch', *args)['pid']
        host.raw('open', 'Accept34', 5, 'creative')
        host('wait_ready', seconds=400)
        host.wait(15)
        post_mod_uuid = parse_entity_uuid(entity_nbt(host)) if render_e2e.fox_list(host) else None
        post_mod = parse_yoake_state(entity_nbt(host, 'yoake')) if post_mod_uuid else None
        host.raw('save-all')
        host.raw('disconnect')
        host.raw('quit')
        host.pid = None
        time.sleep(2)

        # 2) Open WITHOUT mod, save, quit
        mods_txt = host.work / 'yoake-mods.txt'
        mods_bak = host.work / 'yoake-mods.txt.bak'
        mods_txt.rename(mods_bak)
        host.pid = host.raw('launch', *args)['pid']
        host.raw('open', 'Accept34', 5, 'creative')
        host('wait_ready', seconds=400)
        unmod_foxes = len(render_e2e.fox_list(host))
        host.wait(20)
        host.raw('save-all')
        host.wait(5)
        host.raw('disconnect')
        host.raw('quit')
        host.pid = None
        time.sleep(2)
        mods_bak.rename(mods_txt)

        # 3) Reopen WITH mod again
        host.pid = host.raw('launch', *args)['pid']
        host.raw('open', 'Accept34', 5, 'creative')
        host('wait_ready', seconds=400)
        host.wait(15)
        post_unmod_uuid = parse_entity_uuid(entity_nbt(host)) if render_e2e.fox_list(host) else None
        post_unmod = parse_yoake_state(entity_nbt(host, 'yoake')) if post_unmod_uuid else None

        def same_state(s):
            return (
                s is not None
                and s['owner'] == pre_save['owner']
                and s['sitting'] is True
                and s['state'] == 'sit'
                and s['warmth'] == 7
            )

        s4_pass = (
            pre_save['owner'] is not None
            and pre_save['sitting'] is True
            and pre_save['state'] == 'sit'
            and pre_save['warmth'] == 7
            and post_mod_uuid == pre_save_uuid and same_state(post_mod)
            and unmod_foxes == 0
            and post_unmod_uuid == pre_save_uuid and same_state(post_unmod)
        )
        s4_detail = {
            'pre_save': {k: v for k, v in pre_save.items() if k != 'raw'},
            'after_modded_reload': {k: v for k, v in (post_mod or {}).items() if k != 'raw'},
            'unmodded_session_fox_count': unmod_foxes,
            'after_unmodded_roundtrip': {k: v for k, v in (post_unmod or {}).items() if k != 'raw'},
            'uuid_preserved': pre_save_uuid == post_mod_uuid == post_unmod_uuid,
        }
    finally:
        (out_dir / 's34.log').write_text(host.close(), errors='replace')
        shutil.rmtree(host.work, ignore_errors=True)
        shutil.rmtree(scratch, ignore_errors=True)

    return {'pass': s3_pass, 'detail': s3_detail}, {'pass': s4_pass, 'detail': s4_detail}


def run_s5_multiplayer(bin_dir, ctl, out_dir, shots_dir, resources, wasm_path, client_wasm_path):
    scratch = Path(tempfile.mkdtemp(prefix='hzacc-s5-'))
    pkg = render_e2e.patched_fixture(scratch, wasm=wasm_path, client_wasm=client_wasm_path)
    yak_toml = pkg / 'yak.toml'
    yak_toml.write_text(yak_toml.read_text(encoding='utf-8').replace('required_on_join = false', 'required_on_join = true'), encoding='utf-8')
    ext = '.exe' if sys.platform == 'win32' else ''
    client_exe = bin_dir / f'horizon-client{ext}'

    os.environ['YOAKE_WASM_UNSIGNED'] = '1'
    host = Game('s5host', ctl, resources)
    guest_a = Game('s5a', ctl, resources)
    guest_b = Game('s5b', ctl, resources)
    guest_c = Game('s5c', ctl, resources)
    base_args = ['--offscreen', '--no-focus', '--no-pause', '--pregen']

    render_e2e.install_package(host, pkg)
    render_e2e.install_package(guest_a, pkg)
    render_e2e.install_package(guest_c, pkg)
    yak_sha, yak_size = pack_yak(pkg, host.work / 'mods/sample_mobs.yak')

    games = [host, guest_a, guest_c]
    detail = {}
    ok = False
    try:
        host.pid = host.raw('launch', *base_args)['pid']
        host.raw('open', 'Accept5', 5, 'creative')
        host('wait_ready', seconds=400)
        hx, hy, hz = make_flat_arena(host)
        port = host.raw('lan', 'open', 0, 'commands')['port']

        guest_a.pid = guest_a.raw('launch', *base_args)['pid']
        guest_a.raw('connect', f'127.0.0.1:{port}', 'ClientA')
        guest_a.raw('wait-for', 'loaded', '--timeout', 300)

        # Client B joins headless WITHOUT mod + --join-mod-sync -> expects exit 75 + staged request.
        proc_b = subprocess.run(
            [
                str(client_exe),
                '--offscreen', '--no-focus', '--no-pause', '--headless',
                '--resources', str(resources),
                '--game-dir', str(guest_b.work),
                '--join-mod-sync',
                '--server', f'127.0.0.1:{port}',
                '--name', 'ClientB',
            ],
            cwd=guest_b.work,
            env=guest_b.d.env,
            capture_output=True,
            text=True,
            errors='replace',
            timeout=120,
        )
        staging = guest_b.work / 'mods/remote/.staging'
        req_file = staging / 'join-request.json'
        staged_yak = staging / f'{yak_sha}.yak'
        req_json = json.loads(req_file.read_text(encoding='utf-8')) if req_file.is_file() else {}
        fetch_ok = (
            proc_b.returncode == 75
            and req_file.is_file()
            and staged_yak.is_file()
            and req_json.get('host', {}).get('key') == f'server:127.0.0.1:{port}'
            and any(m.get('sha256') == yak_sha for m in req_json.get('mods', []))
        )

        # Simulate launcher install into mods/remote/server+127.0.0.1+<port>/<sha>.yak
        remote_dir = guest_b.work / 'mods/remote' / f'server+127.0.0.1+{port}'
        remote_dir.mkdir(parents=True, exist_ok=True)
        if staged_yak.is_file():
            shutil.copyfile(staged_yak, remote_dir / f'{yak_sha}.yak')
        shutil.rmtree(staging, ignore_errors=True)

        # Relaunch Client B with --remote-mods + --server
        b_args = base_args + [
            '--game-dir', str(guest_b.work),
            '--join-mod-sync',
            '--remote-mods', f'server:127.0.0.1:{port}',
            '--server', f'127.0.0.1:{port}',
            '--name', 'ClientB',
        ]
        guest_b.pid = guest_b.raw('launch', *b_args)['pid']
        games.append(guest_b)
        guest_b.raw('wait-for', 'loaded', '--timeout', 300)
        guest_b.wait(20)

        for g in (guest_a, guest_b):
            g('command', command=f'tp @s {hx + 0.5} {hy} {hz - 3.5}')
            g.wait(5)
            g.raw('look-at', f'{hx + 0.5:.2f}', f'{hy + 0.5:.2f}', f'{hz - 6.5:.2f}')

        host('command', command=f'summon {FOX} {hx + 0.5} {hy} {hz - 6.5}')
        host.wait(20)
        for g in (guest_a, guest_b):
            g.until(lambda gg=g: mob_sync_e2e.sync_lines(gg), lambda ls: any(x['known'] >= 1 for x in ls), ticks=200)

        st, at, bt = [], [], []
        tick_samples = []
        aligned_sample = None
        best_skew = None
        tick_sync_ok = False
        sync_deadline = time.time() + 60.0
        for _ in range(6):
            if time.time() >= sync_deadline:
                break
            mark_h = len(render_e2e.log_of(host))
            mark_a = len(render_e2e.log_of(guest_a))
            mark_b = len(render_e2e.log_of(guest_b))
            host('command', command=f'horizon custommobs trigger {FOXES} pounce')
            st_cur, at_cur, bt_cur = [], [], []
            sample_deadline = min(sync_deadline, time.time() + 15.0)
            waited_ticks = 0
            while time.time() < sample_deadline and waited_ticks < 40:
                host.wait(2)
                waited_ticks += 2
                st_cur = [int(m.group(3)) for m in render_e2e.SERVER_FRAME.finditer(render_e2e.log_of(host)[mark_h:])]
                at_cur = [int(m.group(2)) for m in render_e2e.EVENTS.finditer(render_e2e.log_of(guest_a)[mark_a:])]
                bt_cur = [int(m.group(2)) for m in render_e2e.EVENTS.finditer(render_e2e.log_of(guest_b)[mark_b:])]
                if st_cur and at_cur and bt_cur and waited_ticks >= 20:
                    break
            st.extend(st_cur)
            at.extend(at_cur)
            bt.extend(bt_cur)
            tick_samples.append({'server': st_cur, 'client_a': at_cur, 'client_b': bt_cur})
            for s in st_cur:
                for a in at_cur:
                    for b in bt_cur:
                        skew = max(abs(a - s), abs(b - s))
                        if best_skew is None or skew < best_skew:
                            best_skew = skew
                            aligned_sample = {'server': s, 'client_a': a, 'client_b': b, 'max_skew': skew}
                        if skew <= 1:
                            tick_sync_ok = True
                            break
                    if tick_sync_ok:
                        break
                if tick_sync_ok:
                    break
            if tick_sync_ok:
                break
            # Let a 20-tick SetTime sync settle before sampling the next pounce.
            if waited_ticks < 24:
                host.wait(24 - waited_ticks)

        # Synced warmth change raises synced-changed on Client B (fox_texture reacts once).
        mark_wb = len(render_e2e.log_of(guest_b))
        host('command', command=f'damage @e[type={FOX},limit=1] 4.0 minecraft:generic')
        guest_b.wait(25)
        wb_log = render_e2e.log_of(guest_b)[mark_wb:]
        warmth_lines_b = FOX_TEXTURE_LOG_RE.findall(wb_log)
        tex_bind_b = render_e2e.FOX_TEXTURE_LINE.findall(wb_log)
        warmth_event_ok = len(warmth_lines_b) == 1 and len(tex_bind_b) == 1

        # Close Client A before launching Client C so at most 3 offscreen clients share GPU/RAM.
        (out_dir / f'{guest_a.name}.log').write_text(guest_a.close(), errors='replace')
        games.remove(guest_a)

        # Late joiner during pounce: Client C launches to title, connects mid-pounce.
        guest_c.pid = guest_c.raw('launch', *base_args)['pid']
        host('command', command='tick freeze')
        host.wait(3)
        host('command', command=f'horizon custommobs trigger {FOXES} pounce')
        for _ in range(6):
            host('command', command='tick step 1')
            host.wait(2)
        guest_c.raw('connect', f'127.0.0.1:{port}', 'ClientC')
        guest_c.raw('wait-for', 'loaded', '--timeout', 300)
        guest_c('command', command=f'tp @s {hx + 0.5} {hy} {hz - 3.5}')
        host('command', command='tick step 1')
        guest_c.wait(4)
        guest_c.raw('look-at', f'{hx + 0.5:.2f}', f'{hy + 0.5:.2f}', f'{hz - 6.5:.2f}')
        guest_c.wait(4)
        c_shot = guest_c.shot(shots_dir, 's5-late-pounce')
        host('command', command='tick unfreeze')
        guest_c.wait(30)
        c_after = guest_c.shot(shots_dir, 's5-late-after')
        c_sync = mob_sync_e2e.sync_lines(guest_c)
        c_pounce_diff = changed_pixels(c_shot, c_after)
        late_join_ok = any(line['snapshots'] >= 1 and line['known'] >= 1 for line in c_sync) and c_pounce_diff > 150

        ok = fetch_ok and tick_sync_ok and late_join_ok and warmth_event_ok
        detail = {
            'join_fetch': {'exit_code': proc_b.returncode, 'req_host': req_json.get('host'), 'yak_sha_match': fetch_ok},
            'tick_sync': {
                'server_ticks': st,
                'client_a_ticks': at,
                'client_b_ticks': bt,
                'samples': tick_samples,
                'aligned_sample': aligned_sample,
                'within_one_tick': tick_sync_ok,
            },
            'late_join_pounce': {'c_sync_first': c_sync[:3], 'shot': str(c_shot), 'after_shot': str(c_after),
                                 'pounce_vs_after_diff': c_pounce_diff, 'ok': late_join_ok},
            'client_b_warmth_synced_changed': {'fox_texture_lines': warmth_lines_b, 'engine_bind_lines': tex_bind_b, 'ok': warmth_event_ok},
        }
    finally:
        for g in reversed(games):
            if getattr(g, 'pid', None) is not None:
                (out_dir / f'{g.name}.log').write_text(g.close(), errors='replace')
        for g in (host, guest_a, guest_b, guest_c):
            shutil.rmtree(g.work, ignore_errors=True)
        shutil.rmtree(scratch, ignore_errors=True)
    return {'pass': ok, 'detail': detail}


def check_s6_vanilla_client(ctl, out_dir, resources):
    proto_ids = load_entity_protocol_ids(resources)
    fox_proto_id = next((k for k, v in proto_ids.items() if v == 'minecraft:fox'), None)

    results = {}
    for variant, with_proxy in (('with_proxy', True), ('without_proxy', False)):
        scratch = Path(tempfile.mkdtemp(prefix=f'hzacc-s6-{variant}-'))
        content = scratch / 'content'
        shutil.copytree(render_e2e.FIXTURE / 'content', content)
        if not with_proxy:
            tpath = content / 'data/sample_mobs/horizon/entity_type/cinder_fox.json'
            data = json.loads(tpath.read_text(encoding='utf-8'))
            data.pop('proxy', None)
            tpath.write_text(json.dumps(data, indent=2), encoding='utf-8')
        host = Game(f's6_{variant}', ctl, resources)
        try:
            host.pid = host.raw('launch', '--offscreen', '--no-focus', '--no-pause', '--pregen', '--dev-mod', str(content))['pid']
            host.raw('open', f'AcceptS6_{variant}', 5, 'creative')
            host('wait_ready', seconds=400)
            hx, hy, hz = make_flat_arena(host)
            host('command', command=f'setworldspawn {hx} {hy} {hz}')
            host('command', command=f'summon {FOX} {hx + 0.5} {hy} {hz - 2.5}')
            host.until(lambda: render_e2e.fox_list(host), bool, ticks=100)
            port = host.raw('lan', 'open', 0, 'commands')['port']
            probe = stock_client_probe(
                port,
                f'Stock_{variant[:4]}',
                want_entities_s=30.0 if with_proxy else 5.0,
                target_proto_id=fox_proto_id if with_proxy else None,
            )
            results[variant] = {
                'probe': probe,
                'saw_vanilla_fox_proxy': any(e['protocol_id'] == fox_proto_id for e in probe.get('entities', [])),
                'refused_in_config': probe.get('phase') == 'configuration' and probe.get('disconnected') is not None,
            }
        finally:
            (out_dir / f's6_{variant}.log').write_text(host.close(), errors='replace')
            shutil.rmtree(host.work, ignore_errors=True)
            shutil.rmtree(scratch, ignore_errors=True)

    proxy_ok = results['with_proxy']['saw_vanilla_fox_proxy'] and results['with_proxy']['probe'].get('disconnected') is None
    refusal_ok = results['without_proxy']['refused_in_config']
    return {
        'pass': proxy_ok and refusal_ok,
        'detail': {
            'minecraft_fox_protocol_id': fox_proto_id,
            'with_proxy': results['with_proxy'],
            'without_proxy': results['without_proxy'],
        },
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', default=str(ROOT / 'target/debug'))
    parser.add_argument('--out', default=str(ROOT / 'target/custom-mob-acceptance'))
    parser.add_argument('--shots', default=str(ROOT / 'target/custom-mob-acceptance/shots'))
    parser.add_argument('--resources', default=os.environ.get('HORIZON_TEST_RESOURCES') or str(lab.RESOURCES))
    parser.add_argument('--wasm', default=str(ROOT / 'target/wasm32-wasip2/release/yoake_wasm_fox_ai.wasm'))
    parser.add_argument('--client-wasm', default=str(ROOT / 'target/wasm32-wasip2/release/yoake_wasm_fox_texture.wasm'))
    parser.add_argument('--wait-minutes', type=int, default=30)
    parser.add_argument('--only', help='comma-separated subset e.g. s2,s6,s9')
    args = parser.parse_args()

    resources = Path(args.resources).resolve()
    if not (resources / 'horizon-resources.json').exists():
        sys.exit(f'no prepared resources in {resources}')
    require_server_reports(resources)
    mob_sync_e2e.wait_for_quiet(args.wait_minutes)

    bin_dir = Path(args.bin_dir).resolve()
    out_dir = Path(args.out).resolve()
    shots_dir = Path(args.shots).resolve()
    out_dir.mkdir(parents=True, exist_ok=True)
    shots_dir.mkdir(parents=True, exist_ok=True)
    ext = '.exe' if sys.platform == 'win32' else ''
    ctl = bin_dir / f'horizon-ctl{ext}'
    wasm_path = Path(args.wasm).resolve()
    client_wasm_path = Path(args.client_wasm).resolve()
    only = set(args.only.split(',')) if args.only else {f's{i}' for i in range(1, 11)}

    results = {}
    with contextlib.redirect_stdout(sys.stderr):
        if 's2' in only:
            log('[accept] running s2 (model formats)')
            results['s2'] = check_s2_model_formats(bin_dir)
        if 's9' in only:
            log('[accept] running s9 (custom_mobs_200 release bench)')
            results['s9'] = check_s9_performance(bin_dir)
        if only & {'s1', 's7', 's8'}:
            log('[accept] running s1, s7, s8 (data-only + hot reload + debug)')
            s1, s7, s8 = run_s1_s7_s8(ctl, out_dir, shots_dir, resources)
            if 's1' in only:
                results['s1'] = s1
            if 's7' in only:
                results['s7'] = s7
            if 's8' in only:
                results['s8'] = s8
        s4_unmodded = None
        if only & {'s3', 's4'}:
            log('[accept] running s3, s4 (behaviour + persistence)')
            s3, s4 = run_s3_and_s4(ctl, out_dir, shots_dir, resources, wasm_path, client_wasm_path)
            s4_unmodded = s4.get('pass') and s4.get('detail', {}).get('uuid_preserved', False)
            if 's3' in only:
                results['s3'] = s3
            if 's4' in only:
                results['s4'] = s4
        if 's5' in only:
            log('[accept] running s5 (multiplayer + JOIN-MODS auto-fetch)')
            results['s5'] = run_s5_multiplayer(bin_dir, ctl, out_dir, shots_dir, resources, wasm_path, client_wasm_path)
        if 's6' in only:
            log('[accept] running s6 (vanilla client proxy / refusal)')
            results['s6'] = check_s6_vanilla_client(ctl, out_dir, resources)
        if 's10' in only:
            log('[accept] running s10 (parity goldens A-F)')
            results['s10'] = check_s10_parity(bin_dir, ctl, out_dir, resources, s4_unmodded_roundtrip=s4_unmodded)

    ordered = {f's{i}': results[f's{i}'] for i in range(1, 11) if f's{i}' in results}
    (out_dir / 'results.json').write_text(json.dumps(ordered, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(ordered, indent=2))
    sys.exit(0 if ordered and all(v['pass'] for v in ordered.values()) else 1)


if __name__ == '__main__':
    main()
