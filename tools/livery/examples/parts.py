"""Build the SF-26 wings, floor, mirrors, winglets and halo-inner sheets in PhotoCraft.

Positions are read by eye from each UV template, the stock Audi sheets and the
reference photos. Rotations follow each panel's measured orientation.
"""
import json, subprocess, sys
from PIL import Image

LOGOS = "livery/logos/png"
RED, WHITE = "#D8121B", "#F4F4F2"
GREEN, FLAGRED = "#009246", "#CE2B37"

def build(part, base, fills, stickers, shapes):
    calls = []
    run = lambda tool, args, limit=300: calls.append({"tool": tool, "args": args, "limit": limit})
    cmd = lambda id, params=None: run("command_run", {"id": id, "params": params or {}})
    cmd("file.closeAll")
    run("doc_open", {"path": "ref/carbon-tile.png"})
    cmd("edit.definePattern", {"name": "carbon twill"})
    cmd("file.close", {"document": 0, "save": "discard"})
    run("doc_open", {"path": f"ref/uv_{part}.png"})
    scale = base.get("scale", 60)
    if base.get("pattern"):
        cmd("layer.newFillLayer.pattern", {"pattern": "carbon twill", "scale": scale})
    else:
        cmd("layer.newFillLayer.solidColor", {"color": base["color"]})
    cmd("layer.setProps", {"name": "base"})
    for mask, name, colour in fills:
        run("doc_open", {"path": f"ref/masks/{mask}.png"})
        cmd("layer.copyToDocument", {"document": 0, "source": 1, "name": f"mask {mask}"})
        cmd("file.close", {"document": 1, "save": "discard"})
        cmd("document.activate", {"document": 0})
        steps = [{"id": "layer.select", "params": {"name": f"mask {mask}"}},
                 {"id": "select.magicWand", "params": {"x": 2, "y": 2, "tolerance": 8, "contiguous": False, "sampleAllLayers": False, "mode": "replace"}},
                 {"id": "select.inverse", "params": {}},
                 {"id": "layer.setProps", "params": {"visible": False}},
                 {"id": "layer.select", "params": {"name": "base"}}]
        if colour == "carbon":
            steps.append({"id": "layer.newFillLayer.pattern", "params": {"pattern": "carbon twill", "scale": scale}})
        else:
            steps.append({"id": "layer.newFillLayer.solidColor", "params": {"color": colour}})
        steps.append({"id": "layer.setProps", "params": {"name": name}})
        run("command_batch", {"steps": steps})
    for name, kind, params in shapes:
        cmd("shape.create", {"kind": kind, "name": name, **params})
    for name, logo, x, y, width, rot in stickers:
        w, _ = Image.open(f"{LOGOS}/{logo}.png").size
        s = width / w * 100
        run("doc_open", {"path": f"logos/png/{logo}.png"})
        cmd("layer.copyToDocument", {"document": 0, "source": 1, "at": [x, y], "name": name})
        cmd("file.close", {"document": 1, "save": "discard"})
        cmd("document.activate", {"document": 0})
        run("command_batch", {"steps": [
            {"id": "layer.select", "params": {"name": name}},
            {"id": "edit.transform", "params": {"scaleX": s, "scaleY": s, "rotate": rot, "reference": "center"}},
            {"id": "layer.arrange.bringToFront", "params": {}}]})
    run("command_run", {"id": "layer.list", "params": {"flat": True}}, 10**8)
    out = subprocess.run(["python3", "-I", "pc.py", json.dumps(calls)], capture_output=True, text=True).stdout
    if '{"activeLayer"' not in out:
        print(part, "FAILED:", out[-800:]); return
    errors = [l for l in out.splitlines() if '"ok":false' in l or "stopping" in l or "app:" in l]
    if errors:
        print(part, "FAILED:", errors[:3])
        return
    layers = json.loads(out[out.rindex('{"activeLayer"'):].split("\n")[0])["layers"]
    hide = [l["id"] for l in layers if l["name"] == "Background"]
    fin = [{"tool": "command_run", "args": {"id": "layer.setProps", "params": {"layers": hide, "visible": False}}},
           {"tool": "doc_save", "args": {"path": f"out/ferrari_{part}.pcraft"}},
           {"tool": "doc_export", "args": {"path": f"out/ferrari_{part}.png"}}]
    out2 = subprocess.run(["python3", "-I", "pc.py", json.dumps(fin)], capture_output=True, text=True).stdout
    print(part, "errors:", errors[:3], "|", out2.strip().splitlines()[-1][:160])

