"""Per-region stretch of the mapping: singular values of d(template px)/d(surface units).
Reports anisotropy (s1/s2) and scale (sqrt(s1*s2)) percentiles per region.
Usage: python3 -I tools/livery/check_stretch.py MAP.npz BAKE_DIR"""
import sys
import numpy as np

M = np.load(sys.argv[1]); bake = sys.argv[2]
names = list(M["names"])
for part in ["main_body", "front_wing", "rear_wing"]:
    names = list(M[f"{part}_names"]) if f"{part}_names" in M else list(M["names"])
    xy = M[f"{part}_xy"].astype(np.float64); reg = M[f"{part}_region"]
    pos = np.load(f"{bake}/{part}.npz")["pos"].astype(np.float64)
    dxy_u = xy[:, 1:] - xy[:, :-1]; dxy_v = xy[1:] - xy[:-1]
    dp_u = pos[:, 1:] - pos[:, :-1]; dp_v = pos[1:] - pos[:-1]
    A = slice(0, -1)
    T = np.stack([dxy_u[A, :], dxy_v[:, A]], -1)        # (h,w,2,2) template px per texel
    S = np.stack([dp_u[A, :], dp_v[:, A]], -1)          # (h,w,3,2) surface per texel
    r = reg[A, A]
    ok = (r >= 0) & np.isfinite(T).all((-1, -2)) & (reg[A, 1:] == r) & (reg[1:, A] == r)
    ok &= np.linalg.norm(S[..., 0], axis=-1) < 0.05
    ok &= np.linalg.norm(S[..., 1], axis=-1) < 0.05
    Tn, Sn = T[ok], S[ok]
    # Orthonormal frame on surface: QR of S.
    Q, R = np.linalg.qr(Sn)                             # R: 2x2 surface metric
    good = np.abs(np.linalg.det(R)) > 1e-10
    Jm = Tn[good] @ np.linalg.inv(R[good])              # template px per surface unit
    sv = np.linalg.svd(Jm, compute_uv=False)
    aniso = sv[:, 0] / np.maximum(sv[:, 1], 1e-9)
    scale = np.sqrt(sv[:, 0] * sv[:, 1])
    rr = r[ok][good]
    print(f"== {part}")
    for k in np.unique(rr):
        m = rr == k
        if m.sum() < 200:
            continue
        a = aniso[m]; s = scale[m]
        print(f"  {names[k]:13s} n={m.sum():7d} aniso p50={np.median(a):.2f} p90={np.percentile(a, 90):.2f}"
              f"  scale p10/p50/p90={np.percentile(s, 10):.0f}/{np.median(s):.0f}/{np.percentile(s, 90):.0f}")
