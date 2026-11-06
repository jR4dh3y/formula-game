import json, subprocess, sys
number = sys.argv[1]
out = subprocess.run(["python3","-I","pc.py",json.dumps([{"tool":"command_run","args":{"id":"layer.list","params":{"flat":True}},"limit":10**8}])],capture_output=True,text=True).stdout
L = json.loads(out[out.index("{"):])["layers"]
hide = [l["id"] for l in L if l["name"].startswith("mask ") or (l["name"].endswith("number 44") if number=="16" else l["name"].endswith("number 16"))]
show = [l["id"] for l in L if l["name"].endswith(f"number {number}")]
calls = [{"tool":"command_run","args":{"id":"layer.setProps","params":{"layers":hide,"visible":False}}},
         {"tool":"command_run","args":{"id":"layer.setProps","params":{"layers":show,"visible":True}}},
         {"tool":"doc_save","args":{"path":"out/ferrari_main_body_v2.pcraft"}},
         {"tool":"doc_export","args":{"path":f"out/ferrari_main_body_{number}.png"}}]
print(subprocess.run(["python3","-I","pc.py",json.dumps(calls)],capture_output=True,text=True).stdout[:600])
