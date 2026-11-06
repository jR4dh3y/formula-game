import json
from PIL import Image
im = Image.open("livery/ref/zones_preview.png").convert("RGBA")
for r in json.load(open("decal_report.json")):
    if "number 16" in r["name"]: continue
    dec = Image.open(r["path"]).convert("RGBA"); im.alpha_composite(dec, (r["x"], r["y"]))
im.convert("RGB").save("livery/ref/zones_preview_dec.png")
