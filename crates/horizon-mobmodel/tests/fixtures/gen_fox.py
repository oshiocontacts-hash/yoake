"""Generates test_fox.bbmodel and test_fox.geo.json from one description of the same original test fox.

The bbmodel is written in Blockbench space; the geo.json is written by the documented Bedrock conversion
(x -> -x for positions, rotation (x, y, z) -> (-x, -y, z)). Run: python gen_fox.py  (writes next to this file).
Original assets only: a fox made of boxes, with a generated 8x8 texture.
"""
import base64, json, os, struct, zlib

HERE = os.path.dirname(os.path.abspath(__file__))
n0 = lambda x: 0.0 - x  # negate without producing -0.0


def png8():
    w = h = 8
    raw = b""
    for y in range(h):
        raw += b"\x00"
        for x in range(w):
            raw += bytes([(x * 32) % 256, (y * 32) % 256, 128, 255])

    def chunk(t, d):
        c = struct.pack(">I", len(d)) + t + d
        return c + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)

    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)) + chunk(
        b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")


def cube(name, frm, to, uv=None, origin=None, rot=None, inflate=0.0, mirror=False, faces=None):
    return dict(name=name, frm=frm, to=to, uv=uv, origin=origin, rot=rot, inflate=inflate, mirror=mirror, faces=faces)


# faces: name -> (uv x1,y1,x2,y2, rotation) or None (not drawn)
tip_faces = {
    "north": (0, 0, 2, 2, 0), "east": (2, 0, 4, 2, 90), "south": (4, 0, 6, 2, 180),
    "west": (8, 2, 6, 0, 0), "up": (0, 4, 2, 6, 270), "down": None,
}

GROUPS = [
    dict(name="body", parent=None, origin=[0, 10, 0], rot=[0, 0, 0], cubes=[
        cube("torso", [-3, 8, -5], [3, 13, 5], uv=[0, 0]),
        cube("fin", [-0.5, 13, -2], [0.5, 16, 2], uv=[28, 0], origin=[0, 13, 0], rot=[0, 0, 45]),
    ], locators=[("saddle", [0, 13.5, 0])]),
    dict(name="head", parent="body", origin=[0, 11, -5], rot=[10, 0, 0], cubes=[
        cube("skull", [-3, 9, -10], [3, 14, -5], uv=[0, 15]),
        cube("snout", [-1.5, 9, -12], [1.5, 11, -10], uv=[22, 15], inflate=0.1),
    ], locators=[("mouth", [0, 9.5, -11.5])],
        mesh=dict(name="whisker", origin=[3, 10, -10],
                  verts=[[0, 0, 0], [3, 0, 0], [3, 1, 0], [0, 1, 0]],
                  uvs=[[0, 60], [4, 60], [4, 56], [0, 56]])),
    dict(name="ear_l", parent="head", origin=[2, 14, -6], rot=[0, 0, -15], cubes=[
        cube("ear", [1, 14, -7], [3, 17, -6], uv=[32, 15])]),
    dict(name="ear_r", parent="head", origin=[-2, 14, -6], rot=[0, 0, 15], cubes=[
        cube("ear", [-3, 14, -7], [-1, 17, -6], uv=[32, 15], mirror=True)]),
    dict(name="tail", parent="body", origin=[0, 10, 5], rot=[20, 0, 10], cubes=[
        cube("tail", [-1.5, 8.5, 5], [1.5, 11.5, 13], uv=[0, 30])]),
    dict(name="tail_tip", parent="tail", origin=[0, 10, 13], rot=[0, 15, 0], geo_bone_inflate=0.05, cubes=[
        cube("tip", [-1, 9, 13], [1, 11, 15], faces=tip_faces, inflate=0.05)]),
    dict(name="leg_fl", parent="body", origin=[2, 8, -4], rot=[0, 0, 0], cubes=[
        cube("leg", [1, 0, -5], [3, 8, -3], uv=[0, 45])]),
    dict(name="leg_fr", parent="body", origin=[-2, 8, -4], rot=[0, 0, 0], geo_bone_mirror=True, cubes=[
        cube("leg", [-3, 0, -5], [-1, 8, -3], uv=[0, 45], mirror=True)]),
    dict(name="leg_bl", parent="body", origin=[2, 8, 4], rot=[5, 0, 0], cubes=[
        cube("leg", [1, 0, 3], [3, 8, 5], uv=[0, 45])]),
    dict(name="leg_br", parent="body", origin=[-2, 8, 4], rot=[5, 0, 0], cubes=[
        cube("leg", [-3, 0, 3], [-1, 8, 5], uv=[0, 45], mirror=True)]),
]


def uuid(prefix, i):
    return "%s-%04d" % (prefix, i)


def build_bb():
    elements, outliner_by_name, order = [], {}, []
    n = 0
    for g in GROUPS:
        children = []
        for c in g["cubes"]:
            n += 1
            e = {"type": "cube", "name": c["name"], "uuid": uuid("cube", n), "from": c["frm"], "to": c["to"],
                 "origin": c["origin"] if c["origin"] else [0, 0, 0],
                 "rotation": c["rot"] if c["rot"] else [0, 0, 0], "inflate": c["inflate"] or 0,
                 "mirror_uv": c["mirror"]}
            if c["faces"] is None:
                e["box_uv"] = True
                e["uv_offset"] = c["uv"]
                e["faces"] = {f: {"uv": [0, 0, 0, 0], "texture": 0} for f in
                              ["north", "east", "south", "west", "up", "down"]}
            else:
                e["box_uv"] = False
                e["faces"] = {}
                for f in ["north", "east", "south", "west", "up", "down"]:
                    v = c["faces"].get(f)
                    e["faces"][f] = {"uv": [0, 0, 0, 0], "texture": None} if v is None else {
                        "uv": list(v[:4]), "texture": 0, "rotation": v[4]}
            elements.append(e)
            children.append(e["uuid"])
        for name, pos in g.get("locators", []):
            n += 1
            e = {"type": "locator", "name": name, "uuid": uuid("loc", n), "from": pos, "rotation": [0, 0, 0]}
            elements.append(e)
            children.append(e["uuid"])
        if "mesh" in g:
            m = g["mesh"]
            n += 1
            ids = ["v%d" % i for i in range(len(m["verts"]))]
            e = {"type": "mesh", "name": m["name"], "uuid": uuid("mesh", n), "origin": m["origin"],
                 "rotation": [0, 0, 0],
                 "vertices": {i: v for i, v in zip(ids, m["verts"])},
                 "faces": {"f0": {"vertices": ids, "uv": {i: u for i, u in zip(ids, m["uvs"])}, "texture": 0}}}
            elements.append(e)
            children.append(e["uuid"])
        node = {"name": g["name"], "origin": g["origin"], "rotation": g["rot"], "uuid": uuid("grp", len(order) + 1),
                "children": children}
        outliner_by_name[g["name"]] = node
        order.append(g)
    root = []
    for g in GROUPS:
        node = outliner_by_name[g["name"]]
        if g["parent"] is None:
            root.append(node)
        else:
            outliner_by_name[g["parent"]]["children"].append(node)
    return {
        "meta": {"format_version": "4.10", "model_format": "bedrock", "box_uv": True},
        "name": "test_fox", "model_identifier": "geometry.test_fox",
        "resolution": {"width": 64, "height": 64},
        "elements": elements, "outliner": root,
        "textures": [{"name": "test_fox.png", "path": "test_fox.png", "id": "0",
                      "source": "data:image/png;base64," + base64.b64encode(png8()).decode()}],
        "animations": [{"uuid": "anim-0001", "name": "idle", "loop": "loop", "length": 1.0, "animators": {}}],
    }


def conv_pos(p):
    return [n0(p[0]), p[1], p[2]]


def conv_rot(r):
    return [n0(r[0]), n0(r[1]), r[2]]


def build_geo():
    bones = []
    for g in GROUPS:
        b = {"name": g["name"], "pivot": conv_pos(g["origin"])}
        if g["parent"]:
            b["parent"] = g["parent"]
        if any(g["rot"]):
            b["rotation"] = conv_rot(g["rot"])
        if g.get("geo_bone_mirror"):
            b["mirror"] = True
        if g.get("geo_bone_inflate"):
            b["inflate"] = g["geo_bone_inflate"]
        cubes = []
        for c in g["cubes"]:
            frm, to = c["frm"], c["to"]
            o = {"origin": [n0(to[0]), frm[1], frm[2]], "size": [to[0] - frm[0], to[1] - frm[1], to[2] - frm[2]]}
            if c["rot"]:
                o["pivot"] = conv_pos(c["origin"])
                o["rotation"] = conv_rot(c["rot"])
            if c["inflate"] and not g.get("geo_bone_inflate"):
                o["inflate"] = c["inflate"]
            if c["mirror"] and not g.get("geo_bone_mirror"):
                o["mirror"] = True
            if c["faces"] is None:
                o["uv"] = c["uv"]
            else:
                o["uv"] = {}
                for f, v in c["faces"].items():
                    if v is None:
                        continue
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
        if "mesh" in g:
            m = g["mesh"]
            absolute = [[m["origin"][i] + v[i] for i in range(3)] for v in m["verts"]]
            k = len(absolute)
            b["poly_mesh"] = {
                "normalized_uvs": False,
                "positions": [conv_pos(p) for p in absolute],
                "uvs": m["uvs"],
                "polys": [[[i, 0, i] for i in reversed(range(k))]],
            }
        bones.append(b)
    return {
        "format_version": "1.12.0",
        "minecraft:geometry": [{
            "description": {"identifier": "geometry.test_fox", "texture_width": 64, "texture_height": 64,
                            "visible_bounds_width": 4, "visible_bounds_height": 3,
                            "visible_bounds_offset": [0, 1, 0]},
            "bones": bones}],
    }


if __name__ == "__main__":
    with open(os.path.join(HERE, "test_fox.bbmodel"), "w", newline="\n") as f:
        json.dump(build_bb(), f, indent=1)
        f.write("\n")
    with open(os.path.join(HERE, "test_fox.geo.json"), "w", newline="\n") as f:
        json.dump(build_geo(), f, indent=1)
        f.write("\n")
