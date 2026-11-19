"""Generates the "Cinder Fox" fixture set (FR-F4-MOBS.md section 10) from ONE description:

    cinder_fox.bbmodel            Blockbench space, texture embedded, animations[] in Blockbench form
    cinder_fox.geo.json           Bedrock geometry (documented conversion: x -> -x, rotation (x, y, z) -> (-x, -y, z))
    cinder_fox.animation.json     Bedrock animations
    cinder_fox.json               animation controller (our section 6.1 format)
    cinder_fox.png                64x64 texture
    cinder_fox_glowmask.png       64x64 emissive mask (eyes and tail tip)

Run:  python gen_cinder_fox.py      (writes next to this file; deterministic, byte-identical on every run)

Original work only: a fox made of boxes, hand-written keyframes and expressions, generated pixels. No GeckoLib or
Minecraft asset was used or copied.

Animations are written once in Bedrock space (what animation.json holds). The bbmodel copy is derived by the inverse
of the loader's conversion (position x, rotation x and y negated; expression results wrapped in -( ... )), so both
routes should evaluate to the same pose. Known limits (see README.md and the F4M-FOX report): Blockbench's bbmodel
keyframe has no easing / easingArgs, so the bbmodel copy drops them (the bbmodel pose differs on exactly those leaf bones).
"""
import base64, json, os, struct, zlib

HERE = os.path.dirname(os.path.abspath(__file__))
n0 = lambda x: 0.0 - x  # negate without producing -0.0
FACES = ["north", "east", "south", "west", "up", "down"]

# ----------------------------------------------------------------------------------------------------------------
# Model description (Blockbench space, pixels; the fox looks towards -Z)
# ----------------------------------------------------------------------------------------------------------------


def box(name, frm, to, uv, origin=None, rot=None, mirror=False):
    return dict(name=name, frm=frm, to=to, uv=uv, origin=origin, rot=rot, mirror=mirror, faces=None)


def per_face(name, frm, to, faces, origin=None, rot=None):
    return dict(name=name, frm=frm, to=to, uv=None, origin=origin, rot=rot, mirror=False, faces=faces)


# per-face rectangles [x1, y1, x2, y2] (+ uv rotation) in texels
SNOUT_FACES = {
    "north": (0, 40, 3, 42, 0), "south": (3, 40, 6, 42, 0), "east": (6, 40, 9, 42, 0), "west": (9, 40, 12, 42, 0),
    "up": (12, 40, 15, 43, 0), "down": (15, 40, 18, 43, 0),
}
TIP_FACES = {
    "north": (44, 16, 47, 19, 0), "east": (47, 16, 50, 19, 90), "south": (50, 16, 53, 19, 0),
    "west": (53, 16, 56, 19, 0), "up": (44, 19, 47, 22, 0), "down": (47, 19, 50, 22, 0),
}

