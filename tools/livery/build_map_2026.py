"""Build the Monoposto 2026 template -> FP04RB texel correspondence.

Usage: python3 -I tools/livery/build_map_2026.py BAKE_DIR ISLANDS.npy OUT.npz
For each FP04RB paint texture, stores template pixel coordinates (float, NaN = fill)
and a fill code per texel.
"""
import sys
from pathlib import Path
import numpy as np
from scipy import ndimage

# Isolated Python omits the script directory. Import only this repository's module.
sys.path.insert(0, str(Path(__file__).resolve().parent))
import wing_map

bake, islands_path, out = sys.argv[1], sys.argv[2], sys.argv[3]
L = np.load(islands_path)
H = L.shape[0]
CX = 1024.0
AN = 1.35  # max allowed lateral/longitudinal scale ratio
FILL_NONE, FILL_BODY, FILL_CARBON = 0, 1, 2

# ---------- template regions ----------
yy, xx = np.mgrid[0:H, 0:H]
col = L == 26
REG = {
    "ec_left": col & (yy <= 636) & (xx >= CX),   # car left = image right
    "ec_right": col & (yy <= 636) & (xx < CX),
    "ck_left": col & (yy > 636) & (yy <= 1100) & (xx >= CX),
    "ck_right": col & (yy > 636) & (yy <= 1100) & (xx < CX),
    "nose": col & (yy > 1100),
    "sp_right": L == 21, "sp_left": L == 22,
    "cs_right": L == 28, "cs_left": L == 27,
    "halo": L == 4,
    "fwing": L == 42,
    "fw_ep_out_right": L == 15, "fw_ep_out_left": L == 5,
    "rwing": (L == 34) | (L == 36) | (L == 37),
    "ep_out_left": L == 35, "ep_out_right": L == 41,
    "ep_in_left": L == 20, "ep_in_right": L == 30,
    # Round wheel-cover discs (outer r 100, hub hole r 26 template px).
    "front_covers": L == 33, "rear_covers": L == 39,
    # Appended so existing region indices stay stable (see wing_map.py).
    "fw_ep_in_right": L == 12, "fw_ep_in_left": L == 11,
    "fw_flap_top": L == 34, "fw_flap_mid": L == 36, "fw_main": L == 37,
    "rw_main_top": L == 43, "rw_flap_lo": L == 44, "rw_flap_hi": L == 45,
    "rw_flap_back": L == 46, "rw_main_under": L == 47,
}
NAMES = list(REG)
NEAREST = {}
for k, m in REG.items():
    _, idx = ndimage.distance_transform_edt(~m, return_indices=True)
    NEAREST[k] = idx  # (2,H,W): row, col of nearest inside pixel


def row_extent(mask):
    """Per row min/max x (NaN where empty), filled by interpolation."""
    has = mask.any(1)
    lo = np.where(has, np.argmax(mask, 1), np.nan).astype(float)
    hi = np.where(has, H - 1 - np.argmax(mask[:, ::-1], 1), np.nan).astype(float)
    r = np.arange(H)
    for a in (lo, hi):
        ok = ~np.isnan(a)
        a[~ok] = np.interp(r[~ok], r[ok], a[ok])
    return lo, hi


def col_extent(mask):
    return row_extent(mask.T)


# Column longitudinal mapping (FP04RB X -> template row).
XK = np.array([-5.16, -0.41, 1.45, 7.0])
YK = np.array([7.0, 640.0, 1100.0, 2044.0])
def col_y(X):
    return np.interp(X, XK, YK)
KSEG = np.diff(YK) / np.diff(XK)
def col_k(X):
    return np.where(X < XK[1], KSEG[0], np.where(X < XK[2], KSEG[1], KSEG[2]))

# Sidepod longitudinal mapping.
SP_X, SP_Y = (-3.4, 1.26), (151.0, 1265.0)
def sp_y(X):
    return SP_Y[0] + (X - SP_X[0]) / (SP_X[1] - SP_X[0]) * (SP_Y[1] - SP_Y[0])

ec_lo, ec_hi = row_extent(REG["ec_left"])          # half width = ec_hi - CX
EC_HW = ec_hi - CX
spR_lo, spR_hi = row_extent(REG["sp_right"])
spL_lo, spL_hi = row_extent(REG["sp_left"])
ckL_lo, ckL_hi = row_extent(REG["ck_left"])        # rails: inner=lo, outer=hi
ckR_lo, ckR_hi = row_extent(REG["ck_right"])       # inner=hi, outer=lo
csL_lo, csL_hi = row_extent(REG["cs_left"])
csR_lo, csR_hi = row_extent(REG["cs_right"])
nose_lo, nose_hi = row_extent(REG["nose"])
NOSE_HW = (nose_hi - nose_lo) / 2


