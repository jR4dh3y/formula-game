# re-place every projected decal (side, nose, halo) over the 3D zones
import json, subprocess
def pc(calls): return subprocess.run(["python3","-I","pc.py",json.dumps(calls)],capture_output=True,text=True).stdout
pre = [{"tool":"command_run","args":{"id":"file.closeAll","params":{}}},{"tool":"doc_open","args":{"path":"out/ferrari_main_body_v2.pcraft"}},
       {"tool":"command_run","args":{"id":"layer.list","params":{"flat":True}},"limit":10**8}]
out = pc(pre)
L = {l["name"]: l["id"] for l in json.loads(out[out.index('{"activeLayer"'):].split("\n")[0])["layers"]}
calls = [{"tool":"command_run","args":{"id":"layer.delete","params":{"layer":i}}} for n, i in L.items() if n[:2] in ("R ","L ","N ","H ")]
for r in json.load(open("decal_report.json")):
    calls += [{"tool":"doc_open","args":{"path":r["path"].replace("livery/","",1)}},
              {"tool":"command_run","args":{"id":"layer.copyToDocument","params":{"document":0,"source":1,"at":[r["x"]+r["w"]//2, r["y"]+r["h"]//2],"name":r["name"]}}},
              {"tool":"command_run","args":{"id":"file.close","params":{"document":1,"save":"discard"}}},
              {"tool":"command_run","args":{"id":"document.activate","params":{"document":0}}}]
calls.append({"tool":"doc_save","args":{"path":"out/ferrari_main_body_v2.pcraft"}})
r = pc(calls)
print([l[:300] for l in r.splitlines() if "stopping" in l or "app:" in l][:5], r[-120:])
