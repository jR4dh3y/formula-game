# 3D-zone base paint + projected decals replace the 2D shapes
import json, subprocess
def pc(calls): return subprocess.run(["python3","-I","pc.py",json.dumps(calls)],capture_output=True,text=True).stdout
out = pc([{"tool":"command_run","args":{"id":"layer.list","params":{"flat":True}},"limit":10**8}])
L = {l["name"]: l["id"] for l in json.loads(out[out.index('{"activeLayer"'):].split("\n")[0])["layers"]}
drop = ["white upper body left","white upper body right","white cockpit surround","pinstripe left","pinstripe right",
        "carbon halo-front","carbon halo-left","carbon halo-right","H dxc left","H dxc right"]
drop += [n for n in L if n[:2] in ("R ","L ","N ","H ") and n != "N peroni"]
calls = [{"tool":"command_run","args":{"id":"layer.delete","params":{"layer":L[n]}}} for n in drop if n in L]
def place(path, x, y, name):
    return [{"tool":"doc_open","args":{"path":path}},
            {"tool":"command_run","args":{"id":"layer.copyToDocument","params":{"document":0,"source":1,"at":[x,y],"name":name}}},
            {"tool":"command_run","args":{"id":"file.close","params":{"document":1,"save":"discard"}}},
            {"tool":"command_run","args":{"id":"document.activate","params":{"document":0}}},
            {"tool":"command_run","args":{"id":"layer.arrange.bringToFront","params":{"layer":"__last__"}}}]
seq = []
for z, name in (("halo","zone carbon halo"),("white","zone white"),("white_pin","zone white pinstripe"),("red_pin","zone red pinstripe"),("interior","zone cockpit interior")):
    seq += place(f"ref/decals/zone_{z}.png", 2048, 2048, name)
for r in json.load(open("decal_report.json")):
    seq += place(r["path"].replace("livery/","",1), r["x"]+r["w"]//2, r["y"]+r["h"]//2, r["name"])
# copyToDocument lands on top already; drop the bringToFront placeholder steps
seq = [c for c in seq if c["args"].get("params",{}).get("layer") != "__last__"]
r = pc(calls + seq)
bad = [l[:300] for l in r.splitlines() if "stopping" in l or "app:" in l]
print(bad[:5])
# nose stickers back on top of the new zones
out = pc([{"tool":"command_run","args":{"id":"layer.list","params":{"flat":True}},"limit":10**8}])
L2 = json.loads(out[out.index('{"activeLayer"'):].split("\n")[0])["layers"]
fr = [{"tool":"command_run","args":{"id":"layer.arrange.bringToFront","params":{"layer":l["id"]}}} for l in reversed(L2) if l["name"][:2] in ("N ","R ","L ","H ")]
fr.append({"tool":"doc_save","args":{"path":"out/ferrari_main_body_v2.pcraft"}})
print(pc(fr)[-200:])
print([l["name"] for l in L2][:20])