# Depth-first order, parents first (both loaders then give the same bone indices).
GROUPS = [
    dict(name="root", parent=None, origin=[0, 0, 0], rot=[0, 0, 0], cubes=[]),
    dict(name="body", parent="root", origin=[0, 5.5, 0], rot=[0, 0, 0], cubes=[
        box("torso", [-3, 3, -5], [3, 8, 5], [0, 0])]),
    dict(name="head", parent="body", origin=[0, 6, -4], rot=[0, 0, 0], cubes=[
        box("skull", [-3, 5, -9], [3, 10, -4], [0, 16]),
        box("tuft", [-1, 10, -7], [1, 12, -5], [34, 0], origin=[0, 11, -6], rot=[0, 0, 45])],
        locators=[("mouth", [0, 6.5, -12.5])]),
    dict(name="ear_l", parent="head", origin=[2, 10, -5.5], rot=[0, 0, -10], cubes=[
        box("ear", [1, 10, -6], [3, 13, -5], [44, 0])]),
    dict(name="ear_r", parent="head", origin=[-2, 10, -5.5], rot=[0, 0, 10], cubes=[
        box("ear", [-3, 10, -6], [-1, 13, -5], [44, 0], mirror=True)]),
    dict(name="snout", parent="head", origin=[0, 8, -9], rot=[0, 0, 0], cubes=[
        per_face("snout", [-1.5, 7, -12], [1.5, 9, -9], SNOUT_FACES)]),
    dict(name="jaw", parent="head", origin=[0, 6.5, -9], rot=[0, 0, 0], cubes=[
        box("jaw", [-1.5, 6, -12], [1.5, 7, -9], [52, 0])]),
    dict(name="leg_fl", parent="body", origin=[2, 3, -4], rot=[0, 0, 0], cubes=[
        box("leg", [1, 0, -5], [3, 3, -3], [0, 28])]),
    dict(name="leg_fr", parent="body", origin=[-2, 3, -4], rot=[0, 0, 0], cubes=[
        box("leg", [-3, 0, -5], [-1, 3, -3], [0, 28], mirror=True)]),
    dict(name="leg_bl", parent="body", origin=[2, 3, 4], rot=[0, 0, 0], cubes=[
        box("leg", [1, 0, 3], [3, 3, 5], [10, 28])]),
    dict(name="leg_br", parent="body", origin=[-2, 3, 4], rot=[0, 0, 0], cubes=[
        box("leg", [-3, 0, 3], [-1, 3, 5], [10, 28], mirror=True)]),
    dict(name="tail1", parent="body", origin=[0, 6.5, 5], rot=[-25, 0, 0], cubes=[
        box("tail", [-1.5, 5, 5], [1.5, 8, 9], [24, 16])]),
    dict(name="tail2", parent="tail1", origin=[0, 6.5, 9], rot=[-10, 0, 0], cubes=[
        box("tail", [-2, 4.5, 9], [2, 8.5, 13], [24, 28])]),
    dict(name="tail3", parent="tail2", origin=[0, 6.5, 13], rot=[-5, 0, 5], cubes=[
        per_face("tip", [-1.5, 5, 13], [1.5, 8, 16], TIP_FACES)]),
]
assert len(GROUPS) == 14


def conv_pos(p):
    return [n0(p[0]), p[1], p[2]]


def conv_rot(r):
    return [n0(r[0]), n0(r[1]), r[2]]


def uuid(prefix, i):
    return "%s-%04d" % (prefix, i)


# ----------------------------------------------------------------------------------------------------------------
# Texture (64x64), generated pixels
# ----------------------------------------------------------------------------------------------------------------