def flag(prefix, x0, x1, y, h, vertical=False):
    """Italian tricolore as three hand-placed rectangles (green, white, red)."""
    third = (x1 - x0) / 3
    out = []
    for i, colour in enumerate((GREEN, WHITE, FLAGRED)):
        rect = [x0 + i * third, y, third, h] if not vertical else [y, x0 + i * third, h, third]
        out.append((f"{prefix} flag {i}", "rect", {"rect": rect, "fill": colour}))
    return out

parts = set(sys.argv[1:])

if "front_wing" in parts:
    # Upper flaps sit at x ~128-900 on the sheet, one band per flap, span along y,
    # mirrored about y ~ 1984. Outboard to inboard as on the Monoposto sheet:
    # HP on the top flap, V-Power on the middle one, Ray-Ban on the lowest.
    M = 3968
    # centred in each flap's visible strip (between its leading edge and the one above),
    # tilted to follow the flap line in this top-down projection: (name, logo, x, y, width, tilt)
    flaps = [("HP", "hp2025", 266, 640, 150, 19), ("V-Power", "vpower-white", 516, 780, 440, 19),
             ("Ray-Ban", "rayban-white", 723, 820, 340, 17)]
    stickers = []
    for name, logo, x, y, w, tilt in flaps:
        stickers.append((f"{name} a", logo, x, y, w, -90 - tilt))
        stickers.append((f"{name} b", logo, x, M - y, w, -90 + tilt))
    stickers += [("UniCredit right", "unicredit2-white", 3700, 1460, 470, -97),
                 ("UniCredit left", "unicredit2-white", 3700, 2604, 470, -83)]
    shapes = flag("endplate right", 1180, 1740, 3880, 46, vertical=True) + \
             flag("endplate left", 2324, 2884, 3880, 46, vertical=True)
    build("front_wing", {"pattern": True}, [], stickers, shapes)

if "front_covers" in parts:
    build("front_covers", {"color": "#1A1A1D"}, [], [], [])

if "rear_covers" in parts:
    build("rear_covers", {"color": "#1A1A1D"}, [], [], [])

if "rear_wing" in parts:
    # Right outer endplate: upright, front to the right, top at y ~ 2600.
    right = [("V-Power", "vpower-white", 1950, 2960, 820), ("Peroni", "peroni-full-white", 1950, 3380, 560)]
    stickers = []
    for name, logo, x, y, w in right:
        stickers.append((f"{name} right", logo, x, y, w, 0))
        stickers.append((f"{name} left", logo, 1940 - (x - 1952), 902 - (y - 3233), w, 180))
    stickers += [("HP top right", "hp2025", 790, 1100, 300, -90), ("HP top left", "hp2025", 790, 2930, 300, -90)]
    shapes = flag("right", 1500, 2400, 2700, 34) + flag("left", 1492, 2392, 902 - (2700 - 3233) - 34, 34)
    build("rear_wing", {"pattern": True}, [], stickers, shapes)

if "floor" in parts:
    build("floor", {"pattern": True}, [], [
        ("ambipar upper", "ambipar-white", 1286, 1748, 620, 180),
        ("ambipar lower", "ambipar-white", 1286, 3160, 620, 0),
    ], [])

if "mirrors" in parts:
    build("mirrors", {"color": RED}, [], [], [])

if "winglets" in parts:
    build("winglets", {"pattern": True, "scale": 120}, [], [], [])

if "cockpit_foam" in parts:
    build("cockpit_foam", {"pattern": True, "scale": 120}, [], [
        ("UniCredit inner a", "unicredit2-white", 2750, 1288, 560, 180),
        ("Bitdefender inner a", "bitdefender-white", 2050, 1288, 560, 180),
        ("UniCredit inner b", "unicredit2-white", 2050, 3102, 560, 0),
        ("Bitdefender inner b", "bitdefender-white", 2750, 3102, 560, 0),
    ], [])
