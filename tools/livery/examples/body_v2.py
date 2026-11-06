"""Build the Ferrari SF-26 main body in the running PhotoCraft, one named layer at a time.

Every coordinate below was read by eye from the UV template, grid renders and the
reference photo. The right side is written out; the left side is the same design
flipped about y = 5617 (its island mirror line) with stickers turned 180 degrees.
"""
import json, subprocess, sys
from PIL import Image

LOGOS = "livery/logos/png"
MIRROR = 5617
RED, WHITE = "#D8121B", "#F4F4F2"
calls = []

def run(tool, args, limit=300):
    calls.append({"tool": tool, "args": args, "limit": limit})

def cmd(id, params=None, limit=300):
    run("command_run", {"id": id, "params": params or {}}, limit)

def batch(steps, limit=300):
    run("command_batch", {"steps": steps}, limit)

def smooth(points, corners=()):
    """Catmull-Rom handles through hand-picked anchors; indexes in `corners` stay sharp."""
    knots = []
    n = len(points)
    for i, p in enumerate(points):
        if i in corners:
            knots.append({"anchor": list(p), "in": list(p), "out": list(p), "smooth": False})
            continue
        a, b = points[(i - 1) % n], points[(i + 1) % n]
        t = [(b[0] - a[0]) / 6, (b[1] - a[1]) / 6]
        knots.append({"anchor": list(p), "in": [p[0] - t[0], p[1] - t[1]], "out": [p[0] + t[0], p[1] + t[1]], "smooth": True})
    return knots

def mirror(points):
    return [(x, MIRROR - y) for x, y in points]

def shape(name, points, fill=None, stroke=None, closed=True, corners=()):
    params = {"kind": "path", "name": name, "fill": fill,
              "path": {"subpaths": [{"closed": closed, "knots": smooth(points, corners)}]}}
    if stroke:
        params["stroke"] = stroke
    cmd("shape.create", params)

def island_fill(mask, name, colour=None, pattern=None, poly=None):
    steps = [{"id": "layer.select", "params": {"name": f"mask {mask}"}},
             {"id": "select.magicWand", "params": {"x": 2, "y": 2, "tolerance": 8, "contiguous": False, "sampleAllLayers": False, "mode": "replace"}},
             {"id": "select.inverse", "params": {}}]
    if poly:
        steps.append({"id": "select.lasso", "params": {"points": poly, "mode": "intersect"}})
    steps.append({"id": "layer.select", "params": {"name": "base red"}})
    if pattern:
        steps.append({"id": "layer.newFillLayer.pattern", "params": {"pattern": pattern, "scale": 60}})
    else:
        steps.append({"id": "layer.newFillLayer.solidColor", "params": {"color": colour}})
    steps.append({"id": "layer.setProps", "params": {"name": name}})
    batch(steps)

def sticker(name, logo, x, y, width, rot):
    w, _ = Image.open(f"{LOGOS}/{logo}.png").size
    scale = width / w * 100
    run("doc_open", {"path": f"logos/png/{logo}.png"})
    cmd("layer.copyToDocument", {"document": 0, "source": 1, "at": [x, y], "name": name})
    cmd("file.close", {"document": 1, "save": "discard"})
    cmd("document.activate", {"document": 0})
    batch([{"id": "layer.select", "params": {"name": name}},
           {"id": "edit.transform", "params": {"scaleX": scale, "scaleY": scale, "rotate": rot, "reference": "center"}},
           {"id": "layer.arrange.bringToFront", "params": {}}])

# ---- document, base and carbon pattern
cmd("file.closeAll")
run("doc_open", {"path": "ref/carbon-tile.png"})
cmd("edit.definePattern", {"name": "carbon twill"})
cmd("file.close", {"document": 0, "save": "discard"})
run("doc_open", {"path": "ref/uv_main_body.png"})
batch([{"id": "layer.newFillLayer.solidColor", "params": {"color": RED}},
       {"id": "layer.setProps", "params": {"name": "base red"}}])
for m in ["cockpit-rim", "halo-front", "halo-left", "halo-right", "sidepod-left", "nose-side-left", "sidepod-right", "nose-side-right"]:
    run("doc_open", {"path": f"ref/masks/{m}.png"})
    cmd("layer.copyToDocument", {"document": 0, "source": 1, "name": f"mask {m}"})
    cmd("file.close", {"document": 1, "save": "discard"})
    cmd("document.activate", {"document": 0})