def png_bytes(w, h, rgba):
    raw = b""
    for y in range(h):
        raw += b"\x00" + bytes(c for px in rgba[y * w:(y + 1) * w] for c in px)

    def chunk(t, d):
        c = struct.pack(">I", len(d)) + t + d
        return c + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)

    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)) + chunk(
        b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


def box_rects(u, v, w, h, d):
    """Face rectangles (x1, y1, x2, y2) of the classic box layout (docs: FR-F4M-1 convention 5)."""
    return {
        "east": (u, v + d, u + d, v + d + h), "north": (u + d, v + d, u + d + w, v + d + h),
        "west": (u + d + w, v + d, u + 2 * d + w, v + d + h), "south": (u + 2 * d + w, v + d, u + 2 * d + 2 * w, v + d + h),
        "up": (u + d, v, u + d + w, v + d), "down": (u + d + w, v, u + d + 2 * w, v + d),
    }


CHAR, CHAR2, EMBER, EMBER2, CREAM, EYE = (50, 42, 46), (68, 58, 60), (214, 96, 34), (255, 168, 52), (226, 204, 170), (255, 214, 64)


def noise(x, y):
    return ((x * 73856093) ^ (y * 19349663) ^ (x * y * 83492791)) % 9 - 4


def shade(c, k):
    return tuple(max(0, min(255, c[i] + k)) for i in range(3))


def build_textures():
    W = H = 64
    tex = [(0, 0, 0, 0)] * (W * H)
    glow = [(0, 0, 0, 0)] * (W * H)
    claimed = {}

    def put(x, y, c, g=None):
        tex[y * W + x] = (c[0], c[1], c[2], 255)
        if g:
            glow[y * W + x] = (g[0], g[1], g[2], 255)

    def rect(r, who, fn):
        x1, y1, x2, y2 = r
        for y in range(y1, y2):
            for x in range(x1, x2):
                assert (x, y) not in claimed, "UV overlap %s with %s at %d,%d" % (who, claimed[(x, y)], x, y)
                claimed[(x, y)] = who
                c, g = fn(x - x1, y - y1, x, y)
                put(x, y, c, g)

    def fur(base):
        return lambda lx, ly, x, y: (shade(base, noise(x, y)), None)

    # body: charcoal with ember stripes on the back, cream belly
    for face, r in box_rects(0, 0, 6, 5, 10).items():
        if face == "down":
            rect(r, "body." + face, lambda lx, ly, x, y: (shade(CREAM, noise(x, y)), None))
        elif face == "up":
            rect(r, "body." + face, lambda lx, ly, x, y: (shade(EMBER, noise(x, y)) if lx % 3 == 1 else shade(CHAR, noise(x, y)), None))
        else:
            rect(r, "body." + face, lambda lx, ly, x, y: (shade(CHAR2, noise(x, y)) if ly > 3 else shade(CHAR, noise(x, y)), None))
    # head: the eyes on the north (front) face glow
    for face, r in box_rects(0, 16, 6, 5, 5).items():
        if face == "north":
            def eyes(lx, ly, x, y):
                if (lx, ly) in ((1, 1), (4, 1)):
                    return EYE, EYE
                if ly >= 3:
                    return shade(CREAM, noise(x, y)), None
                return shade(CHAR, noise(x, y)), None
            rect(r, "head.north", eyes)
        else:
            rect(r, "head." + face, fur(CHAR))
    for face, r in box_rects(34, 0, 2, 2, 2).items():
        rect(r, "tuft." + face, fur(EMBER))
    for face, r in box_rects(44, 0, 2, 3, 1).items():
        rect(r, "ear." + face, fur(CHAR2) if face != "north" else (lambda lx, ly, x, y: (EMBER, None)))
    for face, r in box_rects(52, 0, 3, 1, 3).items():
        rect(r, "jaw." + face, fur(CREAM))
    for off, who in (((0, 28), "leg_f"), ((10, 28), "leg_b")):
        for face, r in box_rects(off[0], off[1], 2, 3, 2).items():
            rect(r, who + "." + face, lambda lx, ly, x, y: (shade(CHAR, noise(x, y)) if ly < 2 else shade(CREAM, 0), None))
    for face, r in box_rects(24, 16, 3, 3, 4).items():
        rect(r, "tail1." + face, fur(CHAR))
    for face, r in box_rects(24, 28, 4, 4, 4).items():
        rect(r, "tail2." + face, lambda lx, ly, x, y: (shade(EMBER, noise(x, y)) if (lx + ly) % 3 == 0 else shade(CHAR, noise(x, y)), None))
    for face, v in SNOUT_FACES.items():
        rect(v[:4], "snout." + face, fur(CHAR2 if face != "north" else CREAM))
    for face, v in TIP_FACES.items():
        rect(v[:4], "tip." + face, lambda lx, ly, x, y: (shade(EMBER2, noise(x, y)), EMBER2))
    return png_bytes(W, H, tex), png_bytes(W, H, glow)


# ----------------------------------------------------------------------------------------------------------------
# Geometry files
# ----------------------------------------------------------------------------------------------------------------

def build_bb(tex_png, anims):
    elements, nodes, n = [], {}, 0
    for gi, g in enumerate(GROUPS):
        children = []
        for c in g["cubes"]:
            n += 1
            e = {"type": "cube", "name": c["name"], "uuid": uuid("cube", n), "from": c["frm"], "to": c["to"],
                 "origin": c["origin"] if c["origin"] else [0, 0, 0], "rotation": c["rot"] if c["rot"] else [0, 0, 0],
                 "inflate": 0, "mirror_uv": c["mirror"]}
            if c["faces"] is None:
                e["box_uv"] = True
                e["uv_offset"] = c["uv"]
                e["faces"] = {f: {"uv": [0, 0, 0, 0], "texture": 0} for f in FACES}
            else:
                e["box_uv"] = False
                e["faces"] = {f: {"uv": list(c["faces"][f][:4]), "texture": 0, "rotation": c["faces"][f][4]} for f in FACES}
            elements.append(e)
            children.append(e["uuid"])
        for name, pos in g.get("locators", []):
            n += 1
            e = {"type": "locator", "name": name, "uuid": uuid("loc", n), "from": pos, "rotation": [0, 0, 0]}
            elements.append(e)
            children.append(e["uuid"])
        nodes[g["name"]] = {"name": g["name"], "origin": g["origin"], "rotation": g["rot"], "uuid": uuid("grp", gi + 1),
                            "children": children}
    root = []
    for g in GROUPS:
        (root if g["parent"] is None else nodes[g["parent"]]["children"]).append(nodes[g["name"]])
    return {
        "meta": {"format_version": "4.10", "model_format": "bedrock", "box_uv": True},
        "name": "cinder_fox", "model_identifier": "geometry.cinder_fox", "resolution": {"width": 64, "height": 64},
        "elements": elements, "outliner": root,
        "textures": [{"name": "cinder_fox.png", "path": "cinder_fox.png", "id": "0",
                      "source": "data:image/png;base64," + base64.b64encode(tex_png).decode()}],
        "animations": anims,
    }


def build_geo():
    bones = []
    for g in GROUPS:
        b = {"name": g["name"], "pivot": conv_pos(g["origin"])}
        if g["parent"]:
            b["parent"] = g["parent"]
        if any(g["rot"]):
            b["rotation"] = conv_rot(g["rot"])
        cubes = []
        for c in g["cubes"]:
            frm, to = c["frm"], c["to"]
            o = {"origin": [n0(to[0]), frm[1], frm[2]], "size": [to[0] - frm[0], to[1] - frm[1], to[2] - frm[2]]}
            if c["rot"]:
                o["pivot"] = conv_pos(c["origin"])
                o["rotation"] = conv_rot(c["rot"])
            if c["mirror"]:
                o["mirror"] = True
            if c["faces"] is None:
                o["uv"] = c["uv"]
            else:
                o["uv"] = {}
                for f in FACES:
                    v = c["faces"][f]
                    face = {"uv": [v[0], v[1]], "uv_size": [v[2] - v[0], v[3] - v[1]]}
                    if f in ("up", "down"):  # Blockbench's Bedrock export flips both axes
                        face = {"uv": [v[2], v[3]], "uv_size": [v[0] - v[2], v[1] - v[3]]}
                    if v[4]:
                        face["uv_rotation"] = v[4]
                    o["uv"][f] = face
            cubes.append(o)
        if cubes:
            b["cubes"] = cubes
        if g.get("locators"):
            b["locators"] = {name: conv_pos(pos) for name, pos in g["locators"]}
        bones.append(b)
    return {
        "format_version": "1.12.0",
        "minecraft:geometry": [{
            "description": {"identifier": "geometry.cinder_fox", "texture_width": 64, "texture_height": 64,
                            "visible_bounds_width": 3, "visible_bounds_height": 2, "visible_bounds_offset": [0, 0.75, 0]},
            "bones": bones}],
    }


# ----------------------------------------------------------------------------------------------------------------
# Animations, written in Bedrock space
# ----------------------------------------------------------------------------------------------------------------

class V(tuple):
    """A constant (not keyed) channel value."""
    def __new__(cls, a, b, c):
        return tuple.__new__(cls, (a, b, c))


class K:
    def __init__(self, t, post, pre=None, lerp=None, ease=None, ease_args=None, bz=None):
        self.t, self.post, self.pre, self.lerp, self.ease, self.ease_args, self.bz = t, list(post), pre, lerp, ease, ease_args, bz


def bz(lt, lv, rt, rv):
    return dict(bezier_left_time=lt, bezier_left_value=lv, bezier_right_time=rt, bezier_right_value=rv)


def A(name, loop, length, bones, atu=None, timeline=(), sounds=(), particles=()):
    return dict(name="animation.cinder_fox." + name, loop=loop, length=length, bones=bones, atu=atu,
                timeline=list(timeline), sounds=list(sounds), particles=list(particles))


SIN = lambda k: "math.sin(q.anim_time * %s)" % k
GS = "math.clamp(q.ground_speed * 100, 25, 50)"

ANIMS = [
    A("idle", True, 3.0, {
        "body": {"position": V(0, SIN(120) + " * 0.15", 0),
                 "scale": [K(0, [1, 1, 1], lerp="catmullrom"), K(1.5, [1.02, 1.04, 1.02], lerp="catmullrom"), K(3, [1, 1, 1])]},
        "head": {"rotation": [K(0, [0, 0, 0], lerp="catmullrom"), K(1, [2, 8, 0], lerp="catmullrom"),
                              K(2, [-2, -6, 1], lerp="catmullrom"), K(3, [0, 0, 0])]},
        "tail1": {"rotation": V(SIN(120) + " * 3", SIN(240) + " * 8", 0)},
        "tail2": {"rotation": V(0, "math.sin(q.anim_time * 240 - 40) * 10", "math.cos(q.anim_time * 120) * 3")},
        "ear_l": {"rotation": [K(0, [0, 0, 0], lerp="step"), K(1.2, [0, 0, -14], lerp="step"), K(1.3, [0, 0, 0]), K(3, [0, 0, 0])]},
    }),
    # Speed from ground speed through an expression: anim_time_update (stateless) scales the animation clock.
    A("walk", True, 0.8, {
        "leg_fl": {"rotation": V(SIN(450) + " * 30", 0, 0)},
        "leg_br": {"rotation": V(SIN(450) + " * 30", 0, 0)},
        "leg_fr": {"rotation": V("math.sin(q.anim_time * 450 + 180) * 30", 0, 0)},
        "leg_bl": {"rotation": V("math.sin(q.anim_time * 450 + 180) * 30", 0, 0)},
        "body": {"position": [K(0, [0, 0, 0], lerp="catmullrom"), K(0.2, [0, 0.5, 0], lerp="catmullrom"),
                              K(0.4, [0, 0, 0], lerp="catmullrom"), K(0.6, [0, 0.5, 0], lerp="catmullrom"), K(0.8, [0, 0, 0])],
                 "rotation": V(SIN(900) + " * 1.5", SIN(450) + " * math.clamp(q.ground_speed * 30, 2, 6)", 0)},
        "head": {"position": [K(0, [0, 0, 0], lerp="bezier", bz=bz([-0.1] * 3, [0, 0, 0], [0.12] * 3, [0.2, -0.4, 0.2])),
                              K(0.4, [0, -0.6, 0], lerp="bezier", bz=bz([-0.12] * 3, [-0.2, 0.3, 0.1], [0.1] * 3, [0.2, 0.3, -0.1])),
                              K(0.8, [0, 0, 0], bz=bz([-0.1] * 3, [0, 0, 0], [0.1] * 3, [0, 0, 0]))]},
        "tail1": {"rotation": V("math.sin(q.anim_time * 450 + 40) * 5", SIN(450) + " * 8", 0)},
        "tail3": {"rotation": V(0, "math.sin(q.anim_time * 450 - 40) * 12", 0)},
    }, atu="q.anim_time * (0.7 + math.clamp(q.ground_speed, 0, 0.5) * 2.4)"),
    A("run", True, 0.5, {
        "leg_fl": {"rotation": V("math.sin(q.anim_time * 720) * " + GS, 0, 0)},
        "leg_fr": {"rotation": V("math.sin(q.anim_time * 720 + 30) * " + GS, 0, 0)},
        "leg_bl": {"rotation": V("math.sin(q.anim_time * 720 + 160) * " + GS, 0, 0)},
        "leg_br": {"rotation": V("math.sin(q.anim_time * 720 + 190) * " + GS, 0, 0)},
        "body": {"position": V(0, "math.abs(math.sin(q.anim_time * 720)) * 1.2", 0),
                 "rotation": V("math.sin(q.anim_time * 1440 + 20) * 4", 0, 0),
                 "scale": V(1, 1, "1 + math.sin(q.anim_time * 720) * 0.03")},
        "head": {"rotation": V(-8, 0, 0)},
        "tail1": {"rotation": V("-18 + math.sin(q.anim_time * 720) * 5", 0, 0)},
        "tail2": {"rotation": V(0, "math.lerp(-12, 12, (math.sin(q.anim_time * 720) + 1) / 2)", 0)},
    }),
    A("sit", "hold", 0.6, {
        "body": {"position": [K(0, [0, 0, 0], lerp="catmullrom"), K(0.3, [0, -2.4, 1.2], lerp="catmullrom"), K(0.6, [0, -2, 1.5])],
                 "rotation": [K(0, [0, 0, 0]), K(0.6, [-18, 0, 0])]},
        "head": {"rotation": [K(0, [0, 0, 0]), K(0.6, [18, 0, 0])]},
        "leg_bl": {"rotation": [K(0, [0, 0, 0]), K(0.6, [-75, 0, 0], ease="easeOutCubic")]},
        "leg_br": {"rotation": [K(0, [0, 0, 0]), K(0.6, [-75, 0, 0], ease="easeOutCubic")]},
        "leg_fl": {"rotation": [K(0, [0, 0, 0]), K(0.6, [10, 0, 0], ease="easeInOutSine")]},
        "leg_fr": {"rotation": [K(0, [0, 0, 0]), K(0.6, [10, 0, 0], ease="easeInOutSine")]},
        "tail1": {"rotation": [K(0, [0, 0, 0]), K(0.6, [30, 20, 0])]},
        "tail2": {"rotation": [K(0, [0, 0, 0]), K(0.6, [0, 25, 0])]},
    }),
    A("pounce", False, 0.8, {
        "body": {"rotation": [K(0, [0, 0, 0], lerp="catmullrom"), K(0.2, [14, 0, 0], lerp="catmullrom"),
                              K(0.35, [-26, 0, 0], lerp="catmullrom"), K(0.6, [-6, 0, 0], lerp="catmullrom"), K(0.8, [0, 0, 0])],
                 "position": [K(0, [0, 0, 0]), K(0.2, [0, -1.6, 0.8]), K(0.35, [0, 1.6, -3.5]), K(0.6, [0, 0.4, -5]),
                              K(0.8, [0, 0, 0])]},
        "head": {"rotation": [K(0, [0, 0, 0]), K(0.2, [-10, 0, 0]), K(0.35, [-32, 0, 0], pre=[-14, 0, 0]),
                              K(0.6, [-5, 0, 0]), K(0.8, [0, 0, 0])]},
        "jaw": {"rotation": [K(0, [0, 0, 0]), K(0.3, [0, 0, 0]), K(0.35, [42, 0, 0], ease="easeOutQuad"),
                             K(0.6, [0, 0, 0], ease="easeInOutQuad"), K(0.8, [0, 0, 0])]},
        "leg_fl": {"rotation": [K(0, [0, 0, 0]), K(0.2, [-35, 0, 0]), K(0.35, [70, 0, 0], ease="easeOutBack", ease_args=[1.8]),
                                K(0.6, [-20, 0, 0]), K(0.8, [0, 0, 0])]},
        "leg_fr": {"rotation": [K(0, [0, 0, 0]), K(0.2, [-35, 0, 0]), K(0.35, [70, 0, 0], ease="easeOutBack", ease_args=[1.8]),
                                K(0.6, [-20, 0, 0]), K(0.8, [0, 0, 0])]},
        "leg_bl": {"rotation": [K(0, [0, 0, 0]), K(0.2, [35, 0, 0]), K(0.35, [-60, 0, 0], ease="easeOutElastic", ease_args=[1.3]),
                                K(0.6, [10, 0, 0]), K(0.8, [0, 0, 0])]},
        "leg_br": {"rotation": [K(0, [0, 0, 0]), K(0.2, [35, 0, 0]), K(0.35, [-60, 0, 0], ease="easeOutElastic", ease_args=[1.3]),
                                K(0.6, [10, 0, 0]), K(0.8, [0, 0, 0])]},
        "tail1": {"rotation": V("math.cos(q.anim_time * 450) * 10 - 10", 0, 0)},
        "tail3": {"rotation": [K(0, [0, 0, 0]), K(0.35, [0, 0, 0]), K(0.5, [0, 0, 30], ease="easeOutBounce"), K(0.8, [0, 0, 0])]},
    }, timeline=[(0.35, "hit")], sounds=[(0.35, "cinder_fox_pounce")], particles=[(0.35, "cinder_fox_embers", "mouth")]),
    A("death", "hold", 1.2, {
        "body": {"rotation": [K(0, [0, 0, 0], lerp="catmullrom"), K(0.5, [0, 0, 95], lerp="catmullrom"),
                              K(0.8, [0, 0, 82], lerp="catmullrom"), K(1.2, [0, 0, 90])],
                 "position": [K(0, [0, 0, 0]), K(0.5, [3, -3, 0]), K(1.2, [3, -3.5, 0])]},
        "head": {"rotation": [K(0, [0, 0, 0]), K(1.2, [10, 20, -15])]},
        "leg_fl": {"rotation": [K(0, [0, 0, 0]), K(0.6, [-40, 0, 10], ease="easeOutBounce"), K(1.2, [-40, 0, 10])]},
        "leg_fr": {"rotation": [K(0, [0, 0, 0]), K(0.6, [-55, 0, -10], ease="easeOutBack", ease_args=[2.0]), K(1.2, [-55, 0, -10])]},
        "leg_bl": {"rotation": [K(0, [0, 0, 0]), K(0.6, [30, 0, 10], ease="easeOutCubic")]},
        "leg_br": {"rotation": [K(0, [0, 0, 0]), K(0.6, [40, 0, -5], ease="easeOutCubic")]},
        "ear_l": {"rotation": [K(0, [0, 0, 0]), K(0.5, [0, 0, -35], ease="easeOutSine")]},
        "ear_r": {"rotation": [K(0, [0, 0, 0]), K(0.5, [0, 0, 35], ease="easeOutSine")]},
        "tail1": {"rotation": [K(0, [0, 0, 0]), K(1.2, [20, 0, 0])]},
        "tail3": {"scale": [K(0, [1, 1, 1]), K(1.2, [0.4, 0.4, 0.4], ease="easeInQuad")]},
    }),
]

FLIP = {"rotation": (True, True, False), "position": (True, False, False), "scale": (False, False, False)}


def tstr(t):
    return "%g" % t


def bedrock_key(k):
    if k.pre is None and k.lerp is None and k.ease is None and k.bz is None:
        return list(k.post)
    d = {}
    if k.pre is not None:
        d["pre"] = list(k.pre)
    d["post"] = list(k.post)
    if k.lerp:
        d["lerp_mode"] = k.lerp
    if k.ease:
        d["easing"] = k.ease
    if k.ease_args:
        d["easingArgs"] = list(k.ease_args)
    if k.bz:
        d.update({kk: list(vv) for kk, vv in k.bz.items()})
    return d


def build_animation_json():
    out = {}
    for a in ANIMS:
        d = {"loop": "hold_on_last_frame" if a["loop"] == "hold" else a["loop"], "animation_length": a["length"]}
        if a["atu"]:
            d["anim_time_update"] = a["atu"]
        bones = {}
        for bone, chans in a["bones"].items():
            bones[bone] = {}
            for ch, val in chans.items():
                bones[bone][ch] = list(val) if isinstance(val, V) else {tstr(k.t): bedrock_key(k) for k in val}
        d["bones"] = bones
        if a["timeline"]:
            d["timeline"] = {tstr(t): n for t, n in a["timeline"]}
        if a["sounds"]:
            d["sound_effects"] = {tstr(t): {"effect": n} for t, n in a["sounds"]}
        if a["particles"]:
            d["particle_effects"] = {tstr(t): {"effect": n, "locator": loc} for t, n, loc in a["particles"]}
        out[a["name"]] = d
    return {"format_version": "1.8.0", "animations": out}


def flip_comp(v, do):
    if not do:
        return v
    return "-(" + v + ")" if isinstance(v, str) else n0(v)


def bb_point(vec, ch):
    f = FLIP[ch]
    return {"x": flip_comp(vec[0], f[0]), "y": flip_comp(vec[1], f[1]), "z": flip_comp(vec[2], f[2])}


def build_bb_animations():
    out = []
    for ai, a in enumerate(ANIMS):
        animators = {}
        for bi, (bone, chans) in enumerate(a["bones"].items()):
            kfs = []
            for ch, val in chans.items():
                f = FLIP[ch]
                keys = [K(0, list(val))] if isinstance(val, V) else val
                for k in keys:
                    kf = {"channel": ch, "time": k.t, "color": -1, "interpolation": k.lerp or "linear"}
                    pts = [bb_point(k.post, ch)] if k.pre is None else [bb_point(k.pre, ch), bb_point(k.post, ch)]
                    kf["data_points"] = pts
                    if k.bz:
                        for name, vec in k.bz.items():
                            # value handles live in the animated space too, so they flip with it; time handles do not
                            kf[name] = [flip_comp(vec[i], f[i]) for i in range(3)] if name.endswith("value") else list(vec)
                    # Blockbench's bbmodel has no easing / easingArgs: dropped here (documented limit)
                    kfs.append(kf)
            animators["bone-%04d-%04d" % (ai, bi)] = {"name": bone, "type": "bone", "keyframes": kfs}
        effects = []
        for t, n in a["timeline"]:
            effects.append({"channel": "timeline", "time": t, "data_points": [{"script": n}]})
        for t, n in a["sounds"]:
            effects.append({"channel": "sound", "time": t, "data_points": [{"effect": n}]})
        for t, n, loc in a["particles"]:
            effects.append({"channel": "particle", "time": t, "data_points": [{"effect": n, "locator": loc}]})
        if effects:
            animators["effects-%04d" % ai] = {"name": "Effects", "type": "effect", "keyframes": effects}
        d = {"uuid": "anim-%04d" % ai, "name": a["name"],
             "loop": {True: "loop", False: "once", "hold": "hold"}[a["loop"]], "override": False,
             "length": a["length"], "animators": animators}
        if a["atu"]:
            d["anim_time_update"] = a["atu"]
        out.append(d)
    return out


# ----------------------------------------------------------------------------------------------------------------
# Controller (section 6.1 format)
# ----------------------------------------------------------------------------------------------------------------

CONTROLLER = {"controllers": {
    "locomotion": {"initial": "idle", "states": {
        "idle": {"animations": ["idle"], "blend": 0.15,
                 "transitions": [{"to": "walk", "when": "q.ground_speed > 0.05"}]},
        "walk": {"animations": ["walk"], "blend": 0.1,
                 "transitions": [{"to": "run", "when": "q.ground_speed > 0.2"},
                                 {"to": "idle", "when": "q.ground_speed < 0.03"}]},
        "run": {"animations": [{"name": "run", "speed": "math.clamp(q.ground_speed * 3, 0.8, 2)"}], "blend": 0.1,
                "transitions": [{"to": "walk", "when": "q.ground_speed < 0.17"}]},
    }},
    # One-shot on top of the locomotion pose: added, so it leaves no pop when it ends (its last key is 0).
    "action": {"weight": 1, "mode": "add", "triggers": {"pounce": "pounce"}},
    # sit while the synced enum `state` (idle=0, sit=1, pounce=2) is 1. Leaving is a cut (blend 0): see the report.
    "pose": {"initial": "stand", "states": {
        "stand": {"animations": [],
                  "transitions": [{"to": "sit", "when": "q.synced('state') == 1"}]},
        "sit": {"animations": ["sit"], "blend": 0.2,
                "transitions": [{"to": "stand", "when": "q.synced('state') != 1", "blend": 0}]},
    }},
}}


def dump(path, obj):
    with open(os.path.join(HERE, path), "w", newline="\n") as f:
        json.dump(obj, f, indent=1)
        f.write("\n")


if __name__ == "__main__":
    tex, glow = build_textures()
    with open(os.path.join(HERE, "cinder_fox.png"), "wb") as f:
        f.write(tex)
    with open(os.path.join(HERE, "cinder_fox_glowmask.png"), "wb") as f:
        f.write(glow)
    dump("cinder_fox.bbmodel", build_bb(tex, build_bb_animations()))
    dump("cinder_fox.geo.json", build_geo())
    dump("cinder_fox.animation.json", build_animation_json())
    dump("cinder_fox.json", CONTROLLER)