def ri(v):
    return np.clip(np.round(v).astype(int), 0, H - 1)


def finish(region_names, x, y):
    """Snap coordinates to the nearest pixel inside each texel's region."""
    out = np.stack([x, y], -1).astype(np.float32)
    snap = np.zeros(len(x), np.float32)
    for k, name in enumerate(NAMES):
        sel = region_names == k
        if not sel.any():
            continue
        r, c = ri(y[sel]), ri(x[sel])
        inside = REG[name][r, c]
        nr, nc = NEAREST[name][0][r, c], NEAREST[name][1][r, c]
        out[sel, 0] = np.where(inside, x[sel], nc)
        out[sel, 1] = np.where(inside, y[sel], nr)
        snap[sel] = np.hypot(nc - x[sel], nr - y[sel])
    return out, snap


def body(d, g):
    pos, mask, mat = d["pos"], d["mask"], d["mat"]
    s = g["s"]
    X, Y, Z = pos[..., 0], pos[..., 1], pos[..., 2]
    a = np.abs(s)
    left = s < 0
    paint = mask & (mat == 0) & ~np.isnan(s)
    region = np.full(mask.shape, -1, int)
    x = np.full(mask.shape, np.nan)
    y = np.full(mask.shape, np.nan)
    fill = np.where(mask, FILL_CARBON, FILL_NONE)
    fill[paint] = FILL_BODY

    # Halo and its struts: separate UV islands that sit high above the cockpit.
    lab, n = ndimage.label(ndimage.binary_closing(mask, iterations=2))
    halo = np.zeros_like(mask)
    for i in range(1, n + 1):
        m = (lab == i) & paint
        if m.sum() < 500:
            continue
        P = pos[m]
        if P[:, 0].min() > -0.6 and P[:, 0].max() < 2.6 and P[:, 1].min() > 1.6:
            halo |= m
    # amax(X): FP04RB unrolled half-arc per station.
    bins = np.linspace(-5.3, 7.1, 125)
    bi = np.clip(np.digitize(X[paint & ~halo], bins), 1, len(bins) - 1)
    amax_b = np.zeros(len(bins))
    np.maximum.at(amax_b, bi, a[paint & ~halo])
    amax_b = ndimage.maximum_filter1d(amax_b, 3)
    amax = amax_b[np.clip(np.digitize(X, bins), 1, len(bins) - 1)]
    amax = np.maximum(amax, 1e-3)

    yc = col_y(X)
    k = col_k(X)
    ys = sp_y(X)
    rc, rs = ri(yc), ri(ys)
    body_px = paint & ~halo

    # Sidepod width at this station (template px) for balanced splitting.
    wsp = np.where(left, spL_hi[rs] - spL_lo[rs], spR_hi[rs] - spR_lo[rs])
    in_sp_range = (X >= SP_X[0] - 0.4) & (X <= SP_X[1])

    # --- engine cover zone ---
    ez = body_px & (X <= XK[1])
    hw = EC_HW[rc]
    s_bal = np.where(in_sp_range, amax * hw / (hw + wsp), amax)
    kl_ec = np.clip(hw / np.maximum(s_bal, 1e-3), k / AN, k * AN)
    s_b = np.minimum(hw / kl_ec, amax)
    ec = ez & (a <= s_b)
    x[ec] = np.where(left, CX + 4 + a * kl_ec, CX - 4 - a * kl_ec)[ec]
    y[ec] = yc[ec]
    region[ec & left] = NAMES.index("ec_left")
    region[ec & ~left] = NAMES.index("ec_right")

    # --- cockpit zone ---
    cz = body_px & (X > XK[1]) & (X <= XK[2])
    # FP04RB rim offset ~ min |s| of the station.
    a_rim = np.full(len(bins), np.inf)
    sel = cz
    np.minimum.at(a_rim, np.clip(np.digitize(X[sel], bins), 1, len(bins) - 1), a[sel])
    a_rim[np.isinf(a_rim)] = 0
    a0 = a_rim[np.clip(np.digitize(X, bins), 1, len(bins) - 1)]
    rail_in = np.where(left, ckL_lo[rc], ckR_hi[rc])
    rail_out = np.where(left, ckL_hi[rc], ckR_lo[rc])
    rail_w = np.abs(rail_out - rail_in)
    a_rail = a0 + rail_w / k
    rails = cz & (a <= a_rail)
    sgn = np.where(left, 1.0, -1.0)
    x[rails] = (rail_in + sgn * (a - a0) * k)[rails]
    y[rails] = yc[rails]
    region[rails & left] = NAMES.index("ck_left")
    region[rails & ~left] = NAMES.index("ck_right")
    # chassis-side pieces 27/28 for rows 847..1055
    cs_rows = (yc >= 847) & (yc <= 1055)
    cs_in = np.where(left, csL_lo[rc], csR_hi[rc])
    cs_w = np.where(left, csL_hi[rc] - csL_lo[rc], csR_hi[rc] - csR_lo[rc])
    a_cs = a_rail + cs_w / k
    cs = cz & cs_rows & (a > a_rail) & (a <= a_cs)
    x[cs] = (cs_in + sgn * (a - a_rail) * k)[cs]
    y[cs] = yc[cs]
    region[cs & left] = NAMES.index("cs_left")
    region[cs & ~left] = NAMES.index("cs_right")
    s_b = np.where(cz, np.where(cs_rows, a_cs, a_rail), s_b)

    # --- sidepods (beyond the engine cover / cockpit boundary) ---
    sp = (ez | cz) & (a > s_b) & in_sp_range & (region < 0)
    xi = np.where(left, spL_lo[rs], spR_hi[rs])
    xo = np.where(left, spL_hi[rs], spR_lo[rs])
    k_sp = (SP_Y[1] - SP_Y[0]) / (SP_X[1] - SP_X[0])
    kl_sp = np.clip(np.abs(xo - xi) / np.maximum(amax - s_b, 1e-3), k_sp / 1.6, k_sp * 1.6)
    x[sp] = (xi + np.sign(xo - xi) * (a - s_b) * kl_sp)[sp]
    y[sp] = ys[sp]
    region[sp & left] = NAMES.index("sp_left")
    region[sp & ~left] = NAMES.index("sp_right")

    # --- nose / everything else on the body: column lateral unroll ---
    rest = body_px & (region < 0)
    kl = np.clip(NOSE_HW[rc] / amax, 0.8 * k, 1.25 * k)
    kl = np.where(X > XK[2], kl, k)
    x[rest] = (CX + sgn * a * kl)[rest]
    y[rest] = yc[rest]
    nz = rest & (X > XK[2])
    region[nz] = NAMES.index("nose")
    rr = rest & ~nz
    region[rr & (X <= XK[1]) & left] = NAMES.index("ec_left")
    region[rr & (X <= XK[1]) & ~left] = NAMES.index("ec_right")
    region[rr & (X > XK[1]) & left] = NAMES.index("ck_left")
    region[rr & (X > XK[1]) & ~left] = NAMES.index("ck_right")

    # --- halo: polar unroll in top view ---
    # The front support is not part of the U-shaped ribbon in island 4.
    hm = halo & (X < 1.3) & (Y > 1.9)
    fill[halo & ~hm] = FILL_CARBON
    Xc = -0.48
    phi = np.degrees(np.arctan2(Z, X - Xc))       # left (Z<0) negative
    q = np.hypot(Z, X - Xc)
    th = L == 4
    ty, tx = np.nonzero(th)
    hc = np.array([1873.0, 438.0])
    psi = np.degrees(np.arctan2(tx - hc[0], hc[1] - ty))
    r = np.hypot(tx - hc[0], hc[1] - ty)
    pb = np.linspace(-95, 95, 39)
    r0 = ndimage.minimum_filter1d(np.array([r[np.digitize(psi, pb) == i].min() if (np.digitize(psi, pb) == i).any() else np.nan for i in range(1, len(pb))]), 1)
    r1 = np.array([r[np.digitize(psi, pb) == i].max() if (np.digitize(psi, pb) == i).any() else np.nan for i in range(1, len(pb))])
    hb = np.clip(np.digitize(phi[hm], pb), 1, len(pb) - 1)
    qm = np.full(len(pb)-1, np.nan); ym = qm.copy()
    for i in range(1, len(pb)):
        sel = hb == i
        if sel.any():
            v, height = q[hm][sel], Y[hm][sel]
            qm[i-1] = (v.min()+v.max())/2
            ym[i-1] = (height.min()+height.max())/2
    def fillnan(v):
        ok = ~np.isnan(v)
        return np.interp(np.arange(len(v)), np.nonzero(ok)[0], v[ok])
    r0, r1 = map(fillnan, (r0,r1))
    centres = (pb[:-1]+pb[1:])/2
    qm, ym = [ndimage.gaussian_filter1d(fillnan(v),1) for v in (qm,ym)]
    qcentre, ycentre = [np.interp(phi[hm], centres, v) for v in (qm,ym)]
    # Unwrap the tube circumference, rather than projecting its inner and outer
    # walls onto each other. The cut is on the underside.
    theta = np.arctan2(q[hm]-qcentre, Y[hm]-ycentre)
    fraction = .5 + theta/(2*np.pi)
    inner, outer = [np.interp(phi[hm],centres,v) for v in (r0,r1)]
    rr_ = inner + fraction*(outer-inner)
    ang = np.radians(phi[hm])
    x[hm] = hc[0] + rr_ * np.sin(ang)
    y[hm] = hc[1] - rr_ * np.cos(ang)
    region[hm] = NAMES.index("halo")
    global HALO_MASK
    HALO_MASK = halo
    return region, x, y, fill