# ---- carbon: lower sidepods and chassis, and the halo
LOW_R = [[0, 3807], [1024, 3817], [2048, 3796], [2662, 3694], [3277, 3755], [3994, 3899], [4096, 4096], [0, 4096]]
LOW_L = [[x, MIRROR - y] for x, y in LOW_R]
for side, poly in (("right", LOW_R), ("left", LOW_L)):
    island_fill(f"sidepod-{side}", f"carbon lower sidepod {side}", pattern="carbon twill", poly=poly)
    island_fill(f"nose-side-{side}", f"carbon lower chassis {side}", pattern="carbon twill", poly=poly)
for m in ("halo-front", "halo-left", "halo-right"):
    island_fill(m, f"carbon {m}", pattern="carbon twill")
island_fill("cockpit-rim", "white cockpit surround", colour=WHITE)

# ---- white upper body (vector), stepped rear edge, pinstripe
UPPER_R = [(2700, 3000), (2700, 3395), (2420, 3412), (2150, 3430), (1950, 3478), (1829, 3585), (1523, 3605),
           (1150, 3590), (1040, 3555), (936, 3450), (835, 3279), (835, 3000)]
CORNERS = (0, 1, 11)
shape("white upper body right", UPPER_R, fill=WHITE, corners=CORNERS)
shape("white upper body left", mirror(UPPER_R), fill=WHITE, corners=CORNERS)
for side, flip in (("right", lambda p: p), ("left", lambda p: (p[0], MIRROR - p[1]))):
    for i in range(7):
        t = i / 6
        x = 1040 + (835 - 1040) * t
        y = 3555 + (3279 - 3555) * t
        for offset, every in ((58, 1), (112, 2)):
            if i % every:
                continue
            cx, cy = flip((x - offset, y))
            size = 38 if offset == 58 else 28
            cmd("shape.create", {"kind": "rect", "name": f"step {side} {i} {offset}",
                                 "rect": [cx - size / 2, cy - size / 2, size, size], "fill": WHITE})
PIN_R = [(1950, 3506), (1829, 3613), (1523, 3633), (1150, 3618), (1030, 3582), (915, 3470), (808, 3285)]
stroke = {"width": 9, "color": WHITE, "align": "center", "cap": "round", "join": "round"}
shape("pinstripe right", PIN_R, fill=None, stroke=stroke, closed=False)
shape("pinstripe left", mirror(PIN_R), fill=None, stroke=stroke, closed=False)

# ---- stickers: right side positions from the reference photo; left mirrored
SIDE = [
    ("puma", "puma-white", 1795, 3621, 150), ("shell", "shell", 1525, 3680, 240),
    ("ceva", "ceva-white", 1114, 3660, 290), ("unicredit", "unicredit2-white", 717, 3470, 410),
    ("hp", "hp2025", 1227, 3263, 220), ("ibm", "ibm-dark", 1454, 3507, 150),
    ("zyn", "zyn-red", 1681, 3517, 140), ("richardmille", "richardmille-white", 2719, 3615, 380),
    ("shield", "shield", 2418, 3605, 85),
    ("number 16", "num16", 646, 3350, 270), ("number 44", "num44", 646, 3350, 300),
]
for name, logo, x, y, w in SIDE:
    sticker(f"R {name}", logo, x, y, w, 0)
    sticker(f"L {name}", logo, x, MIRROR - y, w, 180)
NOSE = [("bitdefender", "bitdefender-white", 2620, 230), ("zyn", "zyn-white", 2780, 230),
        ("number 16", "num16", 2960, 230), ("number 44", "num44", 2960, 240),
        ("shell", "shell", 3140, 220), ("ibm", "ibm-white", 3300, 190), ("whoop", "whoop-white", 3430, 165),
        ("pirelli", "pirelli-white", 3540, 150), ("genesys", "genesys-white", 3650, 130),
        ("unicredit", "unicredit2-white", 3790, 105)]
for name, logo, x, w in NOSE:
    sticker(f"N {name}", logo, x, 2803, w, -90)
sticker("N peroni", "peroni-red", 2390, 2803, 200, 90)
sticker("H dxc right", "dxc-white", 743, 725, 420, -90)
sticker("H dxc left", "dxc-white", 2896, 725, 420, 90)

# ---- hide guides; 44 hidden by default (16 export first)
cmd("layer.list", {"flat": True}, limit=0)
run("doc_save", {"path": "out/ferrari_main_body_v2.pcraft"})
json.dump(calls, open("body_v2.json", "w"))
result = subprocess.run(["python3", "-I", "pc.py", json.dumps(calls)], capture_output=True, text=True)
print(result.stdout)
if result.returncode:
    print(result.stderr, file=sys.stderr)
    raise SystemExit(result.returncode)
print(len(calls), "calls")
