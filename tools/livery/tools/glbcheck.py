import json, struct, numpy as np, sys
path = sys.argv[1]
b = open(path, "rb").read()
jl = struct.unpack_from("<I", b, 12)[0]; J = json.loads(b[20:20 + jl]); bin0 = 20 + jl + 8
CT = {5126: np.float32, 5123: np.uint16, 5125: np.uint32, 5121: np.uint8}
NC = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}
def acc(i):
    a = J["accessors"][i]; v = J["bufferViews"][a["bufferView"]]
    off = bin0 + v.get("byteOffset", 0) + a.get("byteOffset", 0)
    dt = np.dtype(CT[a["componentType"]]); n = NC[a["type"]]
    stride = v.get("byteStride", 0) or dt.itemsize * n
    arr = np.ndarray((a["count"], n), dt, b, off, (stride, dt.itemsize))
    return arr, off, stride, dt
mats = [m["name"] for m in J["materials"]]
for mi, mesh in enumerate(J["meshes"]):
    for pi, pr in enumerate(mesh["primitives"]):
        name = mats[pr["material"]]
        if "main_body" not in name: continue
        P, *_ = acc(pr["attributes"]["POSITION"]); N, *_ = acc(pr["attributes"]["NORMAL"])
        I, ioff, _, idt = acc(pr["indices"]); I = I.reshape(-1, 3)
        a, b_, c = P[I[:, 0]], P[I[:, 1]], P[I[:, 2]]
        g = np.cross(b_ - a, c - a); g /= np.linalg.norm(g, axis=1)[:, None] + 1e-12
        vn = N[I].mean(axis=1); vn /= np.linalg.norm(vn, axis=1)[:, None] + 1e-12
        dot = (g * vn).sum(1)
        bad = dot < -0.3
        print(mesh["name"], pi, name, "tris", len(I), "flipped", int(bad.sum()), "idx dtype", idt)
        if bad.any():
            cen = (a + b_ + c)[bad] / 3
            print("  flipped centre range (glTF coords)", cen.min(0).round(2), cen.max(0).round(2))