def fill_only(d, kind):
    mask = d["mask"]
    shape = mask.shape
    return (np.full(shape, -1, int), np.full(shape, np.nan), np.full(shape, np.nan),
            np.where(mask, kind, FILL_NONE))


result = {}
d = np.load(f"{bake}/main_body.npz"); g = np.load(f"{bake}/main_body_s.npz")
result["main_body"] = body(d, g)
# Wings are mapped per surface chart and packed by wing_map.
packed = {}
for part, build in (("front_wing", wing_map.front_wing), ("rear_wing", wing_map.rear_wing)):
    d = np.load(f"{bake}/{part}.npz")
    packed[part] = wing_map.pack(build(d, L), d["mask"], NAMES, L)
result["mirrors"] = fill_only(np.load(f"{bake}/mirrors.npz"), FILL_BODY)
result["floor"] = fill_only(np.load(f"{bake}/floor.npz"), FILL_CARBON)
result["cockpit_foam"] = fill_only(np.load(f"{bake}/cockpit_foam.npz"), FILL_CARBON)

def cover(name, size=1024, uv_radius=0.455):
    """Wheel covers have no OBJ; their glTF UVs are a planar disc centred at
    (0.5, 0.5) with radius 0.455 (measured on car.glb). Scale it linearly onto
    the template disc, rim to rim. Upright pieces outside the disc share the
    material and stay carbon."""
    ys, xs = np.nonzero(REG[name])
    cx, cy = (xs.min() + xs.max()) / 2, (ys.min() + ys.max()) / 2
    radius = (xs.max() - xs.min()) / 2
    k = radius / uv_radius
    hole = np.hypot(xs - cx, ys - cy).min() / k  # template hub opening, in UV
    v, u = (np.mgrid[0:size, 0:size] + 0.5) / size
    r = np.hypot(u - 0.5, v - 0.5)
    # Inside the hub opening the template has no artwork; snapping would smear
    # the innermost ring into a solid disc, so the hub stays carbon.
    disc = (r <= uv_radius) & (r >= hole)
    region = np.where(disc, NAMES.index(name), -1)
    return (region, np.where(disc, cx + (u - 0.5) * k, np.nan),
            np.where(disc, cy + (v - 0.5) * k, np.nan), np.where(disc, FILL_NONE, FILL_CARBON))


result["front_covers"] = cover("front_covers")
result["rear_covers"] = cover("rear_covers")

# No demonstrated template correspondence for the winglets. An explicit neutral
# override prevents embedded Audi logos/stripes leaking into converted teams.
shape = (256, 256)
result["winglets"] = (np.full(shape, -1, int), np.full(shape, np.nan),
                      np.full(shape, np.nan), np.full(shape, FILL_CARBON))

save = {"names": np.array(NAMES), "main_body_halo_mask": HALO_MASK}
for part, arrays in packed.items():
    for key, value in arrays.items():
        save[f"{part}_{key}"] = value
    print(part, "mapped", int((arrays["region"] >= 0).sum()), "fill", int((arrays["fill"] > 0).sum()), flush=True)
for part, (region, x, y, fill) in result.items():
    coords = np.full(region.shape + (2,), np.nan, np.float32)
    has = region >= 0
    coords[has], sd = finish(region[has], x[has], y[has])
    snapimg = np.zeros(region.shape, np.float16); snapimg[has] = sd
    save[f"{part}_snap"] = snapimg
    fill = fill.copy(); fill[has] = FILL_NONE
    save[f"{part}_xy"] = coords
    save[f"{part}_fill"] = fill.astype(np.int8)
    save[f"{part}_region"] = region.astype(np.int8)
    print(part, "mapped", int(has.sum()), "fill", int((fill > 0).sum()), flush=True)
np.savez_compressed(out, **save)
